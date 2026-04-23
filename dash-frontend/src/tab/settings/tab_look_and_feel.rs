use crate::tab::settings::{
	SettingType, SettingsMountParams, SettingsTab,
	macros::{options_category, options_checkbox, options_dropdown, options_slider_f32},
};

pub struct State {}

impl SettingsTab for State {}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let c = options_category(
			par.mp,
			par.id_parent,
			"APP_SETTINGS.LOOK_AND_FEEL",
			"dashboard/palette.svg",
		)?;
		options_dropdown::<wlx_common::locale::Language>(par.mp, c, &SettingType::Language)?;
		options_checkbox(par.mp, c, SettingType::OpaqueBackground)?;
		options_checkbox(par.mp, c, SettingType::HideUsername)?;
		options_checkbox(par.mp, c, SettingType::HideGrabHelp)?;
		options_slider_f32(par.mp, c, SettingType::UiAnimationSpeed, 0.5, 5.0, 0.1)?; // min, max, step
		options_slider_f32(par.mp, c, SettingType::UiGradientIntensity, 0.0, 1.0, 0.05)?; // min, max, step
		options_slider_f32(par.mp, c, SettingType::UiRoundMultiplier, 0.5, 5.0, 0.1)?;
		options_checkbox(par.mp, c, SettingType::SetsOnWatch)?;
		options_checkbox(par.mp, c, SettingType::Clock12h)?;
		Ok(State {})
	}
}
