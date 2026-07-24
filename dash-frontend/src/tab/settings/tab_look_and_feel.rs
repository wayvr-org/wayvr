use std::rc::Rc;

use wgui::{
	components::button::ComponentButton,
	globals::WguiGlobals,
	layout::WidgetID,
	parser::{Fetchable, TemplateParams},
	task::Tasks,
};

use crate::{
	frontend::FrontendTasks,
	tab::settings::{
		SettingType, SettingsMountParams, SettingsTab, horiz_cell,
		macros::{options_category, options_checkbox, options_dropdown, options_slider_f32},
		mount_requires_restart,
	},
	util::popup_manager::PopupHolder,
	views::{ViewUpdateParams, color_palettes},
};

#[derive(Clone)]
enum Task {
	OpenColorPalettes,
}

pub struct State {
	popup_color_palettes: PopupHolder<color_palettes::View>,
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	tasks: Tasks<Task>,
}

impl SettingsTab for State {
	fn update(&mut self, par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		self.popup_color_palettes.update(par)?;

		for task in self.tasks.drain() {
			match task {
				Task::OpenColorPalettes => {
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
		let tasks = Tasks::<Task>::new();
		let popup = PopupHolder::<color_palettes::View>::default();

		let c = options_category(
			par.mp,
			par.id_parent,
			"APP_SETTINGS.LOOK_AND_FEEL",
			"dashboard/palette.svg",
		)?;
		create_color_palettes_button(par.mp, c, tasks.clone(), &popup)?;
		options_dropdown::<wlx_common::locale::Language>(par.mp, c, &SettingType::Language)?;
		options_checkbox(par.mp, c, SettingType::OpaqueBackground)?;
		options_checkbox(par.mp, c, SettingType::HideUsername)?;
		options_checkbox(par.mp, c, SettingType::HideGrabHelp)?;
		options_slider_f32(par.mp, c, SettingType::DefaultOverlayScale, 0.7, 1.5, 0.05)?; // min, max, step
		options_slider_f32(par.mp, c, SettingType::UiAnimationSpeed, 0.5, 5.0, 0.1)?; // min, max, step
		options_slider_f32(par.mp, c, SettingType::UiGradientIntensity, 0.0, 1.0, 0.05)?; // min, max, step
		options_slider_f32(par.mp, c, SettingType::UiRoundMultiplier, 0.1, 5.0, 0.1)?;
		options_checkbox(par.mp, c, SettingType::EnableWatch)?;
		options_checkbox(par.mp, c, SettingType::SetsOnWatch)?;
		options_checkbox(par.mp, c, SettingType::Clock12h)?;
		Ok(State {
			popup_color_palettes: popup,
			frontend_tasks: par.frontend_tasks.clone(),
			globals: par.mp.doc_params.globals.clone(),
			tasks,
		})
	}
}

fn create_color_palettes_button(
	mp: &mut crate::tab::settings::macros::MacroParams,
	parent: WidgetID,
	tasks: Tasks<Task>,
	_popup: &PopupHolder<color_palettes::View>,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let id_cell = horiz_cell(mp.layout, parent)?;

	let mut params = TemplateParams::new();
	params.insert("id", &id);
	params.insert("translation", "APP_SETTINGS.COLOR_PALETTE");
	params.insert("icon", "dashboard/palette.svg");

	mp.parser_state
		.instantiate_template(mp.doc_params, "ButtonText", mp.layout, id_cell, params)?;

	let btn = mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
	btn.on_click(Rc::new({
		let tasks = tasks.clone();
		move |_common, _e| {
			tasks.push(Task::OpenColorPalettes);
			Ok(())
		}
	}));

	mount_requires_restart(mp.layout, id_cell)?;

	Ok(())
}
