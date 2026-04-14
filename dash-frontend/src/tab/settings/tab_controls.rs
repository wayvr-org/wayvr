use crate::tab::settings::{
	SettingType, SettingsMountParams, SettingsTab,
	macros::{options_category, options_checkbox, options_dropdown, options_slider_f32, options_slider_i32},
};

pub struct State {}

impl SettingsTab for State {}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let c = options_category(
			par.mp,
			par.id_parent,
			"APP_SETTINGS.CONTROLS",
			"dashboard/controller.svg",
		)?;
		options_dropdown::<wlx_common::config::AltModifier>(par.mp, c, &SettingType::KeyboardMiddleClick)?;
		options_dropdown::<wlx_common::config::HandsfreePointer>(par.mp, c, &SettingType::HandsfreePointer)?;
		options_checkbox(par.mp, c, SettingType::FocusFollowsMouseMode)?;
		options_checkbox(par.mp, c, SettingType::LeftHandedMouse)?;
		options_checkbox(par.mp, c, SettingType::AllowSliding)?;
		options_checkbox(par.mp, c, SettingType::InvertScrollDirectionX)?;
		options_checkbox(par.mp, c, SettingType::InvertScrollDirectionY)?;
		options_slider_f32(par.mp, c, SettingType::ScrollSpeed, 0.1, 5.0, 0.1)?;
		options_slider_f32(par.mp, c, SettingType::LongPressDuration, 0.1, 2.0, 0.1)?;
		options_slider_f32(par.mp, c, SettingType::PointerLerpFactor, 0.1, 1.0, 0.1)?;
		options_slider_f32(par.mp, c, SettingType::XrClickSensitivity, 0.1, 1.0, 0.1)?;
		options_slider_f32(par.mp, c, SettingType::XrClickSensitivityRelease, 0.1, 1.0, 0.1)?;
		options_slider_i32(par.mp, c, SettingType::ClickFreezeTimeMs, 0, 500, 50)?;
		Ok(State {})
	}
}
