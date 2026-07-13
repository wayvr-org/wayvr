use std::{
    ffi::OsStr,
    io::Read,
    os::unix::net::UnixStream,
    path::PathBuf,
    process::{Child, Command},
    sync::Arc,
};

use anyhow::Context;
use glam::Vec2;
use smithay::{
    backend::input::{Axis, AxisSource, ButtonState, Keycode},
    input::{
        keyboard::KeyboardHandle,
        pointer::{AxisFrame, ButtonEvent, MotionEvent, PointerHandle},
    },
    reexports::wayland_server::{self, protocol::wl_surface::WlSurface},
    utils::{Logical, Point, SerialCounter},
};
use wgui::log::LogErr;
use xkbcommon::xkb;

use crate::backend::wayvr::{ExternalProcessRequest, WayVRTask};

use super::{
    ProcessWayVREnv,
    comp::{self, ClientState},
    process,
};

pub struct WayVRClient {
    pub client: wayland_server::Client,
    pub pid: u32,
}

pub struct WayVRCompositor {
    pub state: comp::Application,
    pub seat_keyboard: KeyboardHandle<comp::Application>,
    pub seat_pointer: PointerHandle<comp::Application>,
    pub serial_counter: SerialCounter,
    pub wayland_env: super::WaylandEnv,

    xwayland_satellite: Option<Child>,

    display: wayland_server::Display<comp::Application>,
    listener: wayland_server::ListeningSocket,

    toplevel_surf_count: u32,     // for logging purposes
    scroll_accumulator: [f32; 2], // turn smooth scroll into discrete scroll

    pub clients: Vec<WayVRClient>,
}

impl Drop for WayVRCompositor {
    fn drop(&mut self) {
        if let Some(mut child) = self.xwayland_satellite.take() {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGKILL);
            }
            // reap the pid
            let _ = child.wait();
        }
    }
}

fn get_wayvr_env_from_pid(pid: i32) -> anyhow::Result<ProcessWayVREnv> {
    let path = format!("/proc/{pid}/environ");
    let mut env_data = String::new();
    std::fs::File::open(path)?.read_to_string(&mut env_data)?;

    let lines: Vec<&str> = env_data.split('\0').filter(|s| !s.is_empty()).collect();

    let mut env = ProcessWayVREnv {
        display_auth: None,
        display_name: None,
    };

    for line in lines {
        if let Some((key, value)) = line.split_once('=') {
            if key == "WAYVR_DISPLAY_AUTH" {
                env.display_auth = Some(String::from(value));
            } else if key == "WAYVR_DISPLAY_NAME" {
                env.display_name = Some(String::from(value));
            }
        }
    }

    Ok(env)
}

impl WayVRCompositor {
    pub fn new(
        state: comp::Application,
        display: wayland_server::Display<comp::Application>,
        seat_keyboard: KeyboardHandle<comp::Application>,
        seat_pointer: PointerHandle<comp::Application>,
    ) -> anyhow::Result<Self> {
        let (wayland_env, listener) = create_wayland_listener()?;

        let xwayland_satellite = Command::new(bundled_executable("xwayland-satellite"))
            .arg(wayland_env.display_num_string())
            .env("WAYLAND_DISPLAY", wayland_env.wayland_display_num_string())
            .spawn()
            .log_warn(
                "Could not start xwayland-satellite. Xwayland apps will not work in native mode",
            )
            .ok();

        Ok(Self {
            state,
            display,
            seat_keyboard,
            seat_pointer,
            listener,
            xwayland_satellite,
            wayland_env,
            serial_counter: SerialCounter::new(),
            clients: Vec::new(),
            toplevel_surf_count: 0,
            scroll_accumulator: [0.0, 0.0],
        })
    }

    pub fn add_client(&mut self, client: WayVRClient) {
        self.clients.push(client);
    }

    pub fn cleanup_clients(&mut self) {
        self.clients.retain(|client| {
            let Some(data) = client.client.get_data::<ClientState>() else {
                return false;
            };

            if *data.disconnected.lock().unwrap() {
                return false;
            }

            true
        });
    }

