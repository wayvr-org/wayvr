use std::{collections::HashMap, rc::Rc, str::FromStr};
use strum::VariantNames;
use wayvr_ipc::packet_client::{PositionMode, WvrProcessLaunchParams};
use wgui::{
	assets::AssetPath,
	components::{
		ComponentTrait, button::ComponentButton, checkbox::ComponentCheckbox, radio_group::ComponentRadioGroup,
	},
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState, TemplateParams},
	task::Tasks,
	widget::label::WidgetLabel,
};
use wlx_common::{
	config::{AppCompositorMode, AppOrientationMode, AppPosMode, AppResMode, GeneralConfig, PinnedApp},
	dash_interface::BoxDashInterface,
	desktop_finder::DesktopEntry,
};

use crate::{
	frontend::{FrontendTask, FrontendTasks, SoundType},
	util::popup_manager::{MountPopupOnceParams, PopupHolder},
	views::{ViewTrait, ViewUpdateParams},
};

#[derive(Clone)]
enum Task {
	SetCompositor(AppCompositorMode),
	SetRes(AppResMode),
	SetOrientation(AppOrientationMode),
	SetAutoStart(bool),
	Launch,
	PinApp,
	UnpinApp,
}

struct LaunchParams<'a, T> {
	application: &'a DesktopEntry,
	compositor_mode: AppCompositorMode,
	pos_mode: AppPosMode,
	res_mode: AppResMode,
	orientation_mode: AppOrientationMode,
	globals: &'a WguiGlobals,
	frontend_tasks: &'a FrontendTasks,
	interface: &'a mut BoxDashInterface<T>,
	auto_start: bool,
	data: &'a mut T,
	on_launched: Option<Box<dyn FnOnce()>>,
}

pub struct View {
	#[allow(dead_code)]
	state: ParserState,
	entry: DesktopEntry,
	tasks: Tasks<Task>,
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,

	#[allow(dead_code)]
	radio_compositor: Rc<ComponentRadioGroup>,
	#[allow(dead_code)]
	radio_res: Rc<ComponentRadioGroup>,
	#[allow(dead_code)]
	radio_orientation: Rc<ComponentRadioGroup>,

	compositor_mode: AppCompositorMode,
	pos_mode: AppPosMode,
	res_mode: AppResMode,
	orientation_mode: AppOrientationMode,

	pinned_app: Option<PinnedApp>,

	auto_start: bool,

	on_close_request: Option<Box<dyn FnOnce()>>,
	on_app_pins_changed: Option<Box<dyn Fn()>>,
}

pub struct Params<'a> {
	pub globals: &'a WguiGlobals,
	pub entry: DesktopEntry,
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub config: &'a GeneralConfig,
	pub frontend_tasks: &'a FrontendTasks,
	pub on_close_request: Box<dyn FnOnce()>,
	pub on_app_pins_changed: Box<dyn Fn()>,
	pub pinned_app: Option<PinnedApp>,
}

impl ViewTrait for View {
	fn update(&mut self, _par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		Ok(())
	}
}

