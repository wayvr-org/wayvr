use std::rc::Rc;
use wgui::i18n::Translation;
use wgui::layout::{Layout, LayoutTask};
use wgui::parser::TemplateParams;
use wgui::{components::button::ComponentButton, layout::WidgetID, parser::Fetchable, task::Tasks};
use wlx_common::XrBackend;
use wlx_common::dash_interface::DashBackendTask;

use crate::frontend::FrontendTask;
use crate::tab::settings::macros::{options_button, options_slider, options_split_horiz3};
use crate::tab::settings::{Task, UpdateExtra, horiz_cell, mount_requires_restart};
use crate::util::popup_manager::PopupHolder;

use crate::util::wgui_simple;
use crate::{
	frontend::FrontendTasks,
	tab::settings::{
		SettingType, SettingsMountParams, SettingsTab,
		macros::{options_category, options_checkbox},
	},
	views::{ViewUpdateParams, input_profiles},
};

#[derive(Clone)]
enum LocalTask {
	OpenInputProfilesDash,
	OpenInputProfilesBackend,
}

pub struct State {
	popup_input_profiles: PopupHolder<input_profiles::View>,
	frontend_tasks: FrontendTasks,
	tasks: Tasks<LocalTask>,
	id_space_gravity: Option<WidgetID>,
}

impl SettingsTab for State {
	fn setting_updated(&mut self, sup: &mut super::SettingUpdatedParams) -> anyhow::Result<()> {
		if sup.setting_type == SettingType::SpaceGravityEnabled
			&& let Some(id_space_gravity) = self.id_space_gravity
		{
			set_visible(id_space_gravity, sup.layout, sup.config.space_gravity_enabled);
		}
		Ok(())
	}
	fn update(&mut self, par: &mut ViewUpdateParams, _extra: &UpdateExtra) -> anyhow::Result<()> {
		self.popup_input_profiles.update(par)?;

		for task in self.tasks.drain() {
			match task {
				LocalTask::OpenInputProfilesDash => {
					input_profiles::mount_popup(self.frontend_tasks.clone(), self.popup_input_profiles.clone());
				}
				LocalTask::OpenInputProfilesBackend => {
					self
						.frontend_tasks
						.push(FrontendTask::BackendTask(DashBackendTask::ShowBindingsUI));

					// overlay sort order doesn't work on steamvr,
					// so the dashboard would be drawn over the bindings UI
					// while the bindings UI has input focus
					self.frontend_tasks.push(FrontendTask::HideDashboard);
				}
			}
		}

		Ok(())
	}
}

fn create_input_profiles_button(
	mp: &mut crate::tab::settings::macros::MacroParams,
	parent: WidgetID,
	tasks: Tasks<LocalTask>,
	_popup: &PopupHolder<input_profiles::View>,
	xr_backend: XrBackend,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let id_cell = horiz_cell(mp.layout, parent)?;

	let mut params = TemplateParams::new();
	params.insert("id", &id);
	params.insert("translation", "APP_SETTINGS.INPUT_PROFILES");
	params.insert("icon", "@/dashboard/controller.svg");

	mp.parser_state
		.instantiate_template(mp.doc_params, "ButtonText", mp.layout, id_cell, params)?;

	let btn = mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
	btn.on_click(Rc::new({
		let tasks = tasks.clone();
		move |_common, _e| {
			if xr_backend.is_open_vr() {
				tasks.push(LocalTask::OpenInputProfilesBackend);
			} else {
				tasks.push(LocalTask::OpenInputProfilesDash);
			}
			Ok(())
		}
	}));

	mount_requires_restart(mp.layout, id_cell)?;

	Ok(())
}

