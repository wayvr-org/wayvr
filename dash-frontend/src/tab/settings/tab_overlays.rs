use std::{rc::Rc, sync::Arc};
use wgui::{
	assets::AssetPathRef,
	components::checkbox::ComponentCheckbox,
	i18n::Translation,
	layout::WidgetID,
	parser::{Fetchable, TemplateParams},
	task::Tasks,
};
use wlx_common::{astr_containers::AStrSetExt, config_io::get_gui_root};

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	tab::settings::{
		SettingType, SettingsMountParams, SettingsTab, Task, horiz_cell,
		macros::{
			MacroParams, options_category, options_checkbox, options_dropdown, options_range_f32, options_slider,
			options_split_horiz2,
		},
	},
	util::{popup_manager::PopupHolder, wgui_simple},
	views::keyboard_layout_editor,
};

#[derive(Clone)]
enum LocalTask {
	OpenCustomizationGuide,
	OpenKeyboardLayoutEditor,
}

pub struct State {
	tasks: Tasks<LocalTask>,
	frontend_tasks: FrontendTasks,
	popup_keyboard_layout_editor: PopupHolder<keyboard_layout_editor::View>,
}

impl SettingsTab for State {
	fn update(&mut self, par: &mut crate::views::ViewUpdateParams, _extra: &super::UpdateExtra) -> anyhow::Result<()> {
		self.popup_keyboard_layout_editor.update(par)?;

		for task in self.tasks.drain() {
			match task {
				LocalTask::OpenCustomizationGuide => {
					self.frontend_tasks.push(FrontendTask::OpenURL(Rc::from(
						"https://wayvr.org/docs/basics/customization/",
					)));
				}
				LocalTask::OpenKeyboardLayoutEditor => {
					self.open_keyboard_layout_editor();
				}
			}
		}

		Ok(())
	}
}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let c = options_category(par.mp, par.id_parent, "APP_SETTINGS.OVERLAYS", "@/dashboard/window.svg")?;

		let tasks = Tasks::new();

		// Keyboard
		{
			let c = options_category(par.mp, c, "DEFINITIONS.KEYBOARD", "@/dashboard/keyboard.svg")?;
			options_dropdown::<wlx_common::config::AltModifier>(par.mp, c, &SettingType::KeyboardMiddleClick)?;
			options_dropdown::<wlx_common::keyboard::KeyboardModel>(par.mp, c, &SettingType::KeyboardModel)?;

			wgui_simple::create_button(wgui_simple::CreateButtonParams {
				id_parent: c,
				layout: par.mp.layout,
				content: Translation::from_translation_key("APP_SETTINGS.KEYBOARD_LAYOUT"),
				icon_builtin: AssetPathRef::BuiltIn("dashboard/keys.svg"),
				on_click: tasks.get_button_click_callback(LocalTask::OpenKeyboardLayoutEditor),
			})?;
		}

		// Watch
		{
			let c = options_category(par.mp, c, "APP_SETTINGS.WATCH", "@/dashboard/watch.svg")?;
			options_checkbox(par.mp, c, SettingType::EnableWatch)?;
			options_checkbox(par.mp, c, SettingType::SetsOnWatch)?;
			options_range_f32(
				par.mp,
				c,
				SettingType::WatchViewAngleMin,
				SettingType::WatchViewAngleMax,
				0.1,
				1.0,
				0.1,
			)?;
		}

		// Custom overlays
		{
			let c = options_category(par.mp, c, "APP_SETTINGS.CUSTOM_OVERLAYS", "@/dashboard/magic_wand.svg")?;

			wgui_simple::create_button(wgui_simple::CreateButtonParams {
				id_parent: c,
				layout: par.mp.layout,
				content: Translation::from_translation_key("APP_SETTINGS.OPEN_CUSTOMIZATION_GUIDE"),
				icon_builtin: AssetPathRef::BuiltIn("dashboard/help.svg"),
				on_click: tasks.get_button_click_callback(LocalTask::OpenCustomizationGuide),
			})?;

			let mut panels = find_custom_panels();
			panels.sort();
			panels.dedup();

			for panel in panels {
				options_custom_panel_checkbox(par.mp, c, &panel)?;
			}
		}

		// Misc
		{
			let c = options_category(par.mp, c, "APP_SETTINGS.MISC", "@/dashboard/settings.svg")?;
			options_slider(par.mp, c, SettingType::DefaultCurvature, 0.0, 0.5, 0.05)?; // min, max, step
			options_slider(par.mp, c, SettingType::DefaultOpacity, 0.05, 1.0, 0.05)?; // min, max, step
			options_slider(par.mp, c, SettingType::DefaultOverlayScale, 0.7, 1.5, 0.05)?; // min, max, step
			options_dropdown::<wlx_common::config::DefaultPositioning>(par.mp, c, &SettingType::DefaultPositioning)?;

			{
				let (id1, id2) = options_split_horiz2(par.mp, c)?;
				options_checkbox(par.mp, id1, SettingType::InvertScrollDirectionX)?;
				options_checkbox(par.mp, id2, SettingType::InvertScrollDirectionY)?;
			}

			options_slider(par.mp, c, SettingType::ScrollSpeed, 0.1, 5.0, 0.1)?;
			options_slider(par.mp, c, SettingType::LongPressDuration, 0.1, 2.0, 0.1)?;
			options_slider(par.mp, c, SettingType::ClickFreezeTimeMs, 0, 500, 50)?;
		}

		Ok(State {
			tasks,
			frontend_tasks: par.frontend_tasks.clone(),
			popup_keyboard_layout_editor: Default::default(),
		})
	}

	fn open_keyboard_layout_editor(&mut self) {
		keyboard_layout_editor::mount_popup(self.popup_keyboard_layout_editor.clone(), self.frontend_tasks.clone());
	}
}

fn find_custom_panels() -> Vec<Arc<str>> {
	let mut panels = Vec::new();

	let Ok(entries) = std::fs::read_dir(get_gui_root()) else {
		return panels;
	};

	for entry in entries.flatten() {
		let path = entry.path();

		let name = if path.is_file() && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("xml")) {
			path.file_stem()
		} else if path.is_dir() && path.join("layout.xml").is_file() {
			path.file_name()
		} else {
			continue;
		};

		if let Some(name) = name.and_then(|n| n.to_str()).filter(|n| !n.starts_with('.')) {
			panels.push(name.into());
		}
	}

	panels
}

fn options_custom_panel_checkbox(mp: &mut MacroParams, parent: WidgetID, panel: &str) -> anyhow::Result<()> {
	let id = mp.gen_id();

	let mut params = TemplateParams::new();
	params.insert("id", &id);
	params.insert("text", panel);
	params.insert(
		"checked",
		if mp.config.custom_panels.arc_get(panel) {
			"1"
		} else {
			"0"
		},
	);

	let id_cell = horiz_cell(mp.layout, parent)?;

	mp.parser_state
		.instantiate_template(mp.doc_params, "CheckBoxSetting", mp.layout, id_cell, params)?;

	let checkbox = mp.parser_state.fetch_component_as::<ComponentCheckbox>(&id)?;
	checkbox.on_toggle(Box::new({
		let tasks = mp.tasks.clone();
		let panel: Arc<str> = panel.into();
		move |_common, e| {
			tasks.push(Task::ToggleCustomPanel(panel.clone(), e.checked));
			Ok(())
		}
	}));

	Ok(())
}
