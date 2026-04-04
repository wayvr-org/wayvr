use crate::tab::settings::{
	SettingType,
	macros::{MacroParams, options_category, options_checkbox},
};
use wgui::layout::WidgetID;

pub fn mount(mp: &mut MacroParams, parent: WidgetID) -> anyhow::Result<()> {
	let c = options_category(mp, parent, "APP_SETTINGS.SKYBOX", "dashboard/globe.svg")?;
	options_checkbox(mp, c, SettingType::UseSkybox)?;
	options_checkbox(mp, c, SettingType::OpaqueBackground)?;
	Ok(())
}
