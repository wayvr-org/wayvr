use std::sync::atomic::Ordering;

use dash_frontend::frontend::{self, FrontendTask, FrontendUpdateParams};
use glam::{Affine2, Affine3A, Vec2, vec2, vec3};
use wayvr_ipc::{
    packet_client::WvrProcessLaunchParams,
    packet_server::{WvrProcess, WvrProcessHandle, WvrWindow, WvrWindowHandle},
};
use wgui::{
    event::{
        Event as WguiEvent, MouseButtonEvent, MouseButtonIndex, MouseLeaveEvent, MouseMotionEvent,
        MouseWheelEvent,
    },
    gfx::cmd::WGfxClearMode,
    renderer_vk::context::Context as WguiContext,
    widget::EventResult,
};
use wlx_common::{
    dash_interface::{self, DashInterface, RecenterMode},
    locale::WayVRLangProvider,
    overlays::{BackendAttrib, BackendAttribValue},
};
use wlx_common::{
    timestep::Timestep,
    windowing::{OverlayWindowState, Positioning},
};

#[cfg(feature = "openxr")]
use libmonado::{ClientLogic, DeviceLogic};

use crate::{
    RESTART, RUNNING,
    backend::{
        XrBackend,
        input::{Haptics, HoverResult, PointerHit, PointerMode},
        task::{OverlayTask, PlayspaceTask, TaskType, ToggleMode},
        wayvr::{
            process::{KillSignal, ProcessHandle},
            window::WindowHandle,
        },
    },
    config::save_settings,
    ipc::ipc_server::{gen_args_vec, gen_env_vec},
    state::AppState,
    subsystem::hid::WheelDelta,
    windowing::{
        OverlaySelector, Z_ORDER_DASHBOARD,
        backend::{
            FrameMeta, OverlayBackend, OverlayEventData, RenderResources, ShouldRender,
            ui_transform,
        },
        window::{OverlayCategory, OverlayWindowConfig},
    },
};

pub const DASH_NAME: &str = "Dashboard";

const DASH_RES_U32A: [u32; 2] = [1920, 1080];
const DASH_RES_VEC2: Vec2 = vec2(DASH_RES_U32A[0] as _, DASH_RES_U32A[1] as _);

pub struct DashFrontend {
    inner: frontend::Frontend<AppState>,
    initialized: bool,
    interaction_transform: Option<Affine2>,
    timestep: Timestep,
    has_focus: [bool; 2],
    context: WguiContext,
}

const GUI_SCALE: f32 = 2.0;

impl DashFrontend {
    fn new(app: &mut AppState) -> anyhow::Result<Self> {
        let mut interface = DashInterfaceLive::new();

        for p in app.session.config.autostart_apps.clone() {
            let _ = interface.process_launch(app, false, p);
        }

        let frontend = frontend::Frontend::new(frontend::InitParams {
            interface: Box::new(interface),
            lang_provider: &WayVRLangProvider::from_config(&app.session.config),
            has_monado: matches!(app.xr_backend, XrBackend::OpenXR),
            theme: app.wgui_theme.clone(),
        })?;

        frontend
            .tasks
            .push(FrontendTask::PlaySound(frontend::SoundType::Startup));

        let context = WguiContext::new(&mut app.wgui_shared, 1.0)?;
        Ok(Self {
            inner: frontend,
            initialized: false,
            interaction_transform: None,
            timestep: Timestep::new(60.0),
            has_focus: [false, false],
            context,
        })
    }

    fn update(&mut self, app: &mut AppState, timestep_alpha: f32) -> anyhow::Result<()> {
        let res = self.inner.update(FrontendUpdateParams {
            data: app,
            width: DASH_RES_VEC2.x / GUI_SCALE,
            height: DASH_RES_VEC2.y / GUI_SCALE,
            timestep_alpha,
        })?;
        self.inner
            .process_update(res, &mut app.audio_system, &mut app.audio_sample_player)?;
        Ok(())
    }

    fn push_event(&mut self, event: &WguiEvent) -> EventResult {
        match self.inner.layout.push_event(event, &mut (), &mut ()) {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to push event: {e:?}");
                EventResult::NoHit
            }
        }
    }
}

impl OverlayBackend for DashFrontend {
    fn init(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        self.context
            .update_viewport(&mut app.wgui_shared, DASH_RES_U32A, GUI_SCALE)?;
        self.interaction_transform = Some(ui_transform(DASH_RES_U32A));

        if self.inner.layout.content_size.x * self.inner.layout.content_size.y != 0.0 {
            self.update(app, 0.0)?;
            self.initialized = true;
        }
        Ok(())
    }

