use std::rc::Rc;

use wgui::{
	globals::WguiGlobals,
	i18n::Translation,
	layout::WidgetID,
	log::LogErr,
	parser::{Fetchable, TemplateParams},
	task::Tasks,
	widget::label::WidgetLabel,
	windowing::context_menu,
};
use wlx_common::{async_executor::AsyncExecutor, config::GeneralConfig, dash_interface::ConfigChangeKind};

use crate::{
	frontend::FrontendTasks,
	tab::settings::{
		SettingType, SettingsMountParams, SettingsTab, TabNameEnum, Task as ParentTask, UpdateExtra, horiz_cell,
		macros::{MacroParams, options_category, options_checkbox, options_range_f32},
	},
	util::{
		popup_manager::PopupHolder,
		wgui_simple,
		whisper::{
			WHISPER_MODELS, WhisperModel, whisper_any_models_downloaded, whisper_delete_all_models, whisper_model_from_name,
			whisper_model_path,
		},
	},
	views::{self, ViewUpdateParams},
};

pub struct State {
	popup_download: PopupHolder<views::download_file::View>,
	popup_dialog: PopupHolder<views::dialog_box::View>,
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	tasks: Tasks<Task>,
	parent_tasks: Tasks<ParentTask>,
}

impl SettingsTab for State {}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let tasks = Tasks::<Task>::new();
		let popup_download = PopupHolder::<views::download_file::View>::default();
		let popup_dialog = PopupHolder::<views::dialog_box::View>::default();

		let c = options_category(
			par.mp,
			par.id_parent,
			"APP_SETTINGS.FEATURES",
			"@/dashboard/options.svg",
		)?;

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
			popup_download,
			popup_dialog,
			frontend_tasks: par.frontend_tasks.clone(),
			globals: par.mp.doc_params.globals.clone(),
		})
	}
}
