use crate::tab::settings::{
	Task,
	macros::{MacroParams, options_category, options_danger_button},
};
use wgui::layout::WidgetID;

pub fn mount(mp: &mut MacroParams, parent: WidgetID) -> anyhow::Result<()> {
	let c = options_category(mp, parent, "APP_SETTINGS.TROUBLESHOOTING", "dashboard/cpu.svg")?;
	options_danger_button(
		mp,
		c,
		"APP_SETTINGS.RESET_PLAYSPACE",
		"dashboard/recenter.svg",
		Task::ResetPlayspace,
	)?;
	options_danger_button(
		mp,
		c,
		"APP_SETTINGS.CLEAR_PIPEWIRE_TOKENS",
		"dashboard/display.svg",
		Task::ClearPipewireTokens,
	)?;
	options_danger_button(
		mp,
		c,
		"APP_SETTINGS.CLEAR_SAVED_STATE",
		"dashboard/binary.svg",
		Task::ClearSavedState,
	)?;
	options_danger_button(
		mp,
		c,
		"APP_SETTINGS.DELETE_ALL_CONFIGS",
		"dashboard/circle.svg",
		Task::DeleteAllConfigs,
	)?;
	options_danger_button(
		mp,
		c,
		"APP_SETTINGS.RESTART_SOFTWARE",
		"dashboard/refresh.svg",
		Task::RestartSoftware,
	)?;
	Ok(())
}
