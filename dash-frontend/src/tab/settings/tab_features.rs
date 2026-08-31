use std::path::PathBuf;
use std::rc::Rc;

use wgui::{
	components::{
		button::{ButtonClickEvent, ComponentButton},
		checkbox::ComponentCheckbox,
	},
	globals::WguiGlobals,
	i18n::Translation,
	layout::WidgetID,
	log::LogErr,
	parser::{Fetchable, TemplateParams},
	task::Tasks,
	widget::label::WidgetLabel,
	windowing::context_menu::{self},
};
use wlx_common::{async_executor::AsyncExecutor, config::GeneralConfig, dash_interface::ConfigChangeKind};

use crate::util::downloadable_file::DownloadableFile;
use crate::{
	frontend::FrontendTasks,
	tab::settings::{
		SettingType, SettingsMountParams, SettingsTab, TabNameEnum, Task as ParentTask, UpdateExtra, horiz_cell,
		macros::{MacroParams, options_category, options_checkbox, options_range_f32},
		mount_requires_restart,
	},
	util::{
		popup_manager::PopupHolder,
		swipe_type::{SWIPE_TYPE_MODEL, swipe_type_delete_all_models, swipe_type_model_path, swwipe_type_model_downloaded},
		whisper::{
			WHISPER_MODELS, whisper_any_models_downloaded, whisper_delete_all_models, whisper_model_from_name,
			whisper_model_path,
		},
	},
	views::{self, ViewUpdateParams},
};

#[derive(Clone)]
enum Task {
	WhisperDownloadClosed,
	WhisperRemoveUnused,
	WhisperDownload(&'static DownloadableFile),
	WhisperDownloadDone,
	SwipeTypeDownloadClosed,
	SwipeTypeRemoveAll,
	SwipeTypeDownloadAll,
	SwipeTypeDownloadDone,
	CloseDialog,
	ReloadTab,
}

pub struct State {
	popup_download: PopupHolder<views::download_file::View>,
	popup_dialog: PopupHolder<views::dialog_box::View>,
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	tasks: Tasks<Task>,
	parent_tasks: Tasks<ParentTask>,
	pending_whisper_download: Option<&'static DownloadableFile>,
	pending_swipe_download: Option<&'static DownloadableFile>,
}

impl SettingsTab for State {
	fn update(&mut self, par: &mut ViewUpdateParams, _extra: &UpdateExtra) -> anyhow::Result<()> {
		self.popup_download.update(par)?;
		self.popup_dialog.update(par)?;

		for task in self.tasks.drain() {
			match task {
				Task::WhisperDownloadClosed => {
					if let Some(model) = self.pending_whisper_download.take()
						&& !whisper_model_path(model.file_name).exists()
					{
						// download failed, set to selection to none
						par.general_config.whisper_model = "".into();
						par.config_change_kind.replace(ConfigChangeKind::Other);
						// reload the tab
						self.parent_tasks.push(ParentTask::SetTab(TabNameEnum::Features));
					}
				}
				Task::WhisperRemoveUnused => {
					let _ = whisper_delete_all_models().log_err("could not remove whisper models");
				}
				Task::WhisperDownload(model) => {
					self.pending_whisper_download = Some(model);
					self.show_download_dialogue(
						model,
						par.executor.clone(),
						whisper_model_path(model.file_name),
						Task::WhisperDownloadClosed,
						Task::WhisperDownloadDone,
					);
				}
				Task::WhisperDownloadDone => {
					if let Some(model) = self.pending_whisper_download.take() {
						par.general_config.whisper_model = model.file_name.into();
						par.config_change_kind.replace(ConfigChangeKind::Other);

						// reload tab so that the downloaded check marks get populated
						self.parent_tasks.push(ParentTask::SetTab(TabNameEnum::Features));
					}
				}
				Task::SwipeTypeDownloadClosed => {
					self.pending_swipe_download = None;
					if !swwipe_type_model_downloaded().unwrap_or_default() {
						par.general_config.keyboard_swipe_to_type_enabled = false;
						par.config_change_kind.replace(ConfigChangeKind::Other);
						self.parent_tasks.push(ParentTask::SetTab(TabNameEnum::Features));
					}
				}
				Task::SwipeTypeRemoveAll => {
					let _ = swipe_type_delete_all_models().log_err("could not remove swipe type models");
					par.general_config.keyboard_swipe_to_type_enabled = false;
				}
				Task::SwipeTypeDownloadAll => {
					self.pending_swipe_download = Some(&SWIPE_TYPE_MODEL);
					self.show_download_dialogue(
						&SWIPE_TYPE_MODEL,
						par.executor.clone(),
						swipe_type_model_path(SWIPE_TYPE_MODEL.file_name),
						Task::SwipeTypeDownloadClosed,
						Task::SwipeTypeDownloadDone,
					);
				}
				Task::SwipeTypeDownloadDone => {
					if let Some(_) = self.pending_swipe_download.take() {
						par.config_change_kind.replace(ConfigChangeKind::Other);
						self.parent_tasks.push(ParentTask::SetTab(TabNameEnum::Features));
					}
				}
				Task::CloseDialog => {
					let close_dialog = self.popup_dialog.get_close_callback(par.layout);
					close_dialog();
				}
				Task::ReloadTab => {
					self.parent_tasks.push(ParentTask::SetTab(TabNameEnum::Features));
				}
			}
		}

		Ok(())
	}

