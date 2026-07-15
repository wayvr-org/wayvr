pub mod client;
mod comp;
mod handle;
pub mod hit_test;
mod image_importer;
mod input_capture;
pub mod process;
mod time;
pub mod window;

use anyhow::Context;
use comp::Application;
use glam::{DVec2, Vec2};
use process::ProcessVec;
use slotmap::SecondaryMap;
use smallvec::SmallVec;
use smithay::{
    desktop::PopupManager,
    input::{SeatState, keyboard::XkbConfig, pointer::CursorImageStatus},
    output::{Mode, Output},
    reexports::{
        wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration_manager as kde_decoration,
        wayland_server::{
            self,
            backend::ClientId,
            protocol::{wl_buffer, wl_surface::WlSurface},
        },
    },
    utils::{Logical, Size},
    wayland::{
        compositor::{self, SurfaceData, with_states},
        dmabuf::{DmabufFeedbackBuilder, DmabufState},
        relative_pointer::RelativePointerManagerState,
        selection::{
            data_device::DataDeviceState, ext_data_control as selection_ext,
            primary_selection::PrimarySelectionState, wlr_data_control as selection_wlr,
        },
        shell::{
            kde::decoration::KdeDecorationState,
            xdg::{
                SurfaceCachedState, ToplevelSurface, XdgShellState, XdgToplevelSurfaceData,
                decoration::XdgDecorationState,
            },
        },
        shm::ShmState,
        viewporter::ViewporterState,
    },
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use vulkano::image::view::ImageView;
use wayland_protocols::xdg::shell::server::xdg_toplevel;
use wayvr_ipc::{packet_client::PositionMode, packet_server};
use wgui::{gfx::WGfx, log::LogErr};
use wlx_capture::frame::Transform;
use wlx_common::{config::GeneralConfig, desktop_finder::DesktopFinder};
use xkbcommon::xkb;

use crate::{
    backend::{
        task::{OverlayTask, SpawnPos, TaskContainer, TaskType, ToggleMode},
        wayvr::{
            image_importer::ImageImporter,
            input_capture::InputCapture,
            process::{KillSignal, Process},
        },
    },
    graphics::{ExtentExt, WGfxExtras},
    ipc::{event_queue::SyncEventQueue, ipc_server, signal::WayVRSignal},
    overlays::wayvr::{WvrCommand, create_wl_window_overlay},
    state::AppState,
    subsystem::{
        dbus::DbusConnector,
        hid::{MODS_TO_KEYS, WheelDelta},
    },
    windowing::{OverlayID, OverlaySelector, backend::OverlayEventData},
};

pub use hit_test::{WvrHitContext, WvrHitTarget, build_hit_context};

#[derive(Debug, Clone)]
pub struct WaylandEnv {
    pub display_num: u32,
}

impl WaylandEnv {
    pub fn wayland_display_num_string(&self) -> String {
        // e.g. "wayland-20"
        format!("wayland-{}", self.display_num)
    }
    pub fn display_num_string(&self) -> String {
        // e.g. ":20"
        format!(":{}", self.display_num)
    }
}

#[derive(Clone)]
pub struct ProcessWayVREnv {
    pub display_auth: Option<String>,
    pub display_name: Option<String>, // Externally spawned process by a user script
}

#[derive(Clone)]
pub struct ExternalProcessRequest {
    #[allow(dead_code)]
    pub env: ProcessWayVREnv,
    pub client: wayland_server::Client,
    pub pid: u32,
}

#[derive(Clone)]
pub enum WayVRTask {
    NewToplevel(ClientId, ToplevelSurface),
    DropToplevel(ClientId, ToplevelSurface),
    MinimizeRequest(ClientId, ToplevelSurface),
    TitleChange(ClientId, ToplevelSurface),
    NewExternalProcess(ExternalProcessRequest),
    ProcessTerminationRequest(process::ProcessHandle, KillSignal),
    CloseWindowRequest(window::WindowHandle),
}

pub struct WvrServerState {
    pub manager: client::WayVRCompositor,
    pub wm: window::WindowManager,
    pub processes: process::ProcessVec,
    pub tasks: SyncEventQueue<WayVRTask>,
    ticks: u64,
    cur_modifiers: u8,
    signals: SyncEventQueue<WayVRSignal>,
    mouse_freeze: Instant,
    window_to_overlay: HashMap<window::WindowHandle, OverlayID>,
    overlay_to_window: SecondaryMap<OverlayID, window::WindowHandle>,
    process_overlays: HashMap<process::ProcessHandle, Vec<OverlayID>>,
    input_capture: Option<InputCapture>,
    has_input_focus: bool,
    grab_toast_sent: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum MouseIndex {
    Left,
    Center,
    Right,
}

pub enum PointerFocusTarget {
    Surface { surface: WlSurface, origin: Vec2 },
    Toplevel,
    None,
}

pub enum TickTask {
    NewExternalProcess(ExternalProcessRequest), // Call WayVRCompositor::add_client after receiving this message
}

const KEY_REPEAT_DELAY: i32 = 200;
const KEY_REPEAT_RATE: i32 = 50;
const WAYVR_SCREEN_RES: [i32; 2] = [2560, 1440];

impl WvrServerState {
    pub fn new(
        gfx: Arc<WGfx>,
        gfx_extras: &WGfxExtras,
        signals: SyncEventQueue<WayVRSignal>,
    ) -> anyhow::Result<Self> {
        const fn filter_allow_any(_: &wayland_server::Client) -> bool {
            true
        }
        log::info!("Initializing WayVR server");
        let display: wayland_server::Display<Application> = wayland_server::Display::new()?;
        let dh = display.handle();
        let compositor = compositor::CompositorState::new::<Application>(&dh);
        let xdg_shell = XdgShellState::new::<Application>(&dh);
        let mut seat_state = SeatState::new();
        let shm = ShmState::new::<Application>(&dh, Vec::new());
        let data_device = DataDeviceState::new::<Application>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Application>(&dh);
        let mut seat = seat_state.new_wl_seat(&dh, "wayvr");

        let ext_data_control_state = selection_ext::DataControlState::new::<Application, _>(
            &dh,
            Some(&primary_selection_state),
            filter_allow_any,
        );
        let wlr_data_control_state = selection_wlr::DataControlState::new::<Application, _>(
            &dh,
            Some(&primary_selection_state),
            filter_allow_any,
        );

        let xdg_decoration_state = XdgDecorationState::new::<Application>(&dh);
        let kde_decoration_state =
            KdeDecorationState::new::<Application>(&dh, kde_decoration::Mode::Server);
        let relative_pointer_state = RelativePointerManagerState::new::<Application>(&dh);
        let viewporter = ViewporterState::new::<Application>(&dh);

        let dummy_milli_hz = 60000; /* refresh rate in millihertz */

        let output = Output::new(
            String::from("wayvr_display"),
            smithay::output::PhysicalProperties {
                size: (530, 300).into(), //physical size in millimeters
                subpixel: smithay::output::Subpixel::None,
                make: String::from("Completely Legit"),
                model: String::from("Virtual WayVR Display"),
            },
        );

        let mode = Mode {
            refresh: dummy_milli_hz,
            size: (WAYVR_SCREEN_RES[0], WAYVR_SCREEN_RES[1]).into(), //logical size in pixels
        };

        let _global = output.create_global::<Application>(&dh);
        output.change_current_state(Some(mode), None, None, None);
        output.set_preferred(mode);

        let main_device = {
            let (major, minor) = gfx_extras.drm_device.as_ref().context("No DRM device!")?;
            libc::makedev(*major as _, *minor as _)
        };

        // this will throw a compile-time error if smithay's drm-fourcc is out of sync with wlx-capture's
        let mut formats: Vec<smithay::backend::allocator::Format> = vec![];

        for f in &*gfx_extras.drm_formats {
            formats.push(*f);
        }

        let dmabuf_state = DmabufFeedbackBuilder::new(main_device, formats.clone())
            .build()
            .map_or_else(
                |_| {
                    log::info!("Falling back to zwp_linux_dmabuf_v1 version 3.");
                    let mut dmabuf_state = DmabufState::new();
                    let dmabuf_global =
                        dmabuf_state.create_global::<Application>(&display.handle(), formats);
                    (dmabuf_state, dmabuf_global, None)
                },
                |default_feedback| {
                    let mut dmabuf_state = DmabufState::new();
                    let dmabuf_global = dmabuf_state
                        .create_global_with_default_feedback::<Application>(
                            &display.handle(),
                            &default_feedback,
                        );
                    (dmabuf_state, dmabuf_global, Some(default_feedback))
                },
            );

        let seat_keyboard =
            seat.add_keyboard(XkbConfig::default(), KEY_REPEAT_DELAY, KEY_REPEAT_RATE)?;
        let seat_pointer = seat.add_pointer();

        let tasks = SyncEventQueue::new();

        let dma_importer = ImageImporter::new(gfx);

        let input_capture = InputCapture::new()
            .log_err("Could not initialize evdev input capture")
            .ok();

        let state = Application {
            output,
            image_importer: dma_importer,
            display_handle: dh,
            compositor,
            xdg_shell,
            seat,
            seat_state,
            shm,
            data_device,
            primary_selection_state,
            wlr_data_control_state,
            ext_data_control_state,
            xdg_decoration_state,
            kde_decoration_state,
            relative_pointer_state,
            wayvr_tasks: tasks.clone(),
            dmabuf_state,
            popup_manager: PopupManager::default(),
            viewporter,
            redraw_requests: HashSet::new(),
            pending_frame_callbacks: HashMap::new(),
            cursor_image: CursorImageStatus::default_named(),
        };

        Ok(Self {
            manager: client::WayVRCompositor::new(state, display, seat_keyboard, seat_pointer)?,
            processes: ProcessVec::new(),
            wm: window::WindowManager::new(),
            ticks: 0,
            tasks,
            cur_modifiers: 0,
            signals,
            mouse_freeze: Instant::now(),
            window_to_overlay: HashMap::new(),
            overlay_to_window: SecondaryMap::new(),
            process_overlays: HashMap::new(),
            input_capture,
            has_input_focus: false,
            grab_toast_sent: false,
        })
    }

    #[allow(clippy::too_many_lines)]
    pub fn tick_events(app: &mut AppState) -> anyhow::Result<Vec<TickTask>> {
        let mut tasks: Vec<TickTask> = Vec::new();

        let Some(wvr_server) = app.wvr_server.as_mut() else {
            return Ok(tasks);
        };

        app.ipc_server.tick(&mut ipc_server::TickParams {
            wvr_server,
            input_state: &app.input_state,
            tasks: &mut tasks,
            signals: &app.wayvr_signals,
        });

        // Tick all child processes
        let mut to_remove: SmallVec<[process::ProcessHandle; 2]> = SmallVec::new();

        for (handle, process) in wvr_server.processes.iter_mut() {
            if !process.is_running() {
                to_remove.push(handle);
            }
        }

        for p_handle in &to_remove {
            wvr_server.processes.remove(p_handle);
            wvr_server.process_removed(&mut app.tasks, *p_handle);
        }

        if !to_remove.is_empty() {
            app.wayvr_signals.send(WayVRSignal::BroadcastStateChanged(
                packet_server::WvrStateChanged::ProcessRemoved,
            ));
        }

        while let Some(task) = wvr_server.tasks.read() {
            match task {
                WayVRTask::NewExternalProcess(req) => {
                    tasks.push(TickTask::NewExternalProcess(req));
                }
                WayVRTask::NewToplevel(client_id, toplevel) => {
                    let toplevel = Rc::new(toplevel);

                    // Attach newly created toplevel surfaces to displays
                    for client in &wvr_server.manager.clients {
                        if client.client.id() != client_id {
                            continue;
                        }

                        let Some(process_handle) =
                            process::find_by_pid(&wvr_server.processes, client.pid)
                        else {
                            log::error!(
                                "WayVR window creation failed: Unexpected process ID {}. It wasn't registered before.",
                                client.pid
                            );
                            continue;
                        };

                        let output_bounds = wvr_server.manager.state.output_logical_size();

                        let (min_size, max_size) = with_states(toplevel.wl_surface(), |state| {
                            let mut guard = state.cached_state.get::<SurfaceCachedState>();
                            let current = guard.current();

                            let mut min_size = current.min_size;
                            let mut max_size = current.max_size;

                            if min_size.is_empty() {
                                min_size = Size::new(1, 1);
                            }

                            if max_size.is_empty() {
                                max_size = output_bounds;
                            } else {
                                max_size = max_size.clamp(Size::new(1, 1), output_bounds);
                            }

                            (min_size, max_size)
                        });

                        // Size, icon & fallback title comes from process
                        let (fallback_size, pos, fallback_title, icon, is_cage) =
                            match wvr_server.processes.get(&process_handle) {
                                Some(Process::Managed(p)) => {
                                    let size: Size<i32, Logical> =
                                        Size::new(p.resolution[0] as _, p.resolution[1] as _);

                                    (
                                        size.clamp(min_size, max_size),
                                        p.pos_mode,
                                        Some(p.app_name.clone()),
                                        p.icon.clone(),
                                        p.exec_path.ends_with("cage"),
                                    )
                                }
                                _ => (
                                    Size::new(1920, 1080).clamp(min_size, max_size),
                                    PositionMode::Float,
                                    None,
                                    None,
                                    false,
                                ),
                            };

                        let window_handle = wvr_server.wm.create_window(
                            toplevel.clone(),
                            process_handle,
                            output_bounds,
                            min_size,
                            max_size,
                            fallback_size.w as _,
                            fallback_size.h as _,
                        );

                        toplevel.with_pending_state(|state| {
                            state.bounds = Some(output_bounds);
                            // suggest an initial size but let the app request a different one
                            state.size = Some(fallback_size);
                            state.states.set(xdg_toplevel::State::Activated);
                        });
                        toplevel.send_configure();

                        let mut title: Arc<str> = fallback_title
                            .unwrap_or_else(|| format!("P{}", client.pid))
                            .into();

                        let spawn_pos = wvr_server
                            .last_process_overlay(process_handle)
                            .map_or(SpawnPos::Spread, |oid| {
                                SpawnPos::Parent(OverlaySelector::Id(oid))
                            });

                        let mut icon = icon;

                        // Try to get title from xdg_toplevel, unless it's running in cage
                        if !is_cage {
                            let mut needs_title = true;
                            let (xdg_title, app_id): (Option<String>, Option<String>) =
                                with_states(toplevel.wl_surface(), |states| {
                                    states.data_map.get::<XdgToplevelSurfaceData>().map_or(
                                        (None, None),
                                        |t| {
                                            let t = t.lock().unwrap();
                                            (t.title.clone(), t.app_id.clone())
                                        },
                                    )
                                });
                            if let Some(xdg_title) = xdg_title {
                                needs_title = false;
                                title = xdg_title.into();
                            }

                            // Try to get title & icon from desktop entry
                            if let Some(app_id) = app_id
                                && let Some(desktop_entry) =
                                    app.desktop_finder.get_cached_entry(&app_id)
                            {
                                if needs_title {
                                    title = desktop_entry.app_name.as_ref().into();
                                }
                                if icon.is_none()
                                    && let Some(icon_path) = desktop_entry.icon_path.as_ref()
                                {
                                    icon = Some(icon_path.as_ref().into());
                                }
                            }
                        }

                        // Fall back to identicon
                        let icon = match icon {
                            Some(icon) => icon,
                            None => DesktopFinder::create_icon(&title)?.into(),
                        };

                        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Spawn(
                            OverlaySelector::Nothing,
                            spawn_pos,
                            Box::new(move |app: &mut AppState| {
                                create_wl_window_overlay(
                                    title,
                                    app,
                                    window_handle,
                                    icon,
                                    [fallback_size.w as _, fallback_size.h as _],
                                    pos,
                                )
                                .context("Could not create WvrWindow overlay")
                                .inspect_err(|e| log::warn!("{e:?}"))
                                .ok()
                            }),
                        )));

                        app.wayvr_signals.send(WayVRSignal::BroadcastStateChanged(
                            packet_server::WvrStateChanged::WindowCreated,
                        ));
                    }
                }
                WayVRTask::DropToplevel(client_id, toplevel) => {
                    for client in &wvr_server.manager.clients {
                        if client.client.id() != client_id {
                            continue;
                        }

                        let Some(window_handle) = wvr_server.wm.find_window_handle(&toplevel)
                        else {
                            log::warn!("DropToplevel: Couldn't find matching window handle");
                            continue;
                        };

                        let process_handle = wvr_server
                            .wm
                            .windows
                            .get(&window_handle)
                            .map(|window| window.process);

                        if let Some(oid) = wvr_server.window_to_overlay.remove(&window_handle) {
                            app.tasks.enqueue(TaskType::Overlay(OverlayTask::Drop(
                                OverlaySelector::Id(oid),
                            )));
                            wvr_server.overlay_to_window.remove(oid);

                            if let Some(process_handle) = process_handle.as_ref() {
                                let mut empty = false;
                                if let Some(overlays) =
                                    wvr_server.process_overlays.get_mut(process_handle)
                                {
                                    overlays.retain(|other| *other != oid);
                                    empty = overlays.is_empty();
                                }

                                if empty {
                                    wvr_server.process_overlays.remove(process_handle);
                                }
                            }
                        }

                        wvr_server.wm.remove_window(window_handle);
                    }
                }
                WayVRTask::MinimizeRequest(client_id, toplevel) => {
                    for client in &wvr_server.manager.clients {
                        if client.client.id() != client_id {
                            continue;
                        }

                        let Some(window_handle) = wvr_server.wm.find_window_handle(&toplevel)
                        else {
                            log::warn!("MinimizeRequest: Couldn't find matching window handle");
                            continue;
                        };

                        if let Some(oid) = wvr_server.window_to_overlay.get(&window_handle) {
                            app.tasks
                                .enqueue(TaskType::Overlay(OverlayTask::ToggleOverlay(
                                    OverlaySelector::Id(*oid),
                                    ToggleMode::EnsureOff,
                                )));
                        }
                    }
                }
                WayVRTask::TitleChange(client_id, toplevel) => {
                    for client in &wvr_server.manager.clients {
                        if client.client.id() != client_id {
                            continue;
                        }
                        let Some(window_handle) = wvr_server.wm.find_window_handle(&toplevel)
                        else {
                            log::warn!("MinimizeRequest: Couldn't find matching window handle");
                            continue;
                        };
                        if let Some(oid) = wvr_server.window_to_overlay.get(&window_handle) {
                            app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
                                OverlaySelector::Id(*oid),
                                Box::new(|app, owc| {
                                    let _ = owc.backend.notify(
                                        app,
                                        OverlayEventData::WvrCommand(WvrCommand::ReloadTitle),
                                    );
                                }),
                            )));
                        }
                    }
                }
                WayVRTask::ProcessTerminationRequest(process_handle, signal) => {
                    if let Some(process) = wvr_server.processes.get_mut(&process_handle) {
                        process.kill(signal);
                    }

                    // Don't clean up all windows in case of SIGTERM,
                    // the app might display a confirmation dialog, etc.
                    if !matches!(signal, KillSignal::Kill) {
                        continue;
                    }

                    for (h, w) in wvr_server.wm.windows.iter() {
                        if w.process != process_handle {
                            continue;
                        }

                        let Some(oid) = wvr_server.window_to_overlay.get(&h) else {
                            continue;
                        };
                        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Drop(
                            OverlaySelector::Id(*oid),
                        )));
                    }
                }
                WayVRTask::CloseWindowRequest(window_handle) => {
                    if let Some(w) = wvr_server.wm.windows.get(&window_handle) {
                        log::info!("Sending window close to {window_handle:?}");
                        w.toplevel.send_close();
                    } else {
                        log::warn!(
                            "Could not close window - no such handle found: {window_handle:?}"
                        );
                    }
                }
            }
        }

        wvr_server.manager.tick_wayland(&mut wvr_server.processes)?;

        if wvr_server.ticks.is_multiple_of(200) {
            wvr_server.manager.cleanup_clients();
            wvr_server.manager.cleanup_handles();
        }

        wvr_server.process_input_capture();

        wvr_server.ticks += 1;

        Ok(tasks)
    }

    pub fn config_changed(&mut self, config: &GeneralConfig) {
        if let Some(cap) = self.input_capture.as_mut() {
            let _ = cap
                .set_pointer_accel(config.wvr_mouse_acceleration, config.wvr_mouse_speed)
                .log_err("Could not set mouse accel/speed");
        }
    }

    pub fn terminate_process(
        &mut self,
        process_handle: process::ProcessHandle,
        signal: KillSignal,
    ) {
        self.tasks
            .send(WayVRTask::ProcessTerminationRequest(process_handle, signal));
    }

    pub fn close_window(&mut self, window_handle: window::WindowHandle) {
        self.tasks
            .send(WayVRTask::CloseWindowRequest(window_handle));
    }

    pub fn overlay_added(&mut self, oid: OverlayID, window: window::WindowHandle) {
        self.overlay_to_window.insert(oid, window);
        self.window_to_overlay.insert(window, oid);

        if let Some(process_handle) = self.wm.windows.get(&window).map(|window| window.process) {
            let overlays = self.process_overlays.entry(process_handle).or_default();
            overlays.retain(|other| *other != oid);
            overlays.push(oid);
        }
    }

    fn last_process_overlay(&self, process_handle: process::ProcessHandle) -> Option<OverlayID> {
        self.process_overlays
            .get(&process_handle)
            .and_then(|overlays| overlays.last())
            .copied()
    }

    pub fn process_removed(&mut self, tasks: &mut TaskContainer, process: process::ProcessHandle) {
        let mut to_remove = vec![];

        for (hnd, win) in self.wm.windows.iter() {
            if win.process != process {
                continue;
            }

            if let Some(oid) = self.window_to_overlay.get(&hnd).copied() {
                tasks.enqueue(TaskType::Overlay(OverlayTask::Drop(OverlaySelector::Id(
                    oid,
                ))));

                self.overlay_to_window.remove(oid);
                self.window_to_overlay.remove(&hnd);
            }

            to_remove.push(hnd);
        }

        for hnd in &to_remove {
            self.wm.windows.remove(hnd);
        }

        self.process_overlays.remove(&process);
    }

    pub fn get_overlay_id(&self, window: window::WindowHandle) -> Option<OverlayID> {
        self.window_to_overlay.get(&window).copied()
    }

    pub fn hit_target_to_focus(
        &self,
        target: WvrHitTarget,
        hover_window: window::WindowHandle,
        _default_pos: Vec2,
    ) -> PointerFocusTarget {
        match target {
            WvrHitTarget::Panel(_) => PointerFocusTarget::None,
            WvrHitTarget::Toplevel { .. } => self
                .wm
                .windows
                .get(&hover_window)
                .map(|w| {
                    let surface = w.toplevel.wl_surface().clone();
                    PointerFocusTarget::Surface {
                        surface,
                        origin: glam::Vec2::ZERO,
                    }
                })
                .unwrap_or(PointerFocusTarget::Toplevel),
            WvrHitTarget::Surface {
                surface, origin, ..
            } => PointerFocusTarget::Surface { surface, origin },
            WvrHitTarget::Popup {
                surface, origin, ..
            } => PointerFocusTarget::Surface { surface, origin },
        }
    }

    pub fn pointer_is_grabbed(&self) -> bool {
        self.manager.seat_pointer.is_grabbed()
    }

    pub fn process_input_capture(&mut self) {
        if !self.has_input_focus {
            return;
        }
        let Some(input_capture) = self.input_capture.as_mut() else {
            return;
        };

        let mut mouse_delta = DVec2::ZERO;
        let mut mouse_delta_raw = DVec2::ZERO;

        for ev in input_capture.drain_events() {
            match ev {
                input_capture::CapturedEvent::Key { code, pressed } => {
                    self.manager.send_key((code as u32) + 8, pressed);
                }
                input_capture::CapturedEvent::PointerButton { button, pressed } => {
                    let Some(mouse_index) = Self::button_to_mouse_index(button) else {
                        continue;
                    };
                    self.manager.send_pointer_button(mouse_index, pressed);
                }
                input_capture::CapturedEvent::PointerMotion {
                    dx,
                    dy,
                    dx_raw,
                    dy_raw,
                } => {
                    mouse_delta += DVec2 { x: dx, y: dy };
                    mouse_delta_raw += DVec2 {
                        x: dx_raw,
                        y: dy_raw,
                    };
                }
                input_capture::CapturedEvent::PointerAxis {
                    horizontal_v120,
                    vertical_v120,
                } => {
                    let delta = WheelDelta {
                        x: horizontal_v120 as f32,
                        y: vertical_v120 as f32,
                    };
                    self.manager.send_pointer_axis_wheel_raw(delta);
                }
                input_capture::CapturedEvent::UngrabbedAll => {
                    self.manager.release_all_keys();
                    self.has_input_focus = false;
                    self.wm.keyboard_focus = None;
                }
                input_capture::CapturedEvent::KeyCombo { combo, pressed } => match combo {
                    input_capture::KeyCombo::AltF4 if pressed => {
                        if let Some(hover) = self.wm.mouse.as_mut() {
                            let window_handle = hover.hover_window;
                            self.close_window(window_handle);
                        }
                    }
                    input_capture::KeyCombo::AltTab => self.alt_tab(),
                    _ => {}
                },
            }
        }

        if mouse_delta.length_squared() > 1e-6 {
            'mouse_update: {
                let Some(ref hover) = self.wm.mouse else {
                    break 'mouse_update;
                };

                let Some(window) = self.wm.windows.get(&hover.hover_window) else {
                    break 'mouse_update;
                };
                let toplevel = window.toplevel.wl_surface().clone();
                let inner_extent = with_states(&toplevel, |states| {
                    SurfaceBufWithImage::get_from_surface(states)
                        .map(|s| s.image.extent_u32arr())
                        .unwrap_or([1, 1])
                });
                let Some(hit_ctx) =
                    build_hit_context(&toplevel, &self.manager.state.popup_manager, inner_extent)
                else {
                    break 'mouse_update;
                };

                let new_x = (hover.pos.x + mouse_delta.x).clamp(0., inner_extent[0] as f64);
                let new_y = (hover.pos.y + mouse_delta.y).clamp(0., inner_extent[1] as f64);
                let new_pos = DVec2::new(new_x, new_y);
                let new_pos_f32 = Vec2::new(new_x as f32, new_y as f32);

                let target = hit_ctx.hit_target_at(new_pos_f32);
                let focus_target = self.hit_target_to_focus(
                    target.unwrap_or(WvrHitTarget::Toplevel { pos: new_pos_f32 }),
                    hover.hover_window,
                    new_pos_f32,
                );
                self.send_mouse_move_relative(
                    focus_target,
                    new_pos,
                    mouse_delta,
                    mouse_delta_raw,
                    hover.hover_window,
                );
            }
        }
    }

    fn alt_tab(&mut self) {
        let mut windows: Vec<_> = self.wm.windows.iter().collect();
        if windows.is_empty() {
            return;
        }

        //TODO
    }

    fn button_to_mouse_index(button: u32) -> Option<MouseIndex> {
        match button {
            272 => Some(MouseIndex::Left),
            273 => Some(MouseIndex::Right),
            274 => Some(MouseIndex::Center),
            _ => None,
        }
    }

    pub fn set_input_focus(&mut self, has_focus: bool) {
        let Some(input_capture) = self.input_capture.as_mut() else {
            return;
        };

        let res = input_capture
            .set_grabbed(has_focus)
            .log_err("Could not grab input.");

        if res.is_ok() && !self.grab_toast_sent {
            self.grab_toast_sent = true;
            //TODO: localize
            let _ = DbusConnector::notify_send(
                "WayVR has your keyboard and mouse!",
                "Ctrl+Alt+Del to release",
                1,
                5000,
                0,
                true,
            );
        }

        self.has_input_focus = has_focus;
        if !has_focus {
            self.wm.mouse = None;
        }
    }

    pub fn get_focused_window(&self) -> Option<window::WindowHandle> {
        if !self.has_input_focus {
            return None;
        }
        self.wm.keyboard_focus.clone()
    }

    fn get_mouse_focus(
        &mut self,
        target: PointerFocusTarget,
        hover_window: window::WindowHandle,
        pressed: bool,
    ) -> (Option<(WlSurface, Vec2)>, Option<WlSurface>) {
        match target {
            PointerFocusTarget::Surface { surface, origin } => (Some((surface, origin)), None),
            PointerFocusTarget::Toplevel => {
                let surface = self
                    .wm
                    .windows
                    .get(&hover_window)
                    .map(|x| x.toplevel.wl_surface().clone());

                (
                    surface.clone().map(|surface| (surface, Vec2::ZERO)),
                    pressed.then_some(surface).flatten(),
                )
            }
            PointerFocusTarget::None => {
                let surface = self
                    .wm
                    .windows
                    .get(&hover_window)
                    .map(|x| x.toplevel.wl_surface().clone());
                (None, pressed.then_some(surface).flatten())
            }
        }
    }

    fn get_mouse_relative(&self, global_pos: DVec2, hover_window: window::WindowHandle) -> DVec2 {
        let Some(ref mouse) = self.wm.mouse else {
            return DVec2::ZERO;
        };
        if hover_window != mouse.hover_window {
            // don't twitch if we just switched windows
            return DVec2::ZERO;
        }
        global_pos - mouse.pos
    }

    fn send_mouse_move_relative(
        &mut self,
        target: PointerFocusTarget,
        global_pos: DVec2,
        delta: DVec2,
        delta_unaccel: DVec2,
        hover_window: window::WindowHandle,
    ) {
        let (focus, _) = self.get_mouse_focus(target, hover_window, false);

        self.manager
            .send_mouse_move(focus, global_pos, delta, delta_unaccel);
        self.mouse_freeze = Instant::now() + Duration::from_millis(1);
        self.wm.mouse = Some(window::MouseState {
            hover_window,
            pos: global_pos,
        });
    }

    pub fn send_mouse_move(
        &mut self,
        target: PointerFocusTarget,
        global_pos: Vec2,
        hover_window: window::WindowHandle,
    ) {
        if self.mouse_freeze > Instant::now() {
            return;
        }

        let global_pos = DVec2::from(global_pos);

        let (focus, _) = self.get_mouse_focus(target, hover_window, false);
        let linear_delta = self.get_mouse_relative(global_pos, hover_window);
        self.manager
            .send_mouse_move(focus, global_pos, linear_delta, linear_delta);
        self.mouse_freeze = Instant::now() + Duration::from_millis(1);
        self.wm.mouse = Some(window::MouseState {
            hover_window,
            pos: global_pos,
        });
    }

    pub fn send_mouse_button(
        &mut self,
        target: PointerFocusTarget,
        global_pos: Vec2,
        hover_window: window::WindowHandle,
        index: MouseIndex,
        pressed: bool,
        click_freeze: i32,
    ) {
        if pressed {
            self.mouse_freeze = Instant::now() + Duration::from_millis(click_freeze.max(0) as u64);
        }

        let (focus, focus_keyboard) = self.get_mouse_focus(target, hover_window, pressed);

        if focus_keyboard.is_some() {
            self.manager.seat_keyboard.set_focus(
                &mut self.manager.state,
                focus_keyboard,
                self.manager.serial_counter.next_serial(),
            );

            self.wm.keyboard_focus = Some(hover_window);
        }

        let global_pos = DVec2::from(global_pos);
        let linear_delta = self.get_mouse_relative(global_pos, hover_window);

        self.manager
            .send_mouse_move(focus, global_pos, linear_delta, linear_delta);
        self.wm.mouse = Some(window::MouseState {
            hover_window,
            pos: global_pos,
        });
        self.manager.send_pointer_button(index, pressed);
    }

    pub fn send_mouse_scroll(
        &mut self,
        hover_window: window::WindowHandle,
        global_pos: Vec2,
        delta: WheelDelta,
    ) {
        let global_pos = DVec2::from(global_pos);
        self.wm.mouse = Some(window::MouseState {
            hover_window,
            pos: global_pos,
        });

        self.manager.send_pointer_axis_wheel_accumulated(delta);
    }

    pub fn send_key(&mut self, virtual_key: u32, down: bool) {
        self.manager.send_key(virtual_key, down);
    }

    pub fn set_keymap(&mut self, keymap: &xkb::Keymap) -> anyhow::Result<()> {
        self.manager.set_keymap(keymap)
    }

    pub fn set_modifiers(&mut self, modifiers: u8) {
        let changed = self.cur_modifiers ^ modifiers;
        for i in 0..8 {
            let m = 1 << i;
            if changed & m != 0
                && let Some(vk) = MODS_TO_KEYS.get(m).into_iter().flatten().next()
            {
                self.send_key(*vk as u32, modifiers & m != 0);
            }
        }
        self.cur_modifiers = modifiers;
    }

    pub fn set_clipboard(&mut self, content: &str) {
        self.manager.state.set_clipboard_text(content);
    }

    pub fn add_external_process(&mut self, pid: u32) -> process::ProcessHandle {
        self.processes
            .add(process::Process::External(process::ExternalProcess { pid }))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_process(
        &mut self,
        app_name: &str,
        exec_path: &str,
        args: &[&str],
        env: &[(&str, &str)],
        resolution: [u32; 2],
        pos_mode: PositionMode,
        working_dir: Option<&str>,
        icon: Option<&str>,
        userdata: HashMap<String, String>,
    ) -> anyhow::Result<process::ProcessHandle> {
        let auth_key = generate_auth_key();

        let is_flatpak =
            exec_path.ends_with("flatpak") && args.first().is_some_and(|a| *a == "run");

        let mut cmd = std::process::Command::new(exec_path);
        self.configure_env(&mut cmd, auth_key.as_str(), is_flatpak, args);

        if let Some(working_dir) = working_dir {
            cmd.current_dir(working_dir);
        }

        for e in env {
            cmd.env(e.0, e.1);
        }

        let child = cmd.spawn().context("Failed to spawn child process")?;

        let handle = self
            .processes
            .add(process::Process::Managed(process::WayVRProcess {
                auth_key,
                child,
                exec_path: String::from(exec_path),
                app_name: String::from(app_name),
                userdata,
                args: args.iter().map(|x| String::from(*x)).collect(),
                working_dir: working_dir.map(String::from),
                env: env
                    .iter()
                    .map(|(a, b)| (String::from(*a), String::from(*b)))
                    .collect(),
                icon: icon.map(Arc::from),
                resolution,
                pos_mode,
            }));

        self.signals.send(WayVRSignal::BroadcastStateChanged(
            packet_server::WvrStateChanged::ProcessCreated,
        ));

        Ok(handle)
    }

    fn configure_env(
        &self,
        cmd: &mut std::process::Command,
        auth_key: &str,
        is_flatpak: bool,
        args: &[&str],
    ) {
        let wayland_display = self.manager.wayland_env.wayland_display_num_string();
        let x11_display = self.manager.wayland_env.display_num_string();

        // these go to env for flatpak as well
        cmd.env("WAYLAND_DISPLAY", wayland_display);
        cmd.env("DISPLAY", x11_display);

        if is_flatpak {
            // need to inject --env after "run" because --env is a
            // "flatpak run" arg, not a global "flatpak" arg
            let mut iter = args.iter();

            if let Some(first) = iter.next() {
                // add "run"
                cmd.arg(first);
            } else {
                // idk so let's just bail
                return;
            }

            // add args for "flatpak run"
            cmd.arg(format!("--env=WAYVR_DISPLAY_AUTH={auth_key}"));
            cmd.arg("--env=GDK_BACKEND=wayland,x11");
            cmd.arg("--env=QT_QPA_PLATFORM=wayland;xcb");
            cmd.arg("--env=SDL_VIDEODRIVER=wayland");
            cmd.arg("--env=CLUTTER_BACKEND=wayland");
            cmd.arg("--env=MOZ_ENABLE_WAYLAND=1");
            cmd.arg("--env=ELECTRON_OZONE_PLATFORM_HINT=wayland");

            // flatpak app id / remaining args
            for arg in iter {
                cmd.arg(arg);
            }

            return;
        }

        cmd.env("WAYVR_DISPLAY_AUTH", auth_key);
        cmd.env("GDK_BACKEND", "wayland,x11");
        cmd.env("QT_QPA_PLATFORM", "wayland;xcb");
        cmd.env("SDL_VIDEODRIVER", "wayland");
        cmd.env("CLUTTER_BACKEND", "wayland");
        cmd.env("MOZ_ENABLE_WAYLAND", "1");
        cmd.env("ELECTRON_OZONE_PLATFORM_HINT", "wayland");

        cmd.args(args);
    }
}

