use crate::tab::settings::{
	SettingType, SettingsMountParams, SettingsTab,
	macros::{options_category, options_checkbox, options_dropdown, options_slider},
};

pub struct State {}

impl SettingsTab for State {}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let c = options_category(par.mp, par.id_parent, "HID", "@/dashboard/mouse.svg")?;

		options_dropdown::<wlx_common::config::InputCaptureMethod>(par.mp, c, &SettingType::InputCaptureMethod)?;
		options_slider(par.mp, c, SettingType::MouseSpeed, -1., 1., 0.05)?;
		options_checkbox(par.mp, c, SettingType::MouseAcceleration)?;

		// "Handsfree" category
		{
			let c = options_category(par.mp, c, "APP_SETTINGS.HANDSFREE_POINTER", "@/dashboard/hand.svg")?;

			options_dropdown::<wlx_common::config::HandsfreePointer>(par.mp, c, &SettingType::HandsfreePointer)?;
			options_dropdown::<wlx_common::config::HandsfreeAltTab>(par.mp, c, &SettingType::HandsfreeAltTab)?;
		}

		Ok(State {})
	}
}
