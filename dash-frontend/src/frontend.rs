use std::{path::PathBuf, rc::Rc};

use chrono::Timelike;
use glam::Vec2;
use wgui::{
	assets::{AssetPath, AssetProvider},
	components::button::ComponentButton,
	font_config::WguiFontConfig,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, LayoutParams, LayoutUpdateParams, LayoutUpdateResult, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	renderer_vk::text::custom_glyph::CustomGlyphData,
	task::Tasks,
	theme::WguiTheme,
	widget::{label::WidgetLabel, rectangle::WidgetRectangle, sprite::WidgetSprite},
	windowing::window::{WguiWindow, WguiWindowParams, WguiWindowParamsExtra, WguiWindowPlacement},
};
use wlx_common::{
	async_executor::AsyncExecutor,
	audio,
	dash_interface::{BoxDashInterface, RecenterMode},
	locale::WayVRLangProvider,
	timestep::{self, Timestep},
};

use crate::{
	assets,
	tab::{Tab, TabType, apps::TabApps, games::TabGames, home::TabHome, monado::TabMonado, settings::TabSettings},
	util::{
		popup_manager::{MountPopupOnceParams, MountPopupParams, PopupManager, PopupManagerParams},
		toast_manager::ToastManager,
	},
	views,
};

pub struct FrontendWidgets {
	id_label_time: WidgetID,
	id_rect_content: WidgetID,
	id_sprite_titlebar_icon: WidgetID,
	id_label_titlebar_title: WidgetID,
}

pub type FrontendTasks = Tasks<FrontendTask>;

pub struct Frontend<T> {
	pub layout: Layout,
	pub globals: WguiGlobals,

	pub interface: BoxDashInterface<T>,

	// async runtime executor
	pub executor: AsyncExecutor,

	#[allow(dead_code)]
	state: ParserState,

	current_tab: Option<Box<dyn Tab<T>>>,

	pub tasks: FrontendTasks,

	ticks: u32,

	widgets: FrontendWidgets,
	popup_manager: PopupManager,
	toast_manager: ToastManager,
	timestep: Timestep,
	sounds_to_play: Vec<SoundType>,

	window_audio_settings: WguiWindow,
	view_audio_settings: Option<views::audio_settings::View>,
}

pub struct FrontendUpdateParams<'a, T> {
	pub data: &'a mut T,
	pub width: f32,
	pub height: f32,
	pub timestep_alpha: f32,
}

pub struct FrontendUpdateResult {
	pub layout_result: LayoutUpdateResult,
	pub sounds_to_play: Vec<SoundType>,
}

pub struct InitParams<'a, T> {
	pub interface: BoxDashInterface<T>,
	pub lang_provider: &'a WayVRLangProvider,
	pub has_monado: bool,
	pub theme: Rc<WguiTheme>,
}

#[derive(Clone)]
pub enum SoundType {
	Startup,
	Launch,
}

#[derive(Clone)]
pub enum FrontendTask {
	SetTab(TabType),
	RefreshClock,
	RefreshBackground,
	MountPopup(MountPopupParams),
	MountPopupOnce(MountPopupOnceParams),
	RefreshPopupManager,
	ShowAudioSettings,
	UpdateAudioSettingsView,
	RecenterPlayspace,
	PushToast(Translation),
	PlaySound(SoundType),
	HideDashboard,
}

