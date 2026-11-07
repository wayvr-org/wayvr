use crate::{
	frontend::FrontendTasks,
	tab::settings::{
		SettingType, SettingsMountParams, SettingsTab,
		macros::{options_category, options_checkbox, options_dropdown, options_slider_f32},
	},
};
use wgui::globals::WguiGlobals;

pub struct State {
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
}

impl SettingsTab for State {}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let c = options_category(
			par.mp,
			par.id_parent,
			"APP_SETTINGS.LOOK_AND_FEEL",
			"@/dashboard/palette.svg",
		)?;
		options_checkbox(par.mp, c, SettingType::HideGrabHelp)?;
		options_slider_f32(par.mp, c, SettingType::UiAnimationSpeed, 0.5, 5.0, 0.1)?; // min, max, step
		options_slider_f32(par.mp, c, SettingType::UiGradientIntensity, 0.0, 1.0, 0.05)?; // min, max, step
		options_slider_f32(par.mp, c, SettingType::UiRoundMultiplier, 0.1, 5.0, 0.1)?;
		options_checkbox(par.mp, c, SettingType::EnableWatch)?;
		options_checkbox(par.mp, c, SettingType::SetsOnWatch)?;
		Ok(State {
			frontend_tasks: par.frontend_tasks.clone(),
			globals: par.mp.doc_params.globals.clone(),
		})
	}
}
