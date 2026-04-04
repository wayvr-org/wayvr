use crate::tab::settings::{
	SettingType, SettingsMountParams, SettingsTab,
	macros::{options_category, options_checkbox, options_dropdown},
};

pub struct State {}

impl SettingsTab for State {}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let c = options_category(par.mp, par.parent_id, "APP_SETTINGS.MISC", "dashboard/blocks.svg")?;
		options_dropdown::<wlx_common::config::CaptureMethod>(par.mp, c, &SettingType::CaptureMethod)?;
		options_checkbox(par.mp, c, SettingType::XwaylandByDefault)?;
		options_checkbox(par.mp, c, SettingType::UprightScreenFix)?;
		options_checkbox(par.mp, c, SettingType::DoubleCursorFix)?;
		options_checkbox(par.mp, c, SettingType::ScreenRenderDown)?;
		Ok(State {})
	}
}