fn set_visible(parent: WidgetID, layout: &mut Layout, n: bool) {
	layout.tasks.push(LayoutTask::SetWidgetVisible(parent, n));
}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let tasks = Tasks::<LocalTask>::new();
		let popup = PopupHolder::<input_profiles::View>::default();
		let mut id_space_gravity: Option<WidgetID> = None;

		let c = options_category(
			par.mp,
			par.id_parent,
			"APP_SETTINGS.CONTROLS",
			"@/dashboard/controller.svg",
		)?;

		create_input_profiles_button(par.mp, c, tasks.clone(), &popup, par.feats.xr_backend)?;

		let monado_or_openvr = par.feats.xr_backend.is_open_vr() || par.feats.monado;

		options_slider(par.mp, c, SettingType::PointerLerpFactor, 0.1, 1.0, 0.1)?;

		if monado_or_openvr {
			// "Playspace" category
			let c = options_category(par.mp, c, "DEFINITIONS.PLAYSPACE", "@/dashboard/playspace.svg")?;

			{
				let (id1, id2, id3) = options_split_horiz3(par.mp, c)?;

				options_button(
					par.mp,
					id1,
					"APP_SETTINGS.RESET_PLAYSPACE",
					"@/dashboard/refresh.svg",
					Task::ResetPlayspace,
				)?;

				options_button(
					par.mp,
					id2,
					"APP_SETTINGS.SAVE_PLAYSPACE_CENTER",
					"@/dashboard/recenter.svg",
					Task::SavePlayspaceCenter,
				)?;

				options_button(
					par.mp,
					id3,
					"APP_SETTINGS.RESET_PLAYSPACE_CENTER",
					"@/dashboard/trash.svg",
					super::Task::ResetPlayspaceCenter,
				)?;
			}

			options_slider(par.mp, c, SettingType::SpaceDragMultiplier, -10.0, 10.0, 0.5)?;
			options_checkbox(par.mp, c, SettingType::SpaceDragAffectsWorld)?;
			options_checkbox(par.mp, c, SettingType::SpaceDragUnlocked)?;

			// openvr can only ever rotate yaw
			if par.feats.monado {
				options_checkbox(par.mp, c, SettingType::SpaceRotateUnlocked)?;
			}

			// "Gravity" category
			{
				let c = options_category(
					par.mp,
					c,
					"APP_SETTINGS.SPACE_GRAVITY_GRAVITY",
					"@/dashboard/gravity.svg",
				)?;

				if par.feats.monado {
					options_checkbox(par.mp, c, SettingType::SpaceGravityEnabled)?;

					let c = wgui_simple::create_flex_down(par.mp.layout, c)?;
					id_space_gravity = Some(c);

					/* space gravity section */
					options_slider(par.mp, c, SettingType::SpaceGravityGravity, 0.0, 10.0, 0.5)?;
					options_slider(par.mp, c, SettingType::SpaceGravityDamping, 0.1, 1.0, 0.01)?;
					options_slider(par.mp, c, SettingType::SpaceGravityFlingStrength, 0.0, 3.0, 0.1)?;
					options_slider(par.mp, c, SettingType::SpaceGravityGroundFriction, 0.0, 1.0, 0.01)?;
					options_slider(par.mp, c, SettingType::SpaceGravityFloorHeight, -5.0, 5.0, 0.1)?;

					set_visible(c, par.mp.layout, par.mp.config.space_gravity_enabled);
				} else {
					wgui_simple::create_label(
						par.mp.layout,
						c,
						Translation::from_translation_key("APP_SETTINGS.NOT_SUPPORTED"),
					)?;
				}
			}
		}

		// "Various" category
		{
			let c = options_category(par.mp, c, "APP_SETTINGS.VARIOUS", "@/dashboard/settings.svg")?;
			options_checkbox(par.mp, c, SettingType::FocusFollowsMouseMode)?;
			options_checkbox(par.mp, c, SettingType::MiddleGrabsSingle)?;
			options_checkbox(par.mp, c, SettingType::AllowSliding)?;
			options_slider(par.mp, c, SettingType::SnapAngleDeg, 0., 45., 5.)?;

			if monado_or_openvr {
				// monado or openvr
				options_checkbox(par.mp, c, SettingType::BlockGameInput)?;
				options_checkbox(par.mp, c, SettingType::BlockGameInputIgnoreWatch)?;
			}

			if par.feats.monado {
				// monado-only
				options_checkbox(par.mp, c, SettingType::BlockPosesOnKbdInteraction)?;
			}
		}

		Ok(State {
			popup_input_profiles: popup,
			frontend_tasks: par.frontend_tasks.clone(),
			tasks,
			id_space_gravity,
		})
	}
}