fn generate_auth_key() -> String {
    let uuid = uuid::Uuid::new_v4();
    uuid.to_string()
}

struct SurfaceBufWithImageContainer {
    inner: RefCell<SurfaceBufWithImage>,
    retained_dmabuf_buffer: RefCell<Option<wl_buffer::WlBuffer>>,
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct SurfaceBufWithImage {
    pub image: Arc<ImageView>,
    pub transform: Transform,
    pub scale: i32,
    pub dmabuf: bool,
}

impl SurfaceBufWithImage {
    fn apply_to_surface(
        self,
        surface_data: &SurfaceData,
        retained_buffer: Option<wl_buffer::WlBuffer>,
    ) -> Option<wl_buffer::WlBuffer> {
        if let Some(container) = surface_data.data_map.get::<SurfaceBufWithImageContainer>() {
            container.inner.replace(self);
            container.retained_dmabuf_buffer.replace(retained_buffer)
        } else {
            surface_data
                .data_map
                .insert_if_missing(|| SurfaceBufWithImageContainer {
                    inner: RefCell::new(self),
                    retained_dmabuf_buffer: RefCell::new(retained_buffer),
                });
            None
        }
    }

    pub fn get_from_surface(surface_data: &SurfaceData) -> Option<Self> {
        surface_data
            .data_map
            .get::<SurfaceBufWithImageContainer>()
            .map(|x| x.inner.borrow().clone())
    }
}
