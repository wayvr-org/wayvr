use crate::tab::settings::{
	SettingType,
	macros::{MacroParams, options_category, options_checkbox, options_dropdown},
};
use wgui::layout::WidgetID;

pub fn mount(mp: &mut MacroParams, parent: WidgetID) -> anyhow::Result<()> {
	let c = options_category(mp, parent, "APP_SETTINGS.MISC", "dashboard/blocks.svg")?;
	options_dropdown::<wlx_common::config::CaptureMethod>(mp, c, &SettingType::CaptureMethod)?;
	options_checkbox(mp, c, SettingType::XwaylandByDefault)?;
	options_checkbox(mp, c, SettingType::UprightScreenFix)?;
	options_checkbox(mp, c, SettingType::DoubleCursorFix)?;
	options_checkbox(mp, c, SettingType::ScreenRenderDown)?;
	Ok(())
}
