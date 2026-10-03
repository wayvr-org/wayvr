use wgui::layout::WidgetID;

use crate::{
	tab::settings::{
		SettingsMountParams, SettingsTab,
		macros::{MacroParams, options_category, options_diagnostics_drm_card, options_diagnostics_row},
	},
	util::is_exec_installed,
};

pub struct State {}

impl SettingsTab for State {}

#[derive(Default)]
struct LspciInfo {
	// slot: String,
	// class: String,
	vendor: String,
	device: String,
	// s_vendor: String,
}

fn lspci_info(vendor: &str /* hex */, device: &str /* hex */) -> Option<LspciInfo> {
	// example output for "lspci -vmmd 1002:747e":
	//
	// Slot:	0b:00.0
	// Class:	VGA compatible controller
	// Vendor:	Advanced Micro Devices, Inc. [AMD/ATI]
	// Device:	Navi 32 [Radeon RX 7700 XT / 7800 XT]
	// SVendor:	Sapphire Technology Limited
	// SDevice:	Device 475a
	// Rev:	c8
	// ProgIf:	00

	let mut cmd = std::process::Command::new("lspci");
	cmd.arg("-vmmd");
	cmd.arg(format!("{}:{}", vendor, device));

	let res = cmd.output().ok()?;

	if !res.status.success() {
		return None;
	}

	let mut info = LspciInfo::default();

	let stdout = str::from_utf8(&res.stdout).ok()?;
	for line in stdout.lines() {
		let Some((key, value)) = line.split_once(":") else {
			continue;
		};

		if key == "Device" {
			info.device = String::from(value.trim());
		}
	}

	Some(info)
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

fn read_sysfs_param(device_path: &str, param: &str) -> anyhow::Result<String> {
	let string = std::fs::read_to_string(format!("{}/{}", device_path, param))?;
	Ok(string.trim().into())
}

fn read_sysfs_param_int(device_path: &str, param: &str) -> anyhow::Result<u64> {
	Ok(read_sysfs_param(device_path, param)?.parse::<u64>()?)
}

fn mount_drm_card(mp: &mut MacroParams, parent: WidgetID, device_path: &str) -> anyhow::Result<()> {
	let vendor = read_sysfs_param(device_path, "vendor")?;
	let device = read_sysfs_param(device_path, "device")?;
	let vram_total = read_sysfs_param_int(device_path, "mem_info_vram_total").ok();
	let vis_vram_total = read_sysfs_param_int(device_path, "mem_info_vis_vram_total").ok();

	let name = match lspci_info(&vendor, &device) {
		Some(info) => {
			format!("{} {}", info.vendor, info.device)
		}
		None => format!("{}:{}", vendor, device),
	};

	let info = if let Some(vram_total) = vram_total
		&& let Some(vis_vram_total) = vis_vram_total
	{
		let mut s = mp.layout.state.globals.i18n().translate_and_replace(
			"APP_SETTINGS.DIAGNOSTICS.CPU_ACCESSIBLE_VRAM",
			(
				"{NUM}",
				&format!("{}/{}", vis_vram_total / 1024 / 1024, vram_total / 1024 / 1024),
			),
		);

		if vis_vram_total != vram_total {
			s += " [ALERT!]\nEnable Resizable BAR in the BIOS settings for optimal VR performance";
		}

		s
	} else {
		String::from("No info")
	};

	options_diagnostics_drm_card(mp, parent, &name, &info)?;

	Ok(())
}

fn mount_drm_cards(mp: &mut MacroParams, parent: WidgetID) -> anyhow::Result<()> {
	for file in std::fs::read_dir("/sys/class/drm/")?.flatten() {
		let file_name = file.file_name();
		let Some(file_name) = file_name.to_str() else {
			continue;
		};

		if !file_name.starts_with("renderD") {
			continue;
		}

		if let Err(e) = mount_drm_card(mp, parent, &format!("/sys/class/drm/{}/device", file_name)) {
			log::error!("failed to mount drm card: {}", e);
		}
	}

	Ok(())
}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<Self> {
		let c = options_category(
			par.mp,
			par.id_parent,
			"APP_SETTINGS.DIAGNOSTICS.TITLE",
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

		mount_drm_cards(par.mp, c)?;

		Ok(State {})
	}
}