	fn context_menu_custom(
		&mut self,
		action: Rc<str>,
		config: &mut GeneralConfig,
		change_kind: &mut Option<ConfigChangeKind>,
		layout: &mut wgui::layout::Layout,
		state: &mut wgui::parser::ParserState,
	) -> anyhow::Result<()> {
		if let (Some(_), Some(id), Some(model_name)) = {
			let mut s = action.splitn(3, ';');
			(s.next(), s.next(), s.next())
		} {
			if !model_name.is_empty()
				&& let Some(model) = whisper_model_from_name(model_name)
			{
				let mut common = layout.common();
				let mut label = state.fetch_widget_as::<WidgetLabel>(common.state, &format!("{id}_value"))?;
				label.set_text(&mut common, Translation::from_raw_text(model.display_name));

				config.whisper_model = model.file_name.into();
				if !whisper_model_path(model.file_name).exists() {
					self.show_whisper_model_dialog_box_download(model)?;
				} else {
					change_kind.replace(ConfigChangeKind::Other);
				}
			} else {
				config.whisper_model = "".into();
				change_kind.replace(ConfigChangeKind::Other);
				// re-init the whole tab
				if whisper_any_models_downloaded().unwrap_or_default() {
					self.show_whisper_model_dialog_box_cleanup()?;
				} else {
					self.parent_tasks.push(ParentTask::SetTab(TabNameEnum::Features));
				}
			}
		}

		Ok(())
	}

	fn push_task_string(
		&mut self,
		action: &str,
		_config: &mut GeneralConfig,
		_change_kind: &mut Option<ConfigChangeKind>,
		_layout: &mut wgui::layout::Layout,
		_state: &mut wgui::parser::ParserState,
	) -> anyhow::Result<()> {
		match action {
			"swipe_type_download" => {
				self.show_swipe_type_model_dialog_box_download()?;
			}
			"swipe_type_remove" => {
				self.tasks.push(Task::SwipeTypeRemoveAll);
				self.tasks.push(Task::ReloadTab);
			}
			_ => {}
		}
		Ok(())
	}
}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let tasks = Tasks::<Task>::new();
		let popup_download = PopupHolder::<views::download_file::View>::default();
		let popup_dialog = PopupHolder::<views::dialog_box::View>::default();

		let c = options_category(par.mp, par.id_parent, "APP_SETTINGS.FEATURES", "dashboard/options.svg")?;

