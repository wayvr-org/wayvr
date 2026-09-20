use wgui::layout::WidgetID;

use crate::tab::settings::{
	SettingsMountParams, SettingsTab,
	macros::{MacroParams, options_category, options_diagnostics_row},
};

pub struct State {}

impl SettingsTab for State {}

// works good enough
fn is_exec_installed(exec: &str) -> bool {
	let mut cmd = std::process::Command::new("sh");
	cmd.arg("-c");
	cmd.arg(format!("command -v {}", exec));

	let Ok(res) = cmd.output() else {
		return false;
	};

	res.status.success()
}

fn mount_installed_path(
	mp: &mut MacroParams,
	parent: WidgetID,
	exec: &str,
	reason_translation: Option<&str>,
) -> anyhow::Result<()> {
	let title = if is_exec_installed(exec) {
		"INSTALLED"
	} else {
		"NOT_INSTALLED"
	};

	let title_translated = mp.layout.state.globals.i18n().translate(title);
	let tooltip = if let Some(reason) = reason_translation {
		format!(
			"{}\n{}",
			title_translated,
			mp.layout.state.globals.i18n().translate(reason)
		)
	} else {
		title_translated.to_string()
	};
	options_diagnostics_row(mp, parent, exec, &tooltip, is_exec_installed(exec))?;
	Ok(())
}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<Self> {
		let c = options_category(
			par.mp,
			par.id_parent,
			"APP_SETTINGS.DIAGNOSTICS",
			"@/dashboard/diagnostics.svg",
		)?;

		mount_installed_path(par.mp, c, "pactl", Some("APP_SETTINGS.DIAGNOSTICS_REASON.PACTL"))?;
		mount_installed_path(par.mp, c, "cage", Some("APP_SETTINGS.DIAGNOSTICS_REASON.CAGE"))?;
		mount_installed_path(
			par.mp,
			c,
			"xwayland-satellite",
			Some("APP_SETTINGS.DIAGNOSTICS_REASON.XWAYLAND_SATELLITE"),
		)?;
		mount_installed_path(par.mp, c, "xdg-open", Some("APP_SETTINGS.DIAGNOSTICS_REASON.XDG_OPEN"))?;
		mount_installed_path(par.mp, c, "steam", None)?;

		Ok(State {})
	}
}
