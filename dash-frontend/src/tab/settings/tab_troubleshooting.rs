use crate::tab::settings::{
	SettingsMountParams, SettingsTab, Task,
	macros::{options_category, options_danger_button},
};

pub struct State {}

impl SettingsTab for State {}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<Self> {
		let c = options_category(
			par.mp,
			par.id_parent,
			"APP_SETTINGS.TROUBLESHOOTING",
			"dashboard/cpu.svg",
		)?;
		options_danger_button(
			par.mp,
			c,
			"APP_SETTINGS.RESET_PLAYSPACE",
			"dashboard/recenter.svg",
			Task::ResetPlayspace,
		)?;
		options_danger_button(
			par.mp,
			c,
			"APP_SETTINGS.CLEAR_PIPEWIRE_TOKENS",
			"dashboard/display.svg",
			Task::ClearPipewireTokens,
		)?;
		options_danger_button(
			par.mp,
			c,
			"APP_SETTINGS.CLEAR_SAVED_STATE",
			"dashboard/binary.svg",
			Task::ClearSavedState,
		)?;
		options_danger_button(
			par.mp,
			c,
			"APP_SETTINGS.DELETE_ALL_CONFIGS",
			"dashboard/circle.svg",
			Task::DeleteAllConfigs,
		)?;
		options_danger_button(
			par.mp,
			c,
			"APP_SETTINGS.RESTART_SOFTWARE",
			"dashboard/refresh.svg",
			Task::RestartSoftware,
		)?;
		Ok(State {})
	}
}