impl View {
	pub fn new(params: Params) -> anyhow::Result<Self> {
		let doc_params = &ParseDocumentParams {
			globals: params.globals.clone(),
			path: AssetPath::BuiltIn("gui/view/app_launcher.xml"),
			extra: Default::default(),
		};

		let mut state = wgui::parser::parse_from_assets(doc_params, params.layout, params.parent_id)?;

		let radio_compositor = state.fetch_component_as::<ComponentRadioGroup>("radio_compositor")?;
		let radio_res = state.fetch_component_as::<ComponentRadioGroup>("radio_res")?;
		// let radio_pos = state.fetch_component_as::<ComponentRadioGroup>("radio_pos")?;
		let radio_orientation = state.fetch_component_as::<ComponentRadioGroup>("radio_orientation")?;
		let cb_autostart = state.fetch_component_as::<ComponentCheckbox>("cb_autostart")?;

		let btn_launch = state.fetch_component_as::<ComponentButton>("btn_launch")?;
		let btn_pin = state.fetch_component_as::<ComponentButton>("btn_pin")?;
		let btn_unpin = state.fetch_component_as::<ComponentButton>("btn_unpin")?;

		{
			let mut label_exec = state.fetch_widget_as::<WidgetLabel>(&params.layout.state, "label_exec")?;

			label_exec.set_text_simple(
				&mut params.globals.get(),
				Translation::from_raw_text_string(format!("{} {}", params.entry.exec_path, params.entry.exec_args)),
			);
		}

		let tasks = Tasks::new();

		tasks.handle_button(&btn_launch, Task::Launch);

		if params.pinned_app.is_some() {
			// "Unpin app"
			tasks.handle_button(&btn_unpin, Task::UnpinApp);
			params.layout.remove_widget(btn_pin.base().get_id());
		} else {
			// "Pin app"
			tasks.handle_button(&btn_pin, Task::PinApp);
			params.layout.remove_widget(btn_unpin.base().get_id());
		}

		let id_icon_parent = state.get_widget_id("icon_parent")?;

		// app icon
		if let Some(icon_path) = &params.entry.icon_path {
			let mut template_params = TemplateParams::new();
			template_params.insert("path", icon_path);
			state.instantiate_template(
				doc_params,
				"ApplicationIcon",
				params.layout,
				id_icon_parent,
				template_params,
			)?;
		}

		let compositor_mode = match &params.pinned_app {
			Some(pinned_app) => pinned_app.compositor_mode,
			None => {
				if params.config.xwayland_by_default {
					AppCompositorMode::Cage
				} else {
					AppCompositorMode::Native
				}
			}
		};

		// TODO: configurable defaults ?
		let mut res_mode = AppResMode::Res1080;
		let mut orientation_mode = AppOrientationMode::Wide;
		let pos_mode = AppPosMode::Anchored;

		if let Some(pinned_app) = &params.pinned_app {
			res_mode = pinned_app.res_mode;
			orientation_mode = pinned_app.orientation_mode;
		}

		// update radios
		{
			let mut common = params.layout.common();
			// TODO: pos_mode is disabled as for now
			radio_compositor.set_value(&mut common, compositor_mode.as_ref())?;
			radio_res.set_value(&mut common, res_mode.as_ref())?;
			radio_orientation.set_value(&mut common, orientation_mode.as_ref())?;
		}

		let auto_start = false;

		radio_compositor.on_value_changed({
			let tasks = tasks.clone();
			Box::new(move |_, ev| {
				if let Some(mode) = ev.value.and_then(|v| {
					AppCompositorMode::from_str(&v)
						.inspect_err(|_| {
							log::error!(
								"Invalid value for compositor: '{v}'. Valid values are: {:?}",
								AppResMode::VARIANTS
							)
						})
						.ok()
				}) {
					tasks.push(Task::SetCompositor(mode));
				}
				Ok(())
			})
		});

		radio_res.on_value_changed({
			let tasks = tasks.clone();
			Box::new(move |_, ev| {
				if let Some(mode) = ev.value.and_then(|v| {
					AppResMode::from_str(&v)
						.inspect_err(|_| {
							log::error!(
								"Invalid value for resolution: '{v}'. Valid values are: {:?}",
								AppResMode::VARIANTS
							)
						})
						.ok()
				}) {
					tasks.push(Task::SetRes(mode));
				}
				Ok(())
			})
		});

		// radio_pos.on_value_changed({
		// 	let tasks = tasks.clone();
		// 	Box::new(move |_, ev| {
		// 		if let Some(mode) = ev.value.and_then(|v| {
		// 			PosMode::from_str(&*v)
		// 				.inspect_err(|_| {
		// 					log::error!(
		// 						"Invalid value for position: '{v}'. Valid values are: {:?}",
		// 						PosMode::VARIANTS
		// 					)
		// 				})
		// 				.ok()
		// 		}) {
		// 			tasks.push(Task::SetPos(mode));
		// 		}
		// 		Ok(())
		// 	})
		// });

		radio_orientation.on_value_changed({
			let tasks = tasks.clone();
			Box::new(move |_, ev| {
				if let Some(mode) = ev.value.and_then(|v| {
					AppOrientationMode::from_str(&v)
						.inspect_err(|_| {
							log::error!(
								"Invalid value for orientation: '{v}'. Valid values are: {:?}",
								AppOrientationMode::VARIANTS
							)
						})
						.ok()
				}) {
					tasks.push(Task::SetOrientation(mode));
				}
				Ok(())
			})
		});

		cb_autostart.on_toggle({
			let tasks = tasks.clone();
			Box::new(move |_, ev| {
				tasks.push(Task::SetAutoStart(ev.checked));
				Ok(())
			})
		});

		let mut label_title = state.fetch_widget_as::<WidgetLabel>(&params.layout.state, "label_title")?;

		label_title.set_text_simple(
			&mut params.globals.get(),
			Translation::from_raw_text(&params.entry.app_name),
		);

		Ok(Self {
			state,
			tasks,
			radio_compositor,
			radio_res,
			radio_orientation,
			compositor_mode,
			pos_mode,
			res_mode,
			orientation_mode,
			auto_start,
			entry: params.entry,
			frontend_tasks: params.frontend_tasks.clone(),
			globals: params.globals.clone(),
			on_close_request: Some(params.on_close_request),
			on_app_pins_changed: Some(params.on_app_pins_changed),
			pinned_app: params.pinned_app,
		})
	}

