use std::rc::Rc;

use crate::tab::settings::{
	SettingsMountParams, SettingsTab,
	macros::{options_autostart_app, options_category},
};

pub struct State {}

impl SettingsTab for State {}

impl State {
	pub fn mount(par: SettingsMountParams, app_button_ids: &mut Vec<Rc<str>>) -> anyhow::Result<State> {
		*app_button_ids = Vec::new();

		if !par.mp.config.autostart_apps.is_empty() {
			let c = options_category(
				par.mp,
				par.parent_id,
				"APP_SETTINGS.AUTOSTART_APPS",
				"dashboard/apps.svg",
			)?;

			// todo: prevent clone
			let autostart_apps = par.mp.config.autostart_apps.clone();
			for app in autostart_apps {
				options_autostart_app(par.mp, c, &app.name, app_button_ids)?;
			}
		}
		Ok(State {})
	}
}
