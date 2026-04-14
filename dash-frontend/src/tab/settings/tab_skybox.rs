use wgui::{assets::AssetPath, i18n::Translation, layout::Layout, task::Tasks};

use crate::{
	frontend::FrontendTasks,
	tab::settings::{
		SettingType, SettingsMountParams, SettingsTab,
		macros::{options_category, options_checkbox},
	},
	util::{popup_manager::PopupHolder, wgui_simple},
	views::{self, ViewUpdateParams, skymap_list},
};

#[derive(Clone)]
enum Task {
	ShowSkymapList,
}

pub struct State {
	popup_skymap_list: PopupHolder<skymap_list::View>,
	tasks: Tasks<Task>,
	frontend_tasks: FrontendTasks,
}

impl SettingsTab for State {
	fn update(&mut self, par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		self.popup_skymap_list.update(par)?;

		for task in self.tasks.drain() {
			match task {
				Task::ShowSkymapList => self.show_skymap_list(par.layout),
			}
		}

		Ok(())
	}
}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<Self> {
		let id_category = options_category(par.mp, par.id_parent, "APP_SETTINGS.SKYBOX", "dashboard/globe.svg")?;
		options_checkbox(par.mp, id_category, SettingType::UseSkybox)?;
		options_checkbox(par.mp, id_category, SettingType::OpaqueBackground)?;

		let tasks = Tasks::<Task>::new();

		// "Browse skymaps" button
		wgui_simple::create_button(wgui_simple::CreateButtonParams {
			id_parent: id_category,
			layout: par.mp.layout,
			content: Translation::from_translation_key("APP_SETTINGS.BROWSE_SKYMAPS"),
			icon_builtin: AssetPath::BuiltIn("dashboard/globe.svg"),
			on_click: tasks.get_button_click_callback(Task::ShowSkymapList),
		})?;

		Ok(Self {
			popup_skymap_list: Default::default(),
			frontend_tasks: par.frontend_tasks.clone(),
			tasks,
		})
	}

	fn show_skymap_list(&mut self, layout: &mut Layout) {
		views::skymap_list::mount_popup(
			self.frontend_tasks.clone(),
			layout.state.globals.clone(),
			self.popup_skymap_list.clone(),
		);
	}
}