    fn pause(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        if app.session.config_dirty {
            save_settings(&app.session.config)?;
            app.session.config_dirty = false;
        }

        Ok(())
    }

    fn resume(&mut self, _app: &mut AppState) -> anyhow::Result<()> {
        self.inner.layout.needs_redraw = true;
        self.timestep.reset();
        Ok(())
    }

    fn should_render(&mut self, app: &mut AppState) -> anyhow::Result<ShouldRender> {
        while self.timestep.on_tick() {
            self.inner.layout.tick()?;
        }

        if let Err(e) = self.update(app, self.timestep.alpha) {
            log::error!("uncaught exception: {e:?}");
        }

        Ok(if self.inner.layout.check_toggle_needs_redraw() {
            ShouldRender::Should
        } else {
            ShouldRender::Can
        })
    }

    fn render(&mut self, app: &mut AppState, rdr: &mut RenderResources) -> anyhow::Result<()> {
        let globals = self.inner.layout.state.globals.clone(); // sorry
        let mut globals = globals.get();

        let primitives = wgui::drawing::draw(&mut wgui::drawing::DrawParams {
            globals: &mut globals,
            layout: &mut self.inner.layout,
            debug_draw: false,
            timestep_alpha: self.timestep.alpha,
        })?;
        self.context.draw(
            &globals.font_system,
            &mut app.wgui_shared,
            rdr.cmd_buf_single(),
            &primitives,
        )?;
        Ok(())
    }

    fn frame_meta(&mut self) -> Option<FrameMeta> {
        Some(FrameMeta {
            clear: WGfxClearMode::Clear([0., 0., 0., 0.]),
            extent: [DASH_RES_U32A[0], DASH_RES_U32A[1]],
            ..Default::default()
        })
    }

    fn notify(&mut self, _app: &mut AppState, _data: OverlayEventData) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_scroll(&mut self, _app: &mut AppState, hit: &PointerHit, delta: WheelDelta) {
        let e = WguiEvent::MouseWheel(MouseWheelEvent {
            delta: vec2(delta.x, delta.y) / 8.0,
            pos: hit.uv * self.inner.layout.content_size,
            device: hit.pointer,
        });
        self.push_event(&e);
    }

    fn on_hover(&mut self, _app: &mut AppState, hit: &PointerHit) -> HoverResult {
        let e = &WguiEvent::MouseMotion(MouseMotionEvent {
            pos: hit.uv * self.inner.layout.content_size,
            device: hit.pointer,
        });

        self.has_focus[hit.pointer] = true;

        let result = self.push_event(e);

        HoverResult {
            consume: result != EventResult::NoHit,
            haptics: self
                .inner
                .layout
                .check_toggle_haptics_triggered()
                .then_some(Haptics {
                    intensity: 0.1,
                    duration: 0.01,
                    frequency: 5.0,
                }),
        }
    }

    fn on_left(&mut self, _app: &mut AppState, pointer: usize) {
        let e = WguiEvent::MouseLeave(MouseLeaveEvent { device: pointer });
        self.has_focus[pointer] = false;
        self.push_event(&e);
    }

    fn on_pointer(&mut self, _app: &mut AppState, hit: &PointerHit, pressed: bool) {
        let index = match hit.mode {
            PointerMode::Left => MouseButtonIndex::Left,
            PointerMode::Right => MouseButtonIndex::Right,
            PointerMode::Middle => MouseButtonIndex::Middle,
            _ => return,
        };

        let e = if pressed {
            WguiEvent::MouseDown(MouseButtonEvent {
                pos: hit.uv * self.inner.layout.content_size,
                index,
                device: hit.pointer,
            })
        } else {
            WguiEvent::MouseUp(MouseButtonEvent {
                pos: hit.uv * self.inner.layout.content_size,
                index,
                device: hit.pointer,
            })
        };
        self.push_event(&e);

        // released while off-panel → send mouse leave as well
        if !pressed && !self.has_focus[hit.pointer] {
            let e = WguiEvent::MouseMotion(MouseMotionEvent {
                pos: vec2(-1., -1.),
                device: hit.pointer,
            });
            self.push_event(&e);
            let e = WguiEvent::MouseLeave(MouseLeaveEvent {
                device: hit.pointer,
            });
            self.push_event(&e);
        }
    }

    fn get_interaction_transform(&mut self) -> Option<Affine2> {
        self.interaction_transform
    }
    fn get_attrib(&self, _attrib: BackendAttrib) -> Option<BackendAttribValue> {
        None
    }
    fn set_attrib(&mut self, _app: &mut AppState, _value: BackendAttribValue) -> bool {
        false
    }
}

