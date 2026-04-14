use crate::tab::settings::{
	SettingType,
	macros::{MacroParams, options_category, options_checkbox, options_dropdown, options_slider_f32, options_slider_i32},
};
use wgui::layout::WidgetID;

pub fn mount(mp: &mut MacroParams, parent: WidgetID) -> anyhow::Result<()> {
	let c = options_category(mp, parent, "APP_SETTINGS.CONTROLS", "dashboard/controller.svg")?;
	options_dropdown::<wlx_common::config::AltModifier>(mp, c, &SettingType::KeyboardMiddleClick)?;
	options_dropdown::<wlx_common::config::HandsfreePointer>(mp, c, &SettingType::HandsfreePointer)?;
	options_checkbox(mp, c, SettingType::FocusFollowsMouseMode)?;
	options_checkbox(mp, c, SettingType::LeftHandedMouse)?;
	options_checkbox(mp, c, SettingType::AllowSliding)?;
	options_checkbox(mp, c, SettingType::InvertScrollDirectionX)?;
	options_checkbox(mp, c, SettingType::InvertScrollDirectionY)?;
	options_slider_f32(mp, c, SettingType::ScrollSpeed, 0.1, 5.0, 0.1)?;
	options_slider_f32(mp, c, SettingType::LongPressDuration, 0.1, 2.0, 0.1)?;
	options_slider_f32(mp, c, SettingType::PointerLerpFactor, 0.1, 1.0, 0.1)?;
	options_slider_f32(mp, c, SettingType::FocusedScreenDistance, 0.2, 2.0, 0.05)?;
	options_slider_f32(mp, c, SettingType::FocusedScreenScale, 1.0, 2.5, 0.05)?;
	options_slider_f32(mp, c, SettingType::FocusedScreenCurveX, 0.0, 0.8, 0.02)?;
	options_slider_f32(mp, c, SettingType::FocusedScreenAssistX, 0.0, 0.75, 0.01)?;
	options_slider_f32(mp, c, SettingType::FocusedScreenAssistY, 0.0, 0.85, 0.01)?;
	options_slider_f32(mp, c, SettingType::FocusedScreenRotateAssistX, 0.0, 0.5, 0.01)?;
	options_slider_f32(mp, c, SettingType::FocusedScreenRotateAssistY, 0.0, 0.5, 0.01)?;
	options_slider_f32(mp, c, SettingType::XrClickSensitivity, 0.1, 1.0, 0.1)?;
	options_slider_f32(mp, c, SettingType::XrClickSensitivityRelease, 0.1, 1.0, 0.1)?;
	options_slider_i32(mp, c, SettingType::ClickFreezeTimeMs, 0, 500, 50)?;
	Ok(())
}
