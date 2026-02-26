use wayvr_ipc::{
	packet_client::WvrProcessLaunchParams,
	packet_server::{WvrProcess, WvrProcessHandle, WvrWindow, WvrWindowHandle},
};

use crate::{
	config::GeneralConfig,
	dash_interface::{self, DashInterface, RecenterMode},
	desktop_finder::DesktopFinder,
	gen_id,
};

#[derive(Debug)]
pub struct EmuProcess {
	name: String,
}

impl EmuProcess {
	fn to(&self, handle: EmuProcessHandle) -> WvrProcess {
		WvrProcess {
			handle: WvrProcessHandle {
				generation: handle.generation,
				idx: handle.idx,
			},
			name: self.name.clone(),
			userdata: Default::default(),
		}
	}
}

#[derive(Debug)]
pub struct EmuWindow {
	visible: bool,
	process_handle: EmuProcessHandle,
}

impl EmuWindow {
	fn to(&self, handle: EmuWindowHandle) -> WvrWindow {
		WvrWindow {
			size_x: 1280, /* stub */
			size_y: 720,  /* stub */
			visible: true,
			handle: WvrWindowHandle {
				generation: handle.generation,
				idx: handle.idx,
			},
			process_handle: WvrProcessHandle {
				generation: self.process_handle.generation,
				idx: self.process_handle.idx,
			},
		}
	}
}

gen_id!(EmuWindowVec, EmuWindow, EmuWindowCell, EmuWindowHandle);

gen_id!(EmuProcessVec, EmuProcess, EmuProcessCell, EmuProcessHandle);

pub struct DashInterfaceEmulated {
	processes: EmuProcessVec,
	windows: EmuWindowVec,
	desktop_finder: DesktopFinder,
	general_config: GeneralConfig,
	monado_clients: Vec<dash_interface::MonadoClient>,
	brightness: f32,
	debug_frame_id: u64,
	debug_counter: u64,
}

impl DashInterfaceEmulated {
	pub fn new() -> Self {
		let mut processes = EmuProcessVec::new();
		let process_handle = processes.add(EmuProcess {
			name: String::from("My app"),
		});

		let mut windows = EmuWindowVec::new();
		windows.add(EmuWindow {
			process_handle,
			visible: true,
		});

		let mut desktop_finder = DesktopFinder::new();
		desktop_finder.refresh();

		// Use serde defaults
		let general_config = serde_json::from_str("{}").unwrap();

		let monado_clients = vec![
			dash_interface::MonadoClient {
				name: String::from("The Best VR Game 3000"),
				is_active: true,
				is_focused: true,
				is_io_active: true,
				is_overlay: false,
				is_primary: true,
				is_visible: true,
			},
			dash_interface::MonadoClient {
				name: String::from("Second app"),
				is_active: true,
				is_focused: false,
				is_io_active: true,
				is_overlay: false,
				is_primary: false,
				is_visible: true,
			},
			dash_interface::MonadoClient {
				name: String::from("Third app"),
				is_active: true,
				is_focused: false,
				is_io_active: true,
				is_overlay: false,
				is_primary: false,
				is_visible: true,
			},
		];

		Self {
			processes,
			windows,
			desktop_finder,
			general_config,
			monado_clients,
			brightness: 1.0,
			debug_counter: 0,
			debug_frame_id: 0,
		}
	}
}

impl Default for DashInterfaceEmulated {
	fn default() -> Self {
		Self::new()
	}
}

impl DashInterface<()> for DashInterfaceEmulated {
	fn window_list(&mut self, _: &mut ()) -> anyhow::Result<Vec<WvrWindow>> {
		Ok(self.windows.iter().map(|(handle, w)| w.to(handle)).collect())
	}

	fn window_request_close(&mut self, _: &mut (), handle: WvrWindowHandle) -> anyhow::Result<()> {
		self.windows.remove(&EmuWindowHandle {
			generation: handle.generation,
			idx: handle.idx,
		});
		Ok(())
	}

	fn process_get(&mut self, _: &mut (), handle: WvrProcessHandle) -> Option<WvrProcess> {
		let emu_handle = EmuProcessHandle::new(handle.idx, handle.generation);
		self.processes.get(&emu_handle).map(|process| process.to(emu_handle))
	}

	fn process_launch(
		&mut self,
		_: &mut (),
		_: bool,
		params: WvrProcessLaunchParams,
	) -> anyhow::Result<WvrProcessHandle> {
		let res = self.processes.add(EmuProcess { name: params.name });

		self.windows.add(EmuWindow {
			process_handle: res,
			visible: true,
		});

		Ok(WvrProcessHandle {
			generation: res.generation,
			idx: res.idx,
		})
	}

