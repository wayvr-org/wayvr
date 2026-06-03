use crate::tab::settings::{
	SettingType, SettingsMountParams, SettingsTab,
	macros::{options_category, options_checkbox, options_range_f32, options_slider_f32},
};

pub struct State {}

impl SettingsTab for State {}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let c = options_category(par.mp, par.id_parent, "APP_SETTINGS.FEATURES", "dashboard/options.svg")?;
		options_checkbox(par.mp, c, SettingType::NotificationsEnabled)?;
		options_checkbox(par.mp, c, SettingType::NotificationsSoundEnabled)?;
		options_checkbox(par.mp, c, SettingType::KeyboardSoundEnabled)?;
		if !par.feats.openxr || par.feats.monado {
			// monado or openvr
			options_checkbox(par.mp, c, SettingType::BlockGameInput)?;
			options_checkbox(par.mp, c, SettingType::BlockGameInputIgnoreWatch)?;
		}
		if par.feats.monado {
			// monado-only
			options_checkbox(par.mp, c, SettingType::BlockPosesOnKbdInteraction)?;
		}

		options_range_f32(
			par.mp,
			c,
			SettingType::WatchViewAngleMin,
			SettingType::WatchViewAngleMax,
			0.1,
			1.0,
			0.1,
		)?;
		Ok(State {})
	}
}
