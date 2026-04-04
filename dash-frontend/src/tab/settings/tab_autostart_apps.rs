use std::rc::Rc;

use crate::tab::settings::macros::{MacroParams, options_autostart_app, options_category};
use wgui::layout::WidgetID;

pub fn mount(mp: &mut MacroParams, parent: WidgetID, app_button_ids: &mut Vec<Rc<str>>) -> anyhow::Result<()> {
	*app_button_ids = Vec::new();

	if !mp.config.autostart_apps.is_empty() {
		let c = options_category(mp, parent, "APP_SETTINGS.AUTOSTART_APPS", "dashboard/apps.svg")?;

		// todo: prevent clone
		let autostart_apps = mp.config.autostart_apps.clone();
		for app in autostart_apps {
			options_autostart_app(mp, c, &app.name, app_button_ids)?;
		}
	}
	Ok(())
}