pub fn create_dash_frontend(app: &mut AppState) -> anyhow::Result<OverlayWindowConfig> {
    Ok(OverlayWindowConfig {
        name: DASH_NAME.into(),
        default_state: OverlayWindowState {
            transform: Affine3A::from_translation(vec3(0., 0., -0.9)),
            grabbable: true,
            interactable: true,
            positioning: Positioning::Floating,
            curvature: Some(0.15),
            ..OverlayWindowState::default()
        },
        z_order: Z_ORDER_DASHBOARD,
        category: OverlayCategory::Dashboard,
        global: true,
        ..OverlayWindowConfig::from_backend(Box::new(DashFrontend::new(app)?))
    })
}
pub struct DashInterfaceLive {}

impl DashInterfaceLive {
    pub const fn new() -> Self {
        Self {}
    }
}

impl DashInterface<AppState> for DashInterfaceLive {
    fn window_list(&mut self, app: &mut AppState) -> anyhow::Result<Vec<WvrWindow>> {
        let wvr_server = app.wvr_server.as_mut().unwrap();
        Ok(wvr_server
            .wm
            .windows
            .iter()
            .map(|(handle, win)| WvrWindow {
                handle: WindowHandle::as_packet(&handle),
                process_handle: ProcessHandle::as_packet(&win.process),
                size_x: win.size_x,
                size_y: win.size_y,
                visible: win.visible,
            })
            .collect())
    }

    fn window_request_close(
        &mut self,
        app: &mut AppState,
        handle: WvrWindowHandle,
    ) -> anyhow::Result<()> {
        let wvr_server = app.wvr_server.as_mut().unwrap();
        wvr_server
            .wm
            .remove_window(WindowHandle::from_packet(handle));
        Ok(())
    }

    fn process_get(&mut self, app: &mut AppState, handle: WvrProcessHandle) -> Option<WvrProcess> {
        let wvr_server = app.wvr_server.as_mut().unwrap();
        let handle = ProcessHandle::from_packet(handle);
        wvr_server
            .processes
            .get(&handle)
            .map(|x| x.to_packet(handle))
    }

    fn process_launch(
        &mut self,
        app: &mut AppState,
        auto_start: bool,
        params: WvrProcessLaunchParams,
    ) -> anyhow::Result<WvrProcessHandle> {
        let wvr_server = app.wvr_server.as_mut().unwrap();

        let args_vec = gen_args_vec(&params.args);
        let env_vec = gen_env_vec(&params.env);

        if auto_start {
            app.session.config.autostart_apps.push(params.clone());
            save_settings(&app.session.config)?;
        }

        wvr_server
            .spawn_process(
                &params.name,
                &params.exec,
                &args_vec,
                &env_vec,
                params.resolution,
                params.pos_mode,
                None,
                params.icon.as_deref(),
                params.userdata,
            )
            .map(|x| x.as_packet())
    }

    fn process_list(&mut self, app: &mut AppState) -> anyhow::Result<Vec<WvrProcess>> {
        let wvr_server = app.wvr_server.as_mut().unwrap();
        Ok(wvr_server
            .processes
            .iter()
            .map(|(hnd, p)| p.to_packet(hnd))
            .collect())
    }

    fn process_terminate(
        &mut self,
        app: &mut AppState,
        handle: WvrProcessHandle,
    ) -> anyhow::Result<()> {
        let wvr_server = app.wvr_server.as_mut().unwrap();
        wvr_server.terminate_process(ProcessHandle::from_packet(handle), KillSignal::Term);
        Ok(())
    }

    fn window_set_visible(
        &mut self,
        app: &mut AppState,
        handle: WvrWindowHandle,
        visible: bool,
    ) -> anyhow::Result<()> {
        let wvr_server = app.wvr_server.as_mut().unwrap();
        let Some(oid) = wvr_server.get_overlay_id(WindowHandle::from_packet(handle)) else {
            return Ok(());
        };

        app.tasks
            .enqueue(TaskType::Overlay(OverlayTask::ToggleOverlay(
                OverlaySelector::Id(oid),
                if visible {
                    ToggleMode::EnsureOn
                } else {
                    ToggleMode::EnsureOff
                },
            )));
        Ok(())
    }

    fn recenter_playspace(&mut self, app: &mut AppState, mode: RecenterMode) -> anyhow::Result<()> {
        let task = match mode {
            RecenterMode::FixFloor => PlayspaceTask::FixFloor,
            RecenterMode::Recenter => PlayspaceTask::Recenter,
            RecenterMode::Reset => PlayspaceTask::Reset,
        };
        app.tasks.enqueue(TaskType::Playspace(task));
        Ok(())
    }