    pub fn cleanup_handles(&mut self) {
        self.state.cleanup();
    }

    fn accept_connection(
        &mut self,
        stream: UnixStream,
        processes: &mut process::ProcessVec,
    ) -> anyhow::Result<()> {
        let client = self
            .display
            .handle()
            .insert_client(stream, Arc::new(comp::ClientState::default()))
            .unwrap();

        let creds = client.get_credentials(&self.display.handle())?;

        let process_env = get_wayvr_env_from_pid(creds.pid)?;

        // Find suitable auth key from the process list
        for p in processes.vec.iter().flatten() {
            if let process::Process::Managed(process) = &p.obj
                && let Some(auth_key) = &process_env.display_auth
            {
                // Find process with matching auth key
                if process.auth_key.as_str() == auth_key {
                    // Add client
                    self.add_client(WayVRClient {
                        client,
                        pid: creds.pid as u32,
                    });
                    return Ok(());
                }
            }
        }

        // This is a new process which we didn't met before.
        // Treat external processes exclusively (spawned by the user or external program)
        log::warn!(
            "External process ID {} connected to this Wayland server",
            creds.pid
        );

        self.state
            .wayvr_tasks
            .send(WayVRTask::NewExternalProcess(ExternalProcessRequest {
                env: process_env,
                client,
                pid: creds.pid as u32,
            }));

        Ok(())
    }

    fn accept_connections(&mut self, processes: &mut process::ProcessVec) -> anyhow::Result<()> {
        if let Some(stream) = self.listener.accept()?
            && let Err(e) = self.accept_connection(stream, processes)
        {
            log::error!("Failed to accept connection: {e}");
        }

        Ok(())
    }

    pub fn tick_wayland(&mut self, processes: &mut process::ProcessVec) -> anyhow::Result<()> {
        if let Err(e) = self.accept_connections(processes) {
            log::error!("accept_connections failed: {e}");
        }

        self.display.dispatch_clients(&mut self.state)?;
        self.display.flush_clients()?;

        let surf_count = self.state.xdg_shell.toplevel_surfaces().len() as u32;
        if surf_count != self.toplevel_surf_count {
            self.toplevel_surf_count = surf_count;
            log::info!("Toplevel surface count changed: {surf_count}");
        }

        Ok(())
    }

    pub fn send_key(&mut self, virtual_key: u32, down: bool) {
        let state = if down {
            smithay::backend::input::KeyState::Pressed
        } else {
            smithay::backend::input::KeyState::Released
        };

        self.seat_keyboard.input::<(), _>(
            &mut self.state,
            Keycode::new(virtual_key),
            state,
            self.serial_counter.next_serial(),
            0,
            |_, _, _| smithay::input::keyboard::FilterResult::Forward,
        );
    }

    pub fn set_keymap(&mut self, keymap: &xkb::Keymap) -> anyhow::Result<()> {
        // Smithay only accepts keymaps in a string form due to thread safety concerns
        self.seat_keyboard
            .set_keymap_from_string(
                &mut self.state,
                keymap.get_as_string(xkb::KEYMAP_FORMAT_USE_ORIGINAL),
            )
            .context("Failed to set keymap")
    }

    pub fn send_mouse_move(&mut self, focus: Option<(WlSurface, Vec2)>, global_pos: Vec2) {
        let location: Point<f64, Logical> = (global_pos.x as f64, global_pos.y as f64).into();

        let focus = focus.map(|(surface, origin)| {
            let focus_location: Point<f64, Logical> = (origin.x as f64, origin.y as f64).into();

            (surface, focus_location)
        });

        self.seat_pointer.motion(
            &mut self.state,
            focus,
            &MotionEvent {
                location,
                serial: self.serial_counter.next_serial(),
                time: super::time::get_millis() as u32,
            },
        );

        self.seat_pointer.frame(&mut self.state);
    }

    pub fn send_pointer_button(&mut self, index: super::MouseIndex, pressed: bool) {
        let state = if pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        };

