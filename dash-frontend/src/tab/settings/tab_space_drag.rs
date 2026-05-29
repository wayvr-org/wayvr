use crate::tab::settings::{
	SettingType, SettingsMountParams, SettingsTab,
	macros::{options_category, options_checkbox, options_slider_f32},
};

pub struct State {}

impl SettingsTab for State {}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let c = options_category(par.mp, par.id_parent, "APP_SETTINGS.SPACE_DRAG", "dashboard/drag.svg")?;
		if !par.feats.openxr || par.feats.monado {
			// monado or openvr
			options_checkbox(par.mp, c, SettingType::SpaceDragUnlocked)?;
			options_slider_f32(par.mp, c, SettingType::SpaceDragMultiplier, -10.0, 10.0, 0.5)?;
			options_slider_f32(par.mp, c, SettingType::SpaceDragGravity, 0.0, 10.0, 0.5)?;
			options_slider_f32(par.mp, c, SettingType::SpaceDragDamping, 0.1, 1.0, 0.01)?;
			options_slider_f32(par.mp, c, SettingType::SpaceDragFlingStrength, 0.0, 3.0, 0.1)?;
			options_slider_f32(par.mp, c, SettingType::SpaceDragGroundFriction, 0.0, 1.0, 0.01)?;
		}
		if par.feats.monado {
			// openvr can only ever rotate yaw
			options_checkbox(par.mp, c, SettingType::SpaceRotateUnlocked)?;
		}
		Ok(State {})
	}
}
