use crate::tab::settings::{
	SettingType,
	macros::{MacroParams, options_category, options_checkbox, options_dropdown, options_slider_f32},
};
use wgui::layout::WidgetID;

pub fn mount(mp: &mut MacroParams, parent: WidgetID) -> anyhow::Result<()> {
	let c = options_category(mp, parent, "APP_SETTINGS.LOOK_AND_FEEL", "dashboard/palette.svg")?;
	options_dropdown::<wlx_common::locale::Language>(mp, c, &SettingType::Language)?;
	options_checkbox(mp, c, SettingType::HideUsername)?;
	options_checkbox(mp, c, SettingType::HideGrabHelp)?;
	options_slider_f32(mp, c, SettingType::UiAnimationSpeed, 0.5, 5.0, 0.1)?; // min, max, step
	options_slider_f32(mp, c, SettingType::UiGradientIntensity, 0.0, 1.0, 0.05)?; // min, max, step
	options_slider_f32(mp, c, SettingType::UiRoundMultiplier, 0.5, 5.0, 0.1)?;
	options_checkbox(mp, c, SettingType::SetsOnWatch)?;
	options_slider_f32(mp, c, SettingType::GridOpacity, 0.0, 1.0, 0.05)?; // min, max, step
	options_checkbox(mp, c, SettingType::UsePassthrough)?;
	options_checkbox(mp, c, SettingType::Clock12h)?;
	Ok(())
}
