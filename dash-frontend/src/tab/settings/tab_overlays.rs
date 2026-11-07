use crate::tab::settings::{
	SettingType, SettingsMountParams, SettingsTab,
	macros::{options_category, options_checkbox, options_dropdown, options_slider_f32, options_split_horiz2},
};
use wgui::task::Tasks;

enum LocalTask {}

pub struct State {
	tasks: Tasks<LocalTask>,
}

impl SettingsTab for State {}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let tasks = Tasks::<LocalTask>::new();

		let c = options_category(par.mp, par.id_parent, "APP_SETTINGS.OVERLAYS", "@/dashboard/window.svg")?;

		options_slider_f32(par.mp, c, SettingType::DefaultCurvature, 0.0, 0.5, 0.05)?; // min, max, step
		options_slider_f32(par.mp, c, SettingType::DefaultOpacity, 0.05, 1.0, 0.05)?; // min, max, step
		options_slider_f32(par.mp, c, SettingType::DefaultOverlayScale, 0.7, 1.5, 0.05)?; // min, max, step
		options_dropdown::<wlx_common::config::DefaultPositioning>(par.mp, c, &SettingType::DefaultPositioning)?;

		{
			let (id1, id2) = options_split_horiz2(par.mp, c)?;
			options_checkbox(par.mp, id1, SettingType::InvertScrollDirectionX)?;
			options_checkbox(par.mp, id2, SettingType::InvertScrollDirectionY)?;
		}

		Ok(State { tasks })
	}
}