        let serial = self.serial_counter.next_serial();
        let time = super::time::get_millis() as u32;
        let button = match index {
            super::MouseIndex::Left => 0x110,
            super::MouseIndex::Center => 0x112,
            super::MouseIndex::Right => 0x111,
        };

        self.seat_pointer.button(
            &mut self.state,
            &ButtonEvent {
                serial,
                time,
                button,
                state,
            },
        );

        self.seat_pointer.frame(&mut self.state);
    }

    pub fn send_pointer_axis_wheel(&mut self, delta: super::WheelDelta) {
        let time = super::time::get_millis() as u32;

        let multiplier = 32.0;
        let delta_x = (delta.x * multiplier) as i32;
        let delta_y = (-delta.y * multiplier) as i32;

        if delta_x == 0 && delta_y == 0 {
            return;
        }

        let steps_x = accumulate_discrete_scroll(&mut self.scroll_accumulator[0], delta_x);
        let steps_y = accumulate_discrete_scroll(&mut self.scroll_accumulator[1], delta_y);

        let mut frame = AxisFrame::new(time).source(AxisSource::Wheel);

        if delta_x != 0 {
            frame = frame.value(Axis::Horizontal, delta_x as f64 / 8.0);

            if steps_x != 0 {
                frame = frame.v120(Axis::Horizontal, steps_x * 120);
            }
        }

        if delta_y != 0 {
            frame = frame.value(Axis::Vertical, delta_y as f64 / 8.0);

            if steps_y != 0 {
                frame = frame.v120(Axis::Vertical, steps_y * 120);
            }
        }

        self.seat_pointer.axis(&mut self.state, frame);
        self.seat_pointer.frame(&mut self.state);
    }
}

const STARTING_WAYLAND_ADDR_IDX: u32 = 20;

fn export_display_number(display_num: u32) -> anyhow::Result<()> {
    let mut path =
        std::env::var("XDG_RUNTIME_DIR").map_or_else(|_| PathBuf::from("/tmp"), PathBuf::from);
    path.push("wayvr.disp");
    std::fs::write(path, format!("{display_num}\n"))?;
    Ok(())
}

fn create_wayland_listener() -> anyhow::Result<(super::WaylandEnv, wayland_server::ListeningSocket)>
{
    let mut env = super::WaylandEnv {
        display_num: STARTING_WAYLAND_ADDR_IDX,
    };

    let listener = loop {
        let display_str = env.wayland_display_num_string();
        log::debug!("Trying to open socket \"{display_str}\"");
        match wayland_server::ListeningSocket::bind(display_str.as_str()) {
            Ok(listener) => {
                log::debug!("Listening to {display_str}");
                break listener;
            }
            Err(e) => {
                log::debug!(
                    "Failed to open socket \"{display_str}\" (reason: {e}), trying next..."
                );

                env.display_num += 1;
                if env.display_num > STARTING_WAYLAND_ADDR_IDX + 20 {
                    // Highly unlikely for the user to have 20 Wayland displays enabled at once. Return error instead.
                    anyhow::bail!("Failed to create wayland-server socket")
                }
            }
        }
    };

    if let Err(e) = export_display_number(env.display_num) {
        log::error!("Could not write wayvr.disp: {e:?}");
    }

    Ok((env, listener))
}

fn accumulate_discrete_scroll(acc: &mut f32, delta_v120: i32) -> i32 {
    const WHEEL_DETENT_V120: f32 = 120.0;
    *acc += delta_v120 as f32;
    let steps = (*acc / WHEEL_DETENT_V120).trunc() as i32;
    if steps != 0 {
        *acc -= steps as f32 * WHEEL_DETENT_V120;
        if acc.abs() < 0.001 {
            *acc = 0.0;
        }
    }

    steps
}

/// Runs executable from APPDIR, falling back to PATH
fn bundled_executable(name: impl AsRef<std::path::Path>) -> PathBuf {
    match std::env::var_os("APPDIR") {
        Some(appdir) => PathBuf::from(appdir).join("usr").join("bin").join(name),
        None => PathBuf::from(name.as_ref()),
    }
}
