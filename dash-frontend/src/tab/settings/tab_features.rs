use crate::tab::settings::{
	SettingType, SettingsMountParams, SettingsTab,
	macros::{options_category, options_checkbox, options_slider_f32},
};

pub struct State {}

impl SettingsTab for State {}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let c = options_category(par.mp, par.parent_id, "APP_SETTINGS.FEATURES", "dashboard/options.svg")?;
		options_checkbox(par.mp, c, SettingType::NotificationsEnabled)?;
		options_checkbox(par.mp, c, SettingType::NotificationsSoundEnabled)?;
		options_checkbox(par.mp, c, SettingType::KeyboardSoundEnabled)?;
		options_checkbox(par.mp, c, SettingType::KeyboardSwipeToTypeEnabled)?;
		options_checkbox(par.mp, c, SettingType::SpaceDragUnlocked)?;
		options_checkbox(par.mp, c, SettingType::SpaceRotateUnlocked)?;
		options_slider_f32(par.mp, c, SettingType::SpaceDragMultiplier, -10.0, 10.0, 0.5)?;
		options_checkbox(par.mp, c, SettingType::BlockGameInput)?;
		options_checkbox(par.mp, c, SettingType::BlockGameInputIgnoreWatch)?;
		options_checkbox(par.mp, c, SettingType::BlockPosesOnKbdInteraction)?;
		Ok(State {})
	}
}