impl<T: 'static> Frontend<T> {
	pub fn new(params: InitParams<T>) -> anyhow::Result<Frontend<T>> {
		let mut assets = Box::new(assets::Asset {});

		let font_binary_bold = assets.load_from_path_gzip("Quicksand-Bold.ttf.gz")?;
		let font_binary_regular = assets.load_from_path_gzip("Quicksand-Regular.ttf.gz")?;
		let font_binary_light = assets.load_from_path_gzip("Quicksand-Light.ttf.gz")?;

		let globals = WguiGlobals::new(
			assets,
			params.lang_provider,
			&WguiFontConfig {
				binaries: vec![&font_binary_regular, &font_binary_bold, &font_binary_light],
				family_name_sans_serif: "Quicksand",
				family_name_serif: "Quicksand",
				family_name_monospace: "",
			},
			PathBuf::new(), //FIXME: pass from somewhere else
		)?;

		let (layout, state) = wgui::parser::new_layout_from_assets(
			&ParseDocumentParams {
				globals: globals.clone(),
				path: AssetPath::BuiltIn("gui/dashboard.xml"),
				extra: Default::default(),
			},
			LayoutParams {
				resize_to_parent: true,
				theme: params.theme,
			},
		)?;

		let id_popup_manager = state.get_widget_id("popup_manager")?;
		let popup_manager = PopupManager::new(PopupManagerParams {
			parent_id: id_popup_manager,
		});

		let toast_manager = ToastManager::new();

		let tasks = FrontendTasks::new();
		tasks.push(FrontendTask::SetTab(TabType::Home));

		let id_label_time = state.get_widget_id("label_time")?;
		let id_rect_content = state.get_widget_id("rect_content")?;
		let id_sprite_titlebar_icon = state.get_widget_id("sprite_titlebar_icon")?;
		let id_label_titlebar_title = state.get_widget_id("label_titlebar_title")?;

		let timestep = Timestep::new(60.0);

		let mut frontend = Self {
			layout,
			state,
			current_tab: None,
			globals,
			tasks,
			ticks: 0,
			widgets: FrontendWidgets {
				id_label_time,
				id_rect_content,
				id_sprite_titlebar_icon,
				id_label_titlebar_title,
			},
			timestep,
			interface: params.interface,
			popup_manager,
			toast_manager,
			window_audio_settings: WguiWindow::default(),
			view_audio_settings: None,
			executor: Rc::new(smol::LocalExecutor::new()),
			sounds_to_play: Vec::new(),
		};

		// init some things first
		frontend.tasks.push(FrontendTask::RefreshBackground);
		frontend.tasks.push(FrontendTask::RefreshClock);

		Frontend::register_widgets(&mut frontend)?;

		Ok(frontend)
	}

	fn queue_play_sound(&mut self, sound_type: SoundType) {
		self.sounds_to_play.push(sound_type);
	}

	fn play_sound(&mut self, audio_system: &mut audio::AudioSystem, sound_type: SoundType) -> anyhow::Result<()> {
		let mut assets = self.globals.assets_builtin();

		let path = match sound_type {
			SoundType::Startup => "sound/startup.mp3",
			SoundType::Launch => "sound/app_start.mp3",
		};

		// try loading a custom sound; if one doesn't exist (or it failed to load), use the built-in asset
		let sound_bytes = match audio::AudioSample::try_bytes_from_config(path) {
			Ok(bytes) => bytes,
			Err(_) => assets.load_from_path(path)?.into(),
		};

		let sample = audio::AudioSample::from_mp3(&sound_bytes)?;
		audio_system.play_sample(&sample);
		Ok(())
	}

	pub fn update(&mut self, mut params: FrontendUpdateParams<T>) -> anyhow::Result<FrontendUpdateResult> {
		let mut tasks = self.tasks.drain();

		while let Some(task) = tasks.pop_front() {
			self.process_task(&mut params, task)?;
		}

		let time_ms = timestep::get_micros() / 1000;

		if let Some(mut tab) = self.current_tab.take() {
			tab.update(self, time_ms as u32, params.data)?;

			self.current_tab = Some(tab);
		}

		// process async runtime tasks
		while self.executor.try_tick() {}

		let res = self.tick(params)?;
		self.ticks += 1;

		Ok(res)
	}

	pub fn process_update(
		&mut self,
		res: FrontendUpdateResult,
		audio_system: &mut audio::AudioSystem,
		audio_sample_player: &mut audio::SamplePlayer,
	) -> anyhow::Result<()> {
		for sound_type in res.sounds_to_play {
			self.play_sound(audio_system, sound_type)?;
		}

		audio_sample_player.play_wgui_samples(audio_system, res.layout_result.sounds_to_play);

		Ok(())
	}

	fn tick(&mut self, params: FrontendUpdateParams<T>) -> anyhow::Result<FrontendUpdateResult> {
		// fixme: timer events instead of this thing
		if self.ticks.is_multiple_of(1000) {
			self.update_time(params.data)?;
		}

		{
			// always 30 times per second
			while self.timestep.on_tick() {
				self.toast_manager.tick(&mut self.layout)?;
			}
		}

		let layout_result = self.layout.update(&mut LayoutUpdateParams {
			size: Vec2::new(params.width, params.height),
			timestep_alpha: params.timestep_alpha,
		})?;

		Ok(FrontendUpdateResult {
			layout_result,
			sounds_to_play: std::mem::take(&mut self.sounds_to_play),
		})
	}

	fn update_time(&mut self, data: &mut T) -> anyhow::Result<()> {
		let mut c = self.layout.start_common();
		let mut common = c.common();

		{
			let mut label = common
				.state
				.widgets
				.cast_as::<WidgetLabel>(self.widgets.id_label_time)?;

			let now = chrono::Local::now();
			let hours = now.hour();
			let minutes = now.minute();

			let text: String = if self.interface.general_config(data).clock_12h {
				let hours_ampm = (hours + 11) % 12 + 1;
				let suffix = if hours >= 12 { "PM" } else { "AM" };
				format!("{hours_ampm:02}:{minutes:02} {suffix}")
			} else {
				format!("{hours:02}:{minutes:02}")
			};

			label.set_text(&mut common, Translation::from_raw_text(&text));
		}

		c.finish()?;
		Ok(())
	}

	fn mount_popup(&mut self, params: MountPopupParams, data: &mut T) -> anyhow::Result<()> {
		let config = self.interface.general_config(data);

		self
			.popup_manager
			.mount_popup(&self.globals, &mut self.layout, &self.tasks, params, config)?;
		Ok(())
	}

	fn mount_popup_once(&mut self, params: MountPopupOnceParams, data: &mut T) -> anyhow::Result<()> {
		let config = self.interface.general_config(data);

		self
			.popup_manager
			.mount_popup_once(&self.globals, &mut self.layout, &self.tasks, params, config)?;
		Ok(())
	}

	fn refresh_popup_manager(&mut self) -> anyhow::Result<()> {
		let mut c = self.layout.start_common();
		self.popup_manager.refresh(c.common().alterables);
		c.finish()?;
		Ok(())
	}

	fn update_background(&mut self, data: &mut T) -> anyhow::Result<()> {
		self.layout.mark_redraw();

		let Some(mut rect) = self
			.layout
			.state
			.widgets
			.get_as::<WidgetRectangle>(self.widgets.id_rect_content)
		else {
			anyhow::bail!("");
		};

		let (alpha1, alpha2) = if self.interface.general_config(data).opaque_background {
			(1.0, 1.0)
		} else {
			(0.8666, 0.9333)
		};

		rect.params.color.a = alpha1;
		rect.params.color2.a = alpha2;

		Ok(())
	}

	fn process_task(&mut self, params: &mut FrontendUpdateParams<T>, task: FrontendTask) -> anyhow::Result<()> {
		match task {
			FrontendTask::SetTab(tab_type) => self.set_tab(params.data, tab_type)?,
			FrontendTask::RefreshClock => self.update_time(params.data)?,
			FrontendTask::RefreshBackground => self.update_background(params.data)?,
			FrontendTask::MountPopup(popup_params) => self.mount_popup(popup_params, params.data)?,
			FrontendTask::MountPopupOnce(popup_params) => self.mount_popup_once(popup_params, params.data)?,
			FrontendTask::RefreshPopupManager => self.refresh_popup_manager()?,
			FrontendTask::ShowAudioSettings => self.action_show_audio_settings()?,
			FrontendTask::UpdateAudioSettingsView => self.action_update_audio_settings()?,
			FrontendTask::RecenterPlayspace => self.action_recenter_playspace(params.data)?,
			FrontendTask::PushToast(content) => self.toast_manager.push(content),
			FrontendTask::PlaySound(sound_type) => self.queue_play_sound(sound_type),
			FrontendTask::HideDashboard => self.action_hide_dashboard(params.data),
		};
		Ok(())
	}

	fn set_tab_title(&mut self, translation: &str, icon: &str) -> anyhow::Result<()> {
		let mut c = self.layout.start_common();
		let mut common = c.common();

		{
			let mut label = common
				.state
				.widgets
				.cast_as::<WidgetLabel>(self.widgets.id_label_titlebar_title)?;
			label.set_text(&mut common, Translation::from_translation_key(translation));
		}

		{
			let mut sprite = common
				.state
				.widgets
				.cast_as::<WidgetSprite>(self.widgets.id_sprite_titlebar_icon)?;
			sprite.set_content(
				&mut common,
				Some(CustomGlyphData::from_assets(&self.globals, AssetPath::BuiltIn(icon))?),
			);
		}

		c.finish()?;
		Ok(())
	}

	fn set_tab(&mut self, data: &mut T, tab_type: TabType) -> anyhow::Result<()> {
		log::info!("Setting tab to {tab_type:?}");
		let widget_content = self.state.fetch_widget(&self.layout.state, "content")?;
		self.layout.remove_children(widget_content.id);

		let (tab_translation, icon_path) = match tab_type {
			TabType::Home => ("HOME_SCREEN", "dashboard/home.svg"),
			TabType::Apps => ("APPLICATIONS", "dashboard/apps.svg"),
			TabType::Games => ("GAMES", "dashboard/games.svg"),
			TabType::Monado => ("MONADO_RUNTIME", "dashboard/monado.svg"),
			TabType::Settings => ("SETTINGS", "dashboard/settings.svg"),
		};

		self.set_tab_title(tab_translation, icon_path)?;

		let tab: Box<dyn Tab<T>> = match tab_type {
			TabType::Home => Box::new(TabHome::new(self, widget_content.id, data)?),
			TabType::Apps => Box::new(TabApps::new(self, widget_content.id, data)?),
			TabType::Games => Box::new(TabGames::new(self, widget_content.id)?),
			TabType::Monado => Box::new(TabMonado::new(self, widget_content.id)?),
			TabType::Settings => Box::new(TabSettings::new(self, widget_content.id, data)?),
		};

		self.current_tab = Some(tab);

		Ok(())
	}

	fn register_widgets(&mut self) -> anyhow::Result<()> {
		// "X" button
		self.tasks.handle_button(
			&self.state.fetch_component_as::<ComponentButton>("btn_close")?,
			FrontendTask::HideDashboard,
		);

		// ################################
		// SIDE BUTTONS
		// ################################

		// "Home" side button
		self.tasks.handle_button(
			&self.state.fetch_component_as::<ComponentButton>("btn_side_home")?,
			FrontendTask::SetTab(TabType::Home),
		);

		// "Apps" side button
		self.tasks.handle_button(
			&self.state.fetch_component_as::<ComponentButton>("btn_side_apps")?,
			FrontendTask::SetTab(TabType::Apps),
		);

		// "Games" side button
		self.tasks.handle_button(
			&self.state.fetch_component_as::<ComponentButton>("btn_side_games")?,
			FrontendTask::SetTab(TabType::Games),
		);

		// "Monado side button"
		self.tasks.handle_button(
			&self.state.fetch_component_as::<ComponentButton>("btn_side_monado")?,
			FrontendTask::SetTab(TabType::Monado),
		);

		// "Settings" side button
		self.tasks.handle_button(
			&self.state.fetch_component_as::<ComponentButton>("btn_side_settings")?,
			FrontendTask::SetTab(TabType::Settings),
		);

		// ################################
		// BOTTOM BAR BUTTONS
		// ################################

		// "Audio" bottom bar button
		self.tasks.handle_button(
			&self.state.fetch_component_as::<ComponentButton>("btn_audio")?,
			FrontendTask::ShowAudioSettings,
		);

		// "Recenter playspace" bottom bar button
		self.tasks.handle_button(
			&self.state.fetch_component_as::<ComponentButton>("btn_recenter")?,
			FrontendTask::RecenterPlayspace,
		);

		Ok(())
	}

	fn action_show_audio_settings(&mut self) -> anyhow::Result<()> {
		self.window_audio_settings.open(&mut WguiWindowParams {
			position: Vec2::new(64.0, 64.0),
			layout: &mut self.layout,
			extra: WguiWindowParamsExtra {
				fixed_width: Some(400.0),
				placement: WguiWindowPlacement::BottomLeft,
				close_if_clicked_outside: true,
				title: Some(Translation::from_translation_key("AUDIO.SETTINGS")),
				..Default::default()
			},
		})?;

		let content = self.window_audio_settings.get_content();

		self.view_audio_settings = Some(views::audio_settings::View::new(views::audio_settings::Params {
			globals: self.globals.clone(),
			frontend_tasks: self.tasks.clone(),
			layout: &mut self.layout,
			parent_id: content.id,
			on_update: {
				let tasks = self.tasks.clone();
				Rc::new(move || {
					tasks.push(FrontendTask::UpdateAudioSettingsView);
				})
			},
		})?);
		Ok(())
	}

	fn action_update_audio_settings(&mut self) -> anyhow::Result<()> {
		let Some(view) = &mut self.view_audio_settings else {
			return Ok(());
		};

		view.update(&mut self.layout)?;

		Ok(())
	}

	fn action_recenter_playspace(&mut self, data: &mut T) -> anyhow::Result<()> {
		self.interface.recenter_playspace(data, RecenterMode::Recenter)?;
		Ok(())
	}

	fn action_hide_dashboard(&mut self, data: &mut T) {
		self.interface.toggle_dashboard(data);
	}
}
