pub mod cached_fetcher;
pub mod networking;
pub mod openxr_bindings;
pub mod pactl_wrapper;
pub mod popup_manager;
pub mod steam_utils;
pub mod toast_manager;
pub mod wgui_simple;
pub mod whisper;

// works good enough
pub fn is_exec_installed(exec: &str) -> bool {
	let mut cmd = std::process::Command::new("sh");
	cmd.arg("-c");
	cmd.arg(format!("command -v {}", exec));

	let Ok(res) = cmd.output() else {
		return false;
	};

	res.status.success()
}
