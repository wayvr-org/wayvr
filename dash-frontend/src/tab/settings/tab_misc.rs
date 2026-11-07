use crate::tab::settings::{
	SettingType, SettingsMountParams, SettingsTab,
	macros::{options_category, options_checkbox, options_dropdown, options_slider},
};

pub struct State {}

impl SettingsTab for State {}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let c = options_category(par.mp, par.id_parent, "APP_SETTINGS.MISC", "@/dashboard/blocks.svg")?;
		options_dropdown::<wlx_common::config::InputEmulationMethod>(par.mp, c, &SettingType::InputEmulationMethod)?;
		options_dropdown::<wlx_common::config::CaptureMethod>(par.mp, c, &SettingType::CaptureMethod)?;
		options_checkbox(par.mp, c, SettingType::LeftHandedMouse)?;
		options_checkbox(par.mp, c, SettingType::XwaylandByDefault)?;
		options_checkbox(par.mp, c, SettingType::UprightScreenFix)?;
		options_checkbox(par.mp, c, SettingType::DoubleCursorFix)?;
		options_checkbox(par.mp, c, SettingType::ScreenRenderDown)?;
		options_checkbox(par.mp, c, SettingType::ShowGrabHelp)?;
		options_slider(par.mp, c, SettingType::UiAnimationSpeed, 0.5, 5.0, 0.1)?; // min, max, step
		options_slider(par.mp, c, SettingType::UiGradientIntensity, 0.0, 1.0, 0.05)?; // min, max, step
		options_slider(par.mp, c, SettingType::UiRoundMultiplier, 0.1, 5.0, 0.1)?;
		Ok(State {})
	}
}