		if par.feats.whisper {
			whisper_models_dropdown(par.mp, c)?;
		}

		if par.feats.swipe_to_type {
			swipe_type_models_button(par.mp, c)?;
			swipe_type_enabled_checkbox(par.mp, c)?;
		}
		options_checkbox(par.mp, c, SettingType::NotificationsEnabled)?;
		options_checkbox(par.mp, c, SettingType::NotificationsSoundEnabled)?;
		options_checkbox(par.mp, c, SettingType::KeyboardSoundEnabled)?;
		if par.feats.xr_backend.is_open_vr() || par.feats.monado {
			// monado or openvr
			options_checkbox(par.mp, c, SettingType::BlockGameInput)?;
			options_checkbox(par.mp, c, SettingType::BlockGameInputIgnoreWatch)?;
		}
		if par.feats.monado {
			// monado-only
			options_checkbox(par.mp, c, SettingType::BlockPosesOnKbdInteraction)?;
		}

		options_range_f32(
			par.mp,
			c,
			SettingType::WatchViewAngleMin,
			SettingType::WatchViewAngleMax,
			0.1,
			1.0,
			0.1,
		)?;
		Ok(State {
			tasks,
			parent_tasks: par.mp.tasks.clone(),
			popup_download,
			popup_dialog,
			frontend_tasks: par.frontend_tasks.clone(),
			globals: par.mp.doc_params.globals.clone(),
			pending_whisper_download: None,
			pending_swipe_download: None,
		})
	}

	fn show_download_dialogue(
		&mut self,
		file: &DownloadableFile,
		executor: AsyncExecutor,
		target_path: PathBuf,
		on_closed: Task,
		on_downloaded: Task,
	) {
		views::download_file::mount_popup(
			self.popup_download.clone(),
			self.frontend_tasks.clone(),
			self.tasks.make_callback_box(on_closed),
			views::download_file::Params {
				globals: self.globals.clone(),
				executor,
				target_path,
				url: file.url.into(),
				on_downloaded: self.tasks.make_callback_box(on_downloaded),
			},
		);
	}

	fn show_whisper_model_dialog_box_download(&mut self, model: &'static DownloadableFile) -> anyhow::Result<()> {
		const ACTION_DOWNLOAD: &str = "download";
		const ACTION_CANCEL: &str = "cancel";

		self.pending_whisper_download = Some(model);

		let tasks = self.tasks.clone();
		views::dialog_box::mount_popup(
			self.popup_dialog.clone(),
			self.frontend_tasks.clone(),
			views::dialog_box::Params {
				globals: self.globals.clone(),
				message: Translation::from_translation_key("APP_SETTINGS.WHISPER.NEED_TO_DOWNLOAD_MODEL"),
				entries: vec![
					views::dialog_box::ButtonEntry {
						content: Translation::from_translation_key("APP_SETTINGS.CANCEL"),
						icon: "dashboard/close.svg",
						action: ACTION_CANCEL,
					},
					views::dialog_box::ButtonEntry {
						content: Translation::from_translation_key("DOWNLOAD"),
						icon: "dashboard/download.svg",
						action: ACTION_DOWNLOAD,
					},
				],
				on_action_click: Box::new(move |action| match action {
					ACTION_DOWNLOAD => {
						tasks.push(Task::WhisperDownload(model));
					}
					ACTION_CANCEL => {
						tasks.push(Task::CloseDialog);
						// treat as failed download
						tasks.push(Task::WhisperDownloadClosed);
					}
					_ => unreachable!(),
				}),
			},
		);

		Ok(())
	}

	fn show_swipe_type_model_dialog_box_download(&mut self) -> anyhow::Result<()> {
		const ACTION_DOWNLOAD: &str = "download";
		const ACTION_CANCEL: &str = "cancel";

		let tasks = self.tasks.clone();
		views::dialog_box::mount_popup(
			self.popup_dialog.clone(),
			self.frontend_tasks.clone(),
			views::dialog_box::Params {
				globals: self.globals.clone(),
				message: Translation::from_translation_key("APP_SETTINGS.SWIPE_TYPE.NEED_TO_DOWNLOAD_MODEL"),
				entries: vec![
					views::dialog_box::ButtonEntry {
						content: Translation::from_translation_key("APP_SETTINGS.CANCEL"),
						icon: "dashboard/close.svg",
						action: ACTION_CANCEL,
					},
					views::dialog_box::ButtonEntry {
						content: Translation::from_translation_key("DOWNLOAD"),
						icon: "dashboard/download.svg",
						action: ACTION_DOWNLOAD,
					},
				],
				on_action_click: Box::new(move |action| match action {
					ACTION_DOWNLOAD => {
						tasks.push(Task::SwipeTypeDownloadAll);
					}
					ACTION_CANCEL => {
						tasks.push(Task::CloseDialog);
						tasks.push(Task::SwipeTypeDownloadClosed);
					}
					_ => unreachable!(),
				}),
			},
		);

		Ok(())
	}

	fn show_whisper_model_dialog_box_cleanup(&mut self) -> anyhow::Result<()> {
		const ACTION_REMOVE: &str = "remove";
		const ACTION_CANCEL: &str = "cancel";

		let tasks = self.tasks.clone();

		views::dialog_box::mount_popup(
			self.popup_dialog.clone(),
			self.frontend_tasks.clone(),
			views::dialog_box::Params {
				globals: self.globals.clone(),
				message: Translation::from_translation_key("APP_SETTINGS.WHISPER.REMOVE_UNUSED_MODELS"),
				entries: vec![
					views::dialog_box::ButtonEntry {
						content: Translation::from_translation_key("APP_SETTINGS.CANCEL"),
						icon: "dashboard/close.svg",
						action: ACTION_CANCEL,
					},
					views::dialog_box::ButtonEntry {
						content: Translation::from_translation_key("REMOVE"),
						icon: "dashboard/trash.svg",
						action: ACTION_REMOVE,
					},
				],
				on_action_click: Box::new(move |action| match action {
					ACTION_REMOVE => {
						tasks.push(Task::WhisperRemoveUnused);
						tasks.push(Task::CloseDialog);
						tasks.push(Task::ReloadTab);
					}
					ACTION_CANCEL => {
						tasks.push(Task::CloseDialog);
						tasks.push(Task::ReloadTab);
					}
					_ => unreachable!(),
				}),
			},
		);

		Ok(())
	}
}