    fn desktop_finder<'a>(
        &'a mut self,
        data: &'a mut AppState,
    ) -> &'a mut wlx_common::desktop_finder::DesktopFinder {
        &mut data.desktop_finder
    }

    fn general_config<'a>(
        &'a mut self,
        data: &'a mut AppState,
    ) -> &'a mut wlx_common::config::GeneralConfig {
        &mut data.session.config
    }

    fn config_changed(&mut self, data: &mut AppState) {
        data.session.config_dirty = true;
        #[cfg(feature = "openxr")]
        {
            use crate::backend::task::OpenXrTask;
            data.tasks
                .enqueue(TaskType::OpenXR(OpenXrTask::SettingsChanged));
        }
        data.tasks
            .enqueue(TaskType::Overlay(OverlayTask::SettingsChanged));
    }

    fn restart(&mut self, _data: &mut AppState) {
        RUNNING.store(false, Ordering::Relaxed);
        RESTART.store(true, Ordering::Relaxed);
    }

    fn toggle_dashboard(&mut self, data: &mut AppState) {
        data.tasks
            .enqueue(TaskType::Overlay(OverlayTask::ToggleDashboard));
    }

    #[cfg(feature = "openxr")]
    fn monado_client_list(
        &mut self,
        app: &mut AppState,
        filtered: bool,
    ) -> anyhow::Result<Vec<dash_interface::MonadoClient>> {
        let Some(monado) = &mut app.monado_state else {
            return Ok(Vec::new()); // no monado available
        };

        let clients = if filtered {
            monado_list_clients_filtered(&mut monado.ipc)?
        } else {
            monado.ipc.clients()?.into_iter().collect()
        };

        let mut res = Vec::<dash_interface::MonadoClient>::new();

        for mut client in clients {
            let client_id = client.id();
            let name = client.name()?;
            let state = client.state()?;

            res.push(dash_interface::MonadoClient {
                id: client_id as i64,
                name,
                is_primary: state.contains(libmonado::ClientState::ClientPrimaryApp),
                is_active: state.contains(libmonado::ClientState::ClientSessionActive),
                is_visible: state.contains(libmonado::ClientState::ClientSessionVisible),
                is_focused: state.contains(libmonado::ClientState::ClientSessionFocused),
                is_overlay: state.contains(libmonado::ClientState::ClientSessionOverlay),
                is_io_active: state.contains(libmonado::ClientState::ClientIoActive),
            });
        }

        Ok(res)
    }

    #[cfg(feature = "openxr")]
    fn monado_client_focus(&mut self, app: &mut AppState, name: &str) -> anyhow::Result<()> {
        let Some(monado) = &mut app.monado_state else {
            return Ok(()); // no monado available
        };

        monado_client_focus(&mut monado.ipc, name)
    }

    #[cfg(feature = "openxr")]
    fn monado_brightness_get(&mut self, app: &mut AppState) -> Option<f32> {
        let Some(monado) = &mut app.monado_state else {
            return None;
        };

        monado_get_brightness(&mut monado.ipc)
    }

    #[cfg(feature = "openxr")]
    fn monado_brightness_set(&mut self, app: &mut AppState, brightness: f32) -> Option<()> {
        let Some(monado) = &mut app.monado_state else {
            return None;
        };

        monado_set_brightness(&mut monado.ipc, brightness).ok()
    }

    #[cfg(feature = "openxr")]
    fn monado_metrics_set_enabled(&mut self, app: &mut AppState, enabled: bool) -> bool {
        #[cfg(feature = "feat-monado-metrics")]
        {
            let Some(monado) = &mut app.monado_state else {
                return false;
            };

            if let Err(e) = monado.set_metrics_enabled(enabled) {
                log::error!("failed to enable metrics: {e:?}");
                return false;
            }
            true
        }
        #[cfg(not(feature = "feat-monado-metrics"))]
        #[allow(path_statements)]
        {
            app;
            enabled;
            false
        }
    }

    #[cfg(feature = "openxr")]
    #[allow(clippy::match_same_arms)]
    fn monado_metrics_dump_session_frames(
        &mut self,
        app: &mut AppState,
    ) -> Vec<dash_interface::MonadoDumpSessionFrame> {
        #[cfg(feature = "feat-monado-metrics")]
        {
            let Some(monado) = &mut app.monado_state else {
                return Vec::new();
            };
            let Some(metrics) = &mut monado.metrics else {
                return Vec::new(); // metrics not enabled or not available
            };

            metrics
                .dump_records()
                .iter()
                .filter_map(|record| {
                    use crate::subsystem::monado_metrics::proto::record;
                    let record = record.record?;
                    match record {
                        record::Record::SessionFrame(sframe) => {
                            // map it to our struct
                            Some(dash_interface::MonadoDumpSessionFrame {
                                session_id: sframe.session_id,
                                frame_id: sframe.frame_id,
                                predicted_frame_time_ns: sframe.predicted_frame_time_ns,
                                predicted_wake_up_time_ns: sframe.predicted_wake_up_time_ns,
                                predicted_gpu_done_time_ns: sframe.predicted_gpu_done_time_ns,
                                predicted_display_time_ns: sframe.predicted_display_time_ns,
                                predicted_display_period_ns: sframe.predicted_display_period_ns,
                                display_time_ns: sframe.display_time_ns,
                                when_predicted_ns: sframe.when_predicted_ns,
                                when_wait_woke_ns: sframe.when_wait_woke_ns,
                                when_begin_ns: sframe.when_begin_ns,
                                when_delivered_ns: sframe.when_delivered_ns,
                                when_gpu_done_ns: sframe.when_gpu_done_ns,
                                discarded: sframe.discarded,
                            })
                        }
                        record::Record::Version(_) => None,
                        record::Record::Used(_) => None,
                        record::Record::SystemFrame(_) => None,
                        record::Record::SystemGpuInfo(_) => None,
                        record::Record::SystemPresentInfo(_) => None,
                    }
                })
                .collect()
        }
        #[cfg(not(feature = "feat-monado-metrics"))]
        #[allow(path_statements)]
        {
            app;
            Vec::new()
        }
    }

    #[cfg(not(feature = "openxr"))]
    fn monado_client_list(
        &mut self,
        _: &mut AppState,
        _filtered: bool,
    ) -> anyhow::Result<Vec<dash_interface::MonadoClient>> {
        anyhow::bail!("Not supported in this build.")
    }
    #[cfg(not(feature = "openxr"))]
    fn monado_client_focus(&mut self, _: &mut AppState, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("Not supported in this build.")
    }
    #[cfg(not(feature = "openxr"))]
    fn monado_brightness_get(&mut self, _: &mut AppState) -> Option<f32> {
        None
    }
    #[cfg(not(feature = "openxr"))]
    fn monado_brightness_set(&mut self, _: &mut AppState, _: f32) -> Option<()> {
        None
    }

    #[cfg(not(feature = "openxr"))]
    fn monado_metrics_set_enabled(&mut self, _: &mut AppState, _enabled: bool) -> bool {
        // not supported in this build
        false
    }

    #[cfg(not(feature = "openxr"))]
    fn monado_metrics_dump_session_frames(
        &mut self,
        _: &mut AppState,
    ) -> Vec<dash_interface::MonadoDumpSessionFrame> {
        Vec::new() // not supported in this build
    }
}