	fn process_list(&mut self, _: &mut ()) -> anyhow::Result<Vec<WvrProcess>> {
		Ok(
			self
				.processes
				.iter()
				.map(|(handle, process)| process.to(handle))
				.collect(),
		)
	}

	fn process_terminate(&mut self, _: &mut (), handle: WvrProcessHandle) -> anyhow::Result<()> {
		let mut to_remove = None;

		for (wh, w) in self.windows.iter() {
			if w.process_handle == EmuProcessHandle::new(handle.idx, handle.generation) {
				to_remove = Some(wh);
			}
		}

		if let Some(wh) = to_remove {
			self.windows.remove(&wh);
		}

		self
			.processes
			.remove(&EmuProcessHandle::new(handle.idx, handle.generation));
		Ok(())
	}

	fn window_set_visible(&mut self, _: &mut (), handle: WvrWindowHandle, visible: bool) -> anyhow::Result<()> {
		match self.windows.get_mut(&EmuWindowHandle {
			generation: handle.generation,
			idx: handle.idx,
		}) {
			Some(w) => {
				w.visible = visible;
				Ok(())
			}
			None => anyhow::bail!("Window not found"),
		}
	}

	fn recenter_playspace(&mut self, _: &mut (), _: RecenterMode) -> anyhow::Result<()> {
		// stub!
		Ok(())
	}

	fn desktop_finder<'a>(&'a mut self, _: &'a mut ()) -> &'a mut DesktopFinder {
		&mut self.desktop_finder
	}

	fn general_config<'a>(&'a mut self, _: &'a mut ()) -> &'a mut crate::config::GeneralConfig {
		&mut self.general_config
	}

	fn config_changed(&mut self, _: &mut ()) {}

	fn restart(&mut self, _data: &mut ()) {}

	fn toggle_dashboard(&mut self, _data: &mut ()) {}

	fn monado_client_list(&mut self, _data: &mut ()) -> anyhow::Result<Vec<dash_interface::MonadoClient>> {
		Ok(self.monado_clients.clone())
	}

	fn monado_client_focus(&mut self, _data: &mut (), name: &str) -> anyhow::Result<()> {
		for client in self.monado_clients.iter_mut() {
			client.is_focused = false;
			client.is_active = false;
			client.is_primary = false;
		}

		if let Some(client) = self.monado_clients.iter_mut().find(|m| m.name == name) {
			client.is_active = true;
			client.is_focused = true;
			client.is_primary = true;
		}
		Ok(())
	}

	fn monado_brightness_get(&mut self, _: &mut ()) -> Option<f32> {
		Some(self.brightness)
	}

	fn monado_brightness_set(&mut self, _: &mut (), brightness: f32) -> Option<()> {
		self.brightness = brightness;
		Some(())
	}

	fn monado_metrics_set_enabled(&mut self, _: &mut (), _enabled: bool) -> bool {
		true
	}

	fn monado_metrics_dump_session_frames(&mut self, _: &mut ()) -> Vec<dash_interface::MonadoDumpSessionFrame> {
		// simulate """""typical""""" wired hmd timing graphs
		self.debug_frame_id += 1;
		self.debug_counter ^= self.debug_frame_id * 383;
		self.debug_counter ^= 0xdeadbeefbaddcafe;
		self.debug_counter >>= 1;
		vec![dash_interface::MonadoDumpSessionFrame {
			session_id: 123,
			frame_id: self.debug_frame_id as _,
			predicted_frame_time_ns: (((self.debug_frame_id) % 15) * 1000 * 1000) as _,
			predicted_wake_up_time_ns: (((self.debug_frame_id * 2) % 16) * 1000 * 1000) as _,
			predicted_gpu_done_time_ns: (((self.debug_frame_id * 3) % 17) * 1000 * 1000) as _,
			predicted_display_time_ns: (((self.debug_frame_id * 4) % 14) * 1000 * 1000) as _,
			predicted_display_period_ns: (((self.debug_frame_id * 5) % 13) * 1000 * 1000) as _,
			display_time_ns: (((self.debug_frame_id * 7) % 12) * 1000 * 1000) as _,
			when_predicted_ns: (((self.debug_frame_id * 53) % 11) * 1000 * 1000) as _,
			when_wait_woke_ns: (((self.debug_frame_id * 23) % 10) * 1000 * 1000) as _,
			when_begin_ns: (((self.debug_frame_id * 13) % 9) * 1000 * 1000) as _,
			when_delivered_ns: (((self.debug_frame_id * 11) % 8) * 1000 * 1000) as _,
			when_gpu_done_ns: (((self.debug_frame_id * 12) % 7) * 1000 * 1000) as _,
			discarded: self.debug_frame_id.is_multiple_of(5),
		}]
	}
}
