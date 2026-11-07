use crate::tab::settings::{
	SettingType, SettingsMountParams, SettingsTab,
	macros::{
		options_category, options_checkbox, options_dropdown, options_range_f32, options_slider, options_split_horiz2,
	},
};

pub struct State {}

impl SettingsTab for State {}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let c = options_category(par.mp, par.id_parent, "APP_SETTINGS.OVERLAYS", "@/dashboard/window.svg")?;

		options_slider(par.mp, c, SettingType::DefaultCurvature, 0.0, 0.5, 0.05)?; // min, max, step
		options_slider(par.mp, c, SettingType::DefaultOpacity, 0.05, 1.0, 0.05)?; // min, max, step
		options_slider(par.mp, c, SettingType::DefaultOverlayScale, 0.7, 1.5, 0.05)?; // min, max, step
		options_dropdown::<wlx_common::config::DefaultPositioning>(par.mp, c, &SettingType::DefaultPositioning)?;

		{
			let (id1, id2) = options_split_horiz2(par.mp, c)?;
			options_checkbox(par.mp, id1, SettingType::InvertScrollDirectionX)?;
			options_checkbox(par.mp, id2, SettingType::InvertScrollDirectionY)?;
		}

		options_dropdown::<wlx_common::config::AltModifier>(par.mp, c, &SettingType::KeyboardMiddleClick)?;

		options_slider(par.mp, c, SettingType::ScrollSpeed, 0.1, 5.0, 0.1)?;
		options_slider(par.mp, c, SettingType::LongPressDuration, 0.1, 2.0, 0.1)?;
		options_slider(par.mp, c, SettingType::ClickFreezeTimeMs, 0, 500, 50)?;

		// Watch
		{
			let c = options_category(par.mp, c, "APP_SETTINGS.WATCH", "@/dashboard/watch.svg")?;
			options_checkbox(par.mp, c, SettingType::EnableWatch)?;
			options_checkbox(par.mp, c, SettingType::SetsOnWatch)?;
			options_range_f32(
				par.mp,
				c,
				SettingType::WatchViewAngleMin,
				SettingType::WatchViewAngleMax,
				0.1,
				1.0,
				0.1,
			)?;
		}

		Ok(State {})
	}
}