#[cfg(feature = "openxr")]
fn monado_get_brightness(monado: &mut libmonado::Monado) -> Option<f32> {
    let device = monado.device_from_role(libmonado::DeviceRole::Head).ok()?;
    device.brightness().ok()
}

#[cfg(feature = "openxr")]
fn monado_set_brightness(monado: &mut libmonado::Monado, brightness: f32) -> anyhow::Result<()> {
    let device = monado.device_from_role(libmonado::DeviceRole::Head)?;
    device.set_brightness(brightness, false)?;
    Ok(())
}

#[cfg(feature = "openxr")]
fn monado_list_clients_filtered(
    monado: &mut libmonado::Monado,
) -> anyhow::Result<Vec<libmonado::Client<'_>>> {
    let mut clients: Vec<_> = monado.clients()?.into_iter().collect();

    let clients: Vec<_> = clients
        .iter_mut()
        .filter_map(|client| {
            use libmonado::ClientState;
            let Ok(state) = client.state() else {
                return None;
            };

            if !state.contains(ClientState::ClientSessionActive)
                || state.contains(ClientState::ClientSessionOverlay)
            {
                return None;
            }

            Some(client.clone())
        })
        .collect();

    Ok(clients)
}

#[cfg(feature = "openxr")]
fn monado_client_focus(monado: &mut libmonado::Monado, name: &str) -> anyhow::Result<()> {
    let clients = monado_list_clients_filtered(monado)?;

    for mut client in clients {
        let client_name = client.name()?;
        if client_name != name {
            continue;
        }

        log::info!("Monado focus set to {client_name}");
        client.set_primary()?;
        return Ok(());
    }

    Ok(())
}
