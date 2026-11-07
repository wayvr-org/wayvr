use std::rc::Rc;

use crate::{
	frontend::FrontendTasks,
	tab::settings::{
		SettingType, SettingsMountParams, SettingsTab, UpdateExtra, horiz_cell,
		macros::{options_category, options_checkbox, options_slider},
		mount_requires_restart,
	},
	util::{popup_manager::PopupHolder, wgui_simple},
	views::{self, ViewUpdateParams, color_palettes, skymap_list},
};
use wgui::{
	assets::AssetPathRef,
	components::button::ComponentButton,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, TemplateParams},
	task::Tasks,
};

#[derive(Clone)]
enum LocalTask {
	OpenColorPalettes,
	ShowSkymapList,
}

pub struct State {
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	tasks: Tasks<LocalTask>,
	popup_color_palettes: PopupHolder<color_palettes::View>,
	popup_skymap_list: PopupHolder<skymap_list::View>,
}

impl SettingsTab for State {
	fn update(&mut self, par: &mut ViewUpdateParams, _extra: &UpdateExtra) -> anyhow::Result<()> {
		self.popup_skymap_list.update(par)?;
		self.popup_color_palettes.update(par)?;

		for task in self.tasks.drain() {
			match task {
				LocalTask::ShowSkymapList => self.show_skymap_list(par.layout),
				LocalTask::OpenColorPalettes => {
					color_palettes::mount_popup(
						self.frontend_tasks.clone(),
						self.globals.clone(),
						self.popup_color_palettes.clone(),
						par.general_config.color_palette.clone(),
					);
				}
			}
		}

		Ok(())
	}
}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let popup_color_palettes = PopupHolder::<color_palettes::View>::default();
		let popup_skymap_list = PopupHolder::<skymap_list::View>::default();
		let tasks = Tasks::<LocalTask>::new();

		let c = options_category(
			par.mp,
			par.id_parent,
			"APP_SETTINGS.APPEARANCE",
			"@/dashboard/palette.svg",
		)?;

		// Dashboard category
		{
			let c = options_category(par.mp, c, "DEFINITIONS.DASHBOARD", "@/dashboard/dashboard.svg")?;

			create_color_palettes_button(par.mp, c, tasks.clone(), &popup_color_palettes)?;
			options_checkbox(par.mp, c, SettingType::OpaqueBackground)?;
			options_checkbox(par.mp, c, SettingType::ShowUsername)?;
		}

		// Skymap category
		{
			let c = options_category(par.mp, c, "DEFINITIONS.SKYMAP", "@/dashboard/globe.svg")?;
			options_checkbox(par.mp, c, SettingType::UseSkybox)?;
			options_checkbox(par.mp, c, SettingType::UsePassthrough)?;
			// "Browse skymaps" button
			wgui_simple::create_button(wgui_simple::CreateButtonParams {
				id_parent: c,
				layout: par.mp.layout,
				content: Translation::from_translation_key("APP_SETTINGS.BROWSE_SKYMAPS"),
				icon_builtin: AssetPathRef::BuiltIn("dashboard/globe.svg"),
				on_click: tasks.get_button_click_callback(LocalTask::ShowSkymapList),
			})?;

			options_slider(par.mp, c, SettingType::GridOpacity, 0.0, 1.0, 0.05)?;
		}

		Ok(State {
			tasks,
			popup_color_palettes,
			popup_skymap_list,
			frontend_tasks: par.frontend_tasks.clone(),
			globals: par.mp.doc_params.globals.clone(),
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

fn create_color_palettes_button(
	mp: &mut crate::tab::settings::macros::MacroParams,
	parent: WidgetID,
	tasks: Tasks<LocalTask>,
	_popup: &PopupHolder<color_palettes::View>,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let id_cell = horiz_cell(mp.layout, parent)?;

	let mut params = TemplateParams::new();
	params.insert("id", &id);
	params.insert("translation", "APP_SETTINGS.COLOR_PALETTE");
	params.insert("icon", "@/dashboard/palette.svg");

	mp.parser_state
		.instantiate_template(mp.doc_params, "ButtonText", mp.layout, id_cell, params)?;

	let btn = mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
	btn.on_click(Rc::new({
		let tasks = tasks.clone();
		move |_common, _e| {
			tasks.push(LocalTask::OpenColorPalettes);
			Ok(())
		}
	}));

	mount_requires_restart(mp.layout, id_cell)?;

	Ok(())
}