	pub fn update<T>(&mut self, interface: &mut BoxDashInterface<T>, data: &mut T) -> anyhow::Result<()> {
		loop {
			let tasks = self.tasks.drain();
			if tasks.is_empty() {
				break;
			}
			for task in tasks {
				match task {
					Task::SetCompositor(mode) => self.compositor_mode = mode,
					Task::SetRes(mode) => self.res_mode = mode,
					Task::SetOrientation(mode) => self.orientation_mode = mode,
					Task::SetAutoStart(auto_start) => self.auto_start = auto_start,
					Task::Launch => self.action_launch(interface, data),
					Task::PinApp => {
						self.action_pin_app(interface.general_config(data));
						interface.config_changed(data, Default::default());
					}
					Task::UnpinApp => {
						self.action_unpin_app(interface.general_config(data));
						interface.config_changed(data, Default::default());
					}
				}
			}
		}

		Ok(())
	}

	fn close(&mut self) {
		if let Some(c) = self.on_close_request.take() {
			c();
		}
	}

	fn action_unpin_app(&mut self, config: &mut GeneralConfig) {
		let Some(pinned_app) = &self.pinned_app else {
			unreachable!();
		};
		self.frontend_tasks.push(FrontendTask::PlaySound(SoundType::Save));
		config.pinned_apps.retain(|p| p != pinned_app);

		if let Some(c) = &self.on_app_pins_changed {
			c();
		}

		self.close();
	}

	fn action_pin_app(&mut self, config: &mut GeneralConfig) {
		self
			.frontend_tasks
			.push(FrontendTask::PushToast(Translation::from_translation_key(
				"SAVED_TO_FAVOURITES",
			)));
		self.frontend_tasks.push(FrontendTask::PlaySound(SoundType::Save));

		config.pinned_apps.push(PinnedApp {
			app_id: self.entry.app_id.clone(),
			compositor_mode: self.compositor_mode,
			pos_mode: self.pos_mode,
			orientation_mode: self.orientation_mode,
			res_mode: self.res_mode,
		});

		if let Some(c) = &self.on_app_pins_changed {
			c();
		}

		self.close();
	}

	fn action_launch<T>(&mut self, interface: &mut BoxDashInterface<T>, data: &mut T) {
		View::try_launch(LaunchParams {
			application: &self.entry,
			frontend_tasks: &self.frontend_tasks,
			globals: &self.globals,
			compositor_mode: self.compositor_mode,
			res_mode: self.res_mode,
			pos_mode: self.pos_mode,
			orientation_mode: self.orientation_mode,
			auto_start: self.auto_start,
			interface,
			data,
			on_launched: self.on_close_request.take(),
		});
	}

	fn try_launch<T>(params: LaunchParams<T>) {
		let globals = params.globals.clone();
		let frontend_tasks = params.frontend_tasks.clone();

		// launch app itself
		let Err(e) = View::launch(params) else { return };

		let str_failed = globals.i18n().translate("FAILED_TO_LAUNCH_APPLICATION");
		frontend_tasks.push(FrontendTask::PushToast(Translation::from_raw_text_string(format!(
			"{} {:?}",
			str_failed, e
		))));
	}

