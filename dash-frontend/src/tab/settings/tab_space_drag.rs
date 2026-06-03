use wgui::{
	assets::AssetPath,
	i18n::Translation,
	layout::{Layout, LayoutTask, WidgetID},
	parser::{Fetchable, ParseDocumentParams},
};

use crate::{
	tab::settings::{
		SettingType, SettingsMountParams, SettingsTab,
		macros::{options_category, options_checkbox, options_slider_f32},
	},
	util::wgui_simple,
};

pub struct State {
	id_space_gravity_parent: WidgetID,
}

fn set_visible(parent: WidgetID, layout: &mut Layout, n: bool) {
	layout.tasks.push(LayoutTask::SetWidgetVisible(parent, n));
}

impl SettingsTab for State {
	fn setting_updated(&mut self, sup: &mut super::SettingUpdatedParams) -> anyhow::Result<()> {
		if sup.setting_type == SettingType::SpaceGravityEnabled {
			set_visible(
				self.id_space_gravity_parent,
				sup.layout,
				sup.config.space_gravity_enabled,
			);
		}
		Ok(())
	}
}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let c = options_category(par.mp, par.id_parent, "APP_SETTINGS.SPACE_DRAG", "dashboard/drag.svg")?;

		let globals = par.mp.layout.state.globals.clone();

		let tab_state = wgui::parser::parse_from_assets(
			&ParseDocumentParams {
				globals,
				path: AssetPath::BuiltIn("gui/tab/settings_tab_space_drag.xml"),
				extra: Default::default(),
			},
			par.mp.layout,
			c,
		)?;

		let id_common_options_parent = tab_state.get_widget_id("common_options_parent")?;
		let id_gravity_enabled_parent = tab_state.get_widget_id("gravity_enabled_parent")?;
		let id_space_gravity_parent = tab_state.get_widget_id("space_gravity_parent")?;

		if !par.feats.openxr || par.feats.monado {
			// monado or openvr
			options_checkbox(par.mp, id_common_options_parent, SettingType::SpaceDragUnlocked)?;

			options_slider_f32(
				par.mp,
				id_common_options_parent,
				SettingType::SpaceDragMultiplier,
				-10.0,
				10.0,
				0.5,
			)?;
		}

		if par.feats.monado {
			// openvr can only ever rotate yaw
			options_checkbox(par.mp, id_common_options_parent, SettingType::SpaceRotateUnlocked)?;
		}

		if par.feats.monado {
			/* space gravity section */
			options_checkbox(par.mp, id_gravity_enabled_parent, SettingType::SpaceGravityEnabled)?;

			options_slider_f32(
				par.mp,
				id_space_gravity_parent,
				SettingType::SpaceGravityGravity,
				0.0,
				10.0,
				0.5,
			)?;
			options_slider_f32(
				par.mp,
				id_space_gravity_parent,
				SettingType::SpaceGravityDamping,
				0.1,
				1.0,
				0.01,
			)?;
			options_slider_f32(
				par.mp,
				id_space_gravity_parent,
				SettingType::SpaceGravityFlingStrength,
				0.0,
				3.0,
				0.1,
			)?;
			options_slider_f32(
				par.mp,
				id_space_gravity_parent,
				SettingType::SpaceGravityGroundFriction,
				0.0,
				1.0,
				0.01,
			)?;
			options_slider_f32(
				par.mp,
				id_space_gravity_parent,
				SettingType::SpaceGravityFloorHeight,
				-5.0,
				5.0,
				0.1,
			)?;
		} else {
			wgui_simple::create_label(
				par.mp.layout,
				id_gravity_enabled_parent,
				Translation::from_translation_key("APP_SETTINGS.NOT_SUPPORTED"),
			)?;
		}

		set_visible(
			id_space_gravity_parent,
			par.mp.layout,
			par.mp.config.space_gravity_enabled,
		);

		Ok(State {
			id_space_gravity_parent,
		})
	}
}