fn whisper_models_dropdown(mp: &mut MacroParams, parent: WidgetID) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let parent = horiz_cell(mp.layout, parent)?;

	let mut downloaded_models = vec![];
	for (idx, m) in WHISPER_MODELS.iter().enumerate() {
		if whisper_model_path(m.file_name).exists() {
			downloaded_models.push(idx);
		}
	}

	let current_file = mp.config.whisper_model.as_ref();
	let current_translation = whisper_model_from_name(current_file)
		.map_or(Translation::from_translation_key("APP_SETTINGS.OPTION.NONE"), |x| {
			Translation::from_raw_text(x.display_name)
		});

	let mut params = TemplateParams::new();
	params.insert("id", &id);
	params.insert("translation", "APP_SETTINGS.WHISPER_MODEL");
	params.insert("tooltip", "APP_SETTINGS.WHISPER_MODEL_HELP");

	mp.parser_state
		.instantiate_template(mp.doc_params, "DropdownButton", mp.layout, parent, params)?;

	{
		let mut label = mp
			.parser_state
			.fetch_widget_as::<WidgetLabel>(&mp.layout.state, &format!("{id}_value"))?;
		label.set_text_simple(&mut mp.layout.state.globals.get(), current_translation);
	}

	let btn = mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
	btn.on_click(Rc::new({
		let parent_tasks = mp.tasks.clone();
		move |_common, e: ButtonClickEvent| {
			let mut cells = WHISPER_MODELS
				.iter()
				.enumerate()
				.map(|(idx, item)| context_menu::Cell {
					action_name: Some(format!(";{id};{}", item.file_name).into()),
					title: Translation::from_raw_text_string(format!(
						"{}{}",
						item.display_name,
						if downloaded_models.contains(&idx) { " ✅" } else { "" }
					)),
					tooltip: None,
					attribs: vec![],
				})
				.collect::<Vec<_>>();

			cells.insert(
				0,
				context_menu::Cell {
					action_name: Some(format!(";{id};").into()),
					title: Translation::from_translation_key("APP_SETTINGS.OPTION.NONE"),
					tooltip: None,
					attribs: vec![],
				},
			);

			parent_tasks.push(ParentTask::OpenContextMenu(
				e.mouse_pos_absolute.unwrap_or_default(),
				cells,
			));
			Ok(())
		}
	}));

	Ok(())
}