	fn launch<T>(mut params: LaunchParams<T>) -> anyhow::Result<()> {
		let mut env = Vec::<String>::new();

		if params.compositor_mode == AppCompositorMode::Native {
			// This list could be larger, feel free to expand it
			env.push("QT_QPA_PLATFORM=wayland".into());
			env.push("GDK_BACKEND=wayland".into());
			env.push("SDL_VIDEODRIVER=wayland".into());
			env.push("XDG_SESSION_TYPE=wayland".into());
			env.push("ELECTRON_OZONE_PLATFORM_HINT=wayland".into());
		}

		let args = match params.compositor_mode {
			AppCompositorMode::Cage => format!("-- {} {}", params.application.exec_path, params.application.exec_args),
			AppCompositorMode::Native => params.application.exec_args.to_string(),
		};

		let exec = match params.compositor_mode {
			AppCompositorMode::Cage => "cage".to_string(),
			AppCompositorMode::Native => params.application.exec_path.to_string(),
		};

		let pos_mode = match params.pos_mode {
			AppPosMode::Floating => PositionMode::Float,
			AppPosMode::Anchored => PositionMode::Anchor,
			AppPosMode::Static => PositionMode::Static,
		};

		let mut userdata = HashMap::new();
		userdata.insert("desktop-entry".to_string(), serde_json::to_string(params.application)?);

		let resolution = Self::calculate_resolution(params.res_mode, params.orientation_mode);

		params.interface.process_launch(
			params.data,
			params.auto_start,
			WvrProcessLaunchParams {
				env,
				exec,
				name: params.application.app_name.to_string(),
				args,
				resolution,
				pos_mode,
				icon: params.application.icon_path.as_ref().map(|x| x.as_ref().to_string()),
				userdata,
			},
		)?;

		params
			.frontend_tasks
			.push(FrontendTask::PushToast(Translation::from_translation_key(
				"APPLICATION_STARTED",
			)));

		params.frontend_tasks.push(FrontendTask::PlaySound(SoundType::Launch));

		if let Some(on_launched) = params.on_launched.take() {
			on_launched();
		}

		// we're done!
		Ok(())
	}

	fn calculate_resolution(res_mode: AppResMode, orientation_mode: AppOrientationMode) -> [u32; 2] {
		let total_pixels = match res_mode {
			AppResMode::Res1440 => 2560 * 1440,
			AppResMode::Res1080 => 1920 * 1080,
			AppResMode::Res720 => 1280 * 720,
			AppResMode::Res480 => 854 * 480,
		};

		let (ratio_w, ratio_h) = match orientation_mode {
			AppOrientationMode::Wide => (16, 9),
			AppOrientationMode::SemiWide => (3, 2),
			AppOrientationMode::Square => (1, 1),
			AppOrientationMode::SemiTall => (2, 3),
			AppOrientationMode::Tall => (9, 16),
		};

		let k = ((total_pixels as f64) / (ratio_w * ratio_h) as f64).sqrt();

		let width = (ratio_w as f64 * k).round() as u64;
		let height = (ratio_h as f64 * k).round() as u64;

		[width as u32, height as u32]
	}
}

pub fn mount_popup(
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	entry: DesktopEntry,
	popup: PopupHolder<View>,
	on_app_pins_changed: Box<dyn Fn()>,
	pinned_app: Option<PinnedApp>,
) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_raw_text(&entry.app_name),
			Box::new(move |data| {
				let on_close_request = popup.get_close_callback(data.layout);
				let view = View::new(Params {
					entry: entry.clone(),
					globals: &globals,
					layout: data.layout,
					parent_id: data.id_content,
					frontend_tasks: &frontend_tasks,
					config: data.config,
					on_close_request,
					on_app_pins_changed,
					pinned_app,
				})?;

				popup.set_view(data.handle, view, None);
				Ok(popup.get_close_callback(data.layout))
			}),
			Default::default(), /* extra */
		)));
}
