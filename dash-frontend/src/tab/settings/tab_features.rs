use crate::tab::settings::{
	SettingType,
	macros::{MacroParams, options_category, options_checkbox, options_slider_f32},
};
use wgui::layout::WidgetID;

pub fn mount(mp: &mut MacroParams, parent: WidgetID) -> anyhow::Result<()> {
	let c = options_category(mp, parent, "APP_SETTINGS.FEATURES", "dashboard/options.svg")?;
	options_checkbox(mp, c, SettingType::NotificationsEnabled)?;
	options_checkbox(mp, c, SettingType::NotificationsSoundEnabled)?;
	options_checkbox(mp, c, SettingType::KeyboardSoundEnabled)?;
	options_checkbox(mp, c, SettingType::KeyboardSwipeToTypeEnabled)?;
	options_checkbox(mp, c, SettingType::SpaceDragUnlocked)?;
	options_checkbox(mp, c, SettingType::SpaceRotateUnlocked)?;
	options_slider_f32(mp, c, SettingType::SpaceDragMultiplier, -10.0, 10.0, 0.5)?;
	options_checkbox(mp, c, SettingType::BlockGameInput)?;
	options_checkbox(mp, c, SettingType::BlockGameInputIgnoreWatch)?;
	options_checkbox(mp, c, SettingType::BlockPosesOnKbdInteraction)?;
	Ok(())
}
