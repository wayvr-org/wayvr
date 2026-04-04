use wayvr_ipc::{
	packet_client::WvrProcessLaunchParams,
	packet_server::{WvrProcess, WvrProcessHandle, WvrWindow, WvrWindowHandle},
};

use crate::{config::GeneralConfig, desktop_finder::DesktopFinder};

#[derive(Clone)]
pub struct MonadoClient {
	pub id: i64,
	pub name: String,
	pub is_primary: bool,
	pub is_active: bool,
	pub is_visible: bool,
	pub is_focused: bool,
	pub is_overlay: bool,
	pub is_io_active: bool,
}

// see `wayvr/src/subsystem/monado_metrics/proto.rs` for documentation
#[derive(Debug)]
pub struct MonadoDumpSessionFrame {
	pub session_id: i64,
	pub frame_id: i64,
	pub predicted_frame_time_ns: u64,
	pub predicted_wake_up_time_ns: u64,
	pub predicted_gpu_done_time_ns: u64,
	pub predicted_display_time_ns: u64,
	pub predicted_display_period_ns: u64,
	pub display_time_ns: u64,
	pub when_predicted_ns: u64,
	pub when_wait_woke_ns: u64,
	pub when_begin_ns: u64,
	pub when_delivered_ns: u64,
	pub when_gpu_done_ns: u64,
	pub discarded: bool,
}

#[derive(Clone, Copy)]
pub enum RecenterMode {
	FixFloor,
	Recenter,
	Reset,
}

pub trait DashInterface<T> {
	fn window_list(&mut self, data: &mut T) -> anyhow::Result<Vec<WvrWindow>>;
	fn window_set_visible(&mut self, data: &mut T, handle: WvrWindowHandle, visible: bool) -> anyhow::Result<()>;
	fn window_request_close(&mut self, data: &mut T, handle: WvrWindowHandle) -> anyhow::Result<()>;
	fn process_get(&mut self, data: &mut T, handle: WvrProcessHandle) -> Option<WvrProcess>;
	fn process_launch(
		&mut self,
		data: &mut T,
		auto_start: bool,
		params: WvrProcessLaunchParams,
	) -> anyhow::Result<WvrProcessHandle>;
	fn process_list(&mut self, data: &mut T) -> anyhow::Result<Vec<WvrProcess>>;
	fn process_terminate(&mut self, data: &mut T, handle: WvrProcessHandle) -> anyhow::Result<()>;
	fn monado_client_list(&mut self, data: &mut T, filtered: bool) -> anyhow::Result<Vec<MonadoClient>>;
	fn monado_client_focus(&mut self, data: &mut T, name: &str) -> anyhow::Result<()>;
	fn monado_brightness_get(&mut self, data: &mut T) -> Option<f32>;
	fn monado_brightness_set(&mut self, data: &mut T, brightness: f32) -> Option<()>;
	fn monado_metrics_set_enabled(&mut self, data: &mut T, enabled: bool) -> bool;
	fn monado_metrics_dump_session_frames(&mut self, data: &mut T) -> Vec<MonadoDumpSessionFrame>;
	fn recenter_playspace(&mut self, data: &mut T, mode: RecenterMode) -> anyhow::Result<()>;
	fn desktop_finder<'a>(&'a mut self, data: &'a mut T) -> &'a mut DesktopFinder;
	fn general_config<'a>(&'a mut self, data: &'a mut T) -> &'a mut GeneralConfig;
	fn config_changed(&mut self, data: &mut T);
	fn restart(&mut self, data: &mut T);
	fn toggle_dashboard(&mut self, data: &mut T);
}

pub type BoxDashInterface<T> = Box<dyn DashInterface<T>>;