fn swipe_type_models_button(mp: &mut MacroParams, parent: WidgetID) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let id_cell = horiz_cell(mp.layout, parent)?;

	let all_downloaded = swwipe_type_model_downloaded().unwrap_or_default();
	let (translation, icon, action) = if all_downloaded {
		(
			"APP_SETTINGS.SWIPE_TYPE.REMOVE_MODEL",
			"dashboard/trash.svg",
			"swipe_type_remove",
		)
	} else {
		(
			"APP_SETTINGS.SWIPE_TYPE.DOWNLOAD_MODEL",
			"dashboard/download.svg",
			"swipe_type_download",
		)
	};

	let mut params = TemplateParams::new();
	params.insert("id", &id);
	params.insert("translation", translation);
	params.insert("icon", icon);

	mp.parser_state
		.instantiate_template(mp.doc_params, "ButtonText", mp.layout, id_cell, params)?;

	let btn = mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
	btn.on_click(Rc::new({
		let parent_tasks = mp.tasks.clone();
		let action = Rc::<str>::from(action);
		move |_common, _e: ButtonClickEvent| {
			parent_tasks.push(ParentTask::SwipeTypeAction(action.clone()));
			Ok(())
		}
	}));

	Ok(())
}

fn swipe_type_enabled_checkbox(mp: &mut MacroParams, parent: WidgetID) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let setting = SettingType::KeyboardSwipeToTypeEnabled;

	// cannot enable swipe-to-type until the model is downloaded
	let model_downloaded = swwipe_type_model_downloaded().unwrap_or_default();

	let mut params = TemplateParams::new();
	params.insert("id", &id);

	match setting.get_translation() {
		Ok(translation) => params.insert("translation", translation),
		Err(raw_text) => params.insert("text", raw_text),
	};

	let tooltip = if model_downloaded {
		setting.get_tooltip()
	} else {
		Some("APP_SETTINGS.SWIPE_TYPE.MODEL_REQUIRED_TO_ENABLE")
	};
	if let Some(tooltip) = tooltip {
		params.insert("tooltip", tooltip);
	}

	let checked = if *setting.mut_bool(mp.config) { "1" } else { "0" };
	params.insert("checked", checked);

	let id_cell = horiz_cell(mp.layout, parent)?;

	mp.parser_state
		.instantiate_template(mp.doc_params, "CheckBoxSetting", mp.layout, id_cell, params)?;

	if setting.requires_restart() {
		mount_requires_restart(mp.layout, id_cell)?;
	}

	let checkbox = mp.parser_state.fetch_component_as::<ComponentCheckbox>(&id)?;

	if !model_downloaded {
		let mut common = mp.layout.common();
		checkbox.set_disabled(&mut common, true);
	}

	checkbox.on_toggle(Box::new({
		let tasks = mp.tasks.clone();
		move |_common, e| {
			tasks.push(ParentTask::UpdateBool(setting, e.checked));
			Ok(())
		}
	}));

	Ok(())
}
