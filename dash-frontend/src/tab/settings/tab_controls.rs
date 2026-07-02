use std::{collections::HashMap, rc::Rc};

use wgui::{
	components::button::ComponentButton, globals::WguiGlobals, layout::WidgetID, parser::Fetchable, task::Tasks,
};

use crate::util::popup_manager::PopupHolder;

use crate::{
	frontend::FrontendTasks,
	tab::settings::{
		SettingType, SettingsMountParams, SettingsTab,
		macros::{options_category, options_checkbox, options_dropdown, options_slider_f32, options_slider_i32},
	},
	views::{ViewUpdateParams, input_profiles},
};

#[derive(Clone)]
enum Task {
	OpenInputProfiles,
}

pub struct State {
	popup_input_profiles: PopupHolder<input_profiles::View>,
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	tasks: Tasks<Task>,
}

impl SettingsTab for State {
	fn update(&mut self, par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		self.popup_input_profiles.update(par)?;

		for task in self.tasks.drain() {
			match task {
				Task::OpenInputProfiles => {
					input_profiles::mount_popup(
						self.frontend_tasks.clone(),
						self.globals.clone(),
						self.popup_input_profiles.clone(),
					);
				}
			}
		}

		Ok(())
	}
}

fn create_input_profiles_button(
	mp: &mut crate::tab::settings::macros::MacroParams,
	parent: WidgetID,
	tasks: Tasks<Task>,
	_popup: &PopupHolder<input_profiles::View>,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("id"), Rc::from(id.as_ref()));
	params.insert(Rc::from("translation"), Rc::from("APP_SETTINGS.INPUT_PROFILES"));
	params.insert(Rc::from("icon"), Rc::from("dashboard/controller.svg"));

	mp.parser_state
		.instantiate_template(mp.doc_params, "ButtonText", mp.layout, parent, params)?;

	let btn = mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
	btn.on_click(Rc::new({
		let tasks = tasks.clone();
		move |_common, _e| {
			tasks.push(Task::OpenInputProfiles);
			Ok(())
		}
	}));

	Ok(())
}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let tasks = Tasks::<Task>::new();
		let popup = PopupHolder::<input_profiles::View>::default();

		let c = options_category(
			par.mp,
			par.id_parent,
			"APP_SETTINGS.CONTROLS",
			"dashboard/controller.svg",
		)?;
		if par.feats.openxr {
			create_input_profiles_button(par.mp, c, tasks.clone(), &popup)?;
		}
		options_dropdown::<wlx_common::config::AltModifier>(par.mp, c, &SettingType::KeyboardMiddleClick)?;
		options_dropdown::<wlx_common::config::HandsfreePointer>(par.mp, c, &SettingType::HandsfreePointer)?;
		options_checkbox(par.mp, c, SettingType::FocusFollowsMouseMode)?;
		options_checkbox(par.mp, c, SettingType::LeftHandedMouse)?;
		options_checkbox(par.mp, c, SettingType::AllowSliding)?;
		options_checkbox(par.mp, c, SettingType::InvertScrollDirectionX)?;
		options_checkbox(par.mp, c, SettingType::InvertScrollDirectionY)?;
		options_slider_f32(par.mp, c, SettingType::ScrollSpeed, 0.1, 5.0, 0.1)?;
		options_slider_f32(par.mp, c, SettingType::LongPressDuration, 0.1, 2.0, 0.1)?;

		if par.feats.openxr {
			options_slider_f32(par.mp, c, SettingType::PointerLerpFactor, 0.1, 1.0, 0.1)?;
		}

		options_slider_i32(par.mp, c, SettingType::ClickFreezeTimeMs, 0, 500, 50)?;

		Ok(State {
			popup_input_profiles: popup,
			frontend_tasks: par.frontend_tasks.clone(),
			globals: par.mp.doc_params.globals.clone(),
			tasks,
		})
	}
}
