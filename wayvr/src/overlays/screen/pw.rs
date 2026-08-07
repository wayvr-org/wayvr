use std::{
    any::Any,
    collections::HashMap,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use glam::Vec2;
use wgui::{
    event::CallbackDataCommon, i18n::Translation, log::LogErr, parser::Fetchable,
    widget::label::WidgetLabel,
};
use wlx_capture::{
    WlxCapture,
    pipewire::{ScreenCastParams, ScreenCastRequestId, ScreenCastResult, capture::PipewireCapture},
    wayland::WlxOutput,
};
use wlx_common::{astr_containers::AStrMapExt, overlays::ToastTopic};

use crate::{
    backend::task::{OverlayTask, TaskType},
    gui::panel::{GuiPanel, NewGuiPanelParams},
    overlays::{
        screen::{
            backend::CaptureType,
            capture::{WlxCaptureIn, WlxCaptureOut},
        },
        toast::Toast,
    },
    state::{AppState, save_pw_token_config},
    subsystem::dbus::DbusConnector,
    windowing::{
        OverlaySelector,
        backend::{DummyBackend, OverlayBackend},
    },
};

use super::{
    backend::ScreenBackend,
    capture::{MainThreadWlxCapture, new_wlx_capture},
};

pub(super) type ScreenCastFinalizeFn = fn(
    Arc<str>,
    Vec2,
    Vec2,
    Box<dyn WlxCapture<WlxCaptureIn, WlxCaptureOut>>,
    &mut AppState,
) -> Box<dyn OverlayBackend>;

pub struct ScreenCastPanelState {
    name: Arc<str>,
    description: Arc<str>,
    logical_pos: Vec2,
    logical_size: Vec2,
    params: Option<ScreenCastParams>,
    finalize_fn: ScreenCastFinalizeFn,
}

pub struct ScreenCastBackend {
    panel: GuiPanel<ScreenCastPanelState>,
}

impl ScreenCastBackend {
    #[cfg(feature = "wayland")]
    pub fn new_wl(
        output: &WlxOutput,
        token: Option<Rc<str>>,
        app: &mut AppState,
    ) -> anyhow::Result<Self> {
        use glam::vec2;

        fn finalize_fn(
            name: Arc<str>,
            logical_pos: Vec2,
            logical_size: Vec2,
            capture: Box<dyn WlxCapture<WlxCaptureIn, WlxCaptureOut>>,
            app: &mut AppState,
        ) -> Box<dyn OverlayBackend> {
            use wlx_capture::frame::Transform;

            let mut backend =
                ScreenBackend::new_raw(name, app.feats.xr_backend, CaptureType::PipeWire, capture);

            backend.logical_pos = logical_pos;
            backend.logical_size = logical_size;
            backend.apply_mouse_transform_with_override(Transform::Undefined);

            Box::new(backend)
        }

        let params = ScreenCastParams {
            token,
            embed_mouse: true,
            screens_only: true,
            persist: true,
            allow_multiple: false,
        };

        Self::new_raw(
            output.name.clone(),
            format!(
                "{} {} @ {},{}",
                output.make, output.model, output.logical_pos.0, output.logical_pos.1
            )
            .into(),
            vec2(output.logical_pos.0 as f32, output.logical_pos.1 as f32),
            vec2(output.logical_size.0 as f32, output.logical_size.1 as f32),
            params,
            app,
            finalize_fn,
        )
    }

    pub(super) fn new_raw(
        name: Arc<str>,
        description: Arc<str>,
        logical_pos: Vec2,
        logical_size: Vec2,
        params: ScreenCastParams,
        app: &mut AppState,
        finalize_fn: ScreenCastFinalizeFn,
    ) -> anyhow::Result<Self> {
        if app.screencast_manager.is_none() {
            anyhow::bail!("xdg-desktop-portal screencasts not supported");
        }

        let panel_params = NewGuiPanelParams {
            extra_vars: HashMap::from([
                ("name".into(), name.as_ref().into()),
                ("description".into(), description.as_ref().into()),
            ]),
            ..Default::default()
        };

        let state = ScreenCastPanelState {
            name,
            description,
            logical_pos,
            logical_size,
            params: Some(params),
            finalize_fn,
        };

        let mut panel =
            GuiPanel::new_from_template(app, "gui/screencast.xml", state, panel_params)?;

        panel.update_layout(app)?;

        Ok(Self { panel })
    }
}

impl OverlayBackend for ScreenCastBackend {
    fn init(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        if let Some(params) = self.panel.state.params.take()
            && let Some(screencast_manager) = app.screencast_manager.as_mut()
        {
            let request_id = screencast_manager.request(params)?;
            check(
                CheckParams {
                    name: self.panel.state.name.clone(),
                    description: self.panel.state.description.clone(),
                    logical_pos: self.panel.state.logical_pos,
                    logical_size: self.panel.state.logical_size,
                    user_wait: 0,
                    notify_id: None,
                    request_id,
                    last_result: None,
                    app,
                },
                self.panel.state.finalize_fn,
            );
        }

        self.panel.init(app)
    }
    fn pause(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        self.panel.pause(app)
    }
    fn resume(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        self.panel.resume(app)
    }
    fn should_render(
        &mut self,
        app: &mut AppState,
    ) -> anyhow::Result<crate::windowing::backend::ShouldRender> {
        self.panel.should_render(app)
    }
    fn render(
        &mut self,
        app: &mut AppState,
        rdr: &mut crate::windowing::backend::RenderResources,
    ) -> anyhow::Result<()> {
        self.panel.render(app, rdr)
    }
    fn frame_meta(&mut self) -> Option<crate::windowing::backend::FrameMeta> {
        self.panel.frame_meta()
    }
    fn notify(
        &mut self,
        app: &mut AppState,
        event_data: crate::windowing::backend::OverlayEventData,
    ) -> anyhow::Result<()> {
        self.panel.notify(app, event_data)
    }
    fn on_hover(
        &mut self,
        app: &mut AppState,
        hit: &crate::backend::input::PointerHit,
    ) -> crate::backend::input::HoverResult {
        self.panel.on_hover(app, hit)
    }
    fn on_left(&mut self, app: &mut AppState, pointer: usize) {
        self.panel.on_left(app, pointer);
    }
    fn on_pointer(
        &mut self,
        app: &mut AppState,
        hit: &crate::backend::input::PointerHit,
        pressed: bool,
    ) {
        self.panel.on_pointer(app, hit, pressed);
    }
    fn on_scroll(
        &mut self,
        app: &mut AppState,
        hit: &crate::backend::input::PointerHit,
        delta: crate::subsystem::hid::WheelDelta,
    ) {
        self.panel.on_scroll(app, hit, delta);
    }
    fn get_interaction_transform(&mut self) -> Option<glam::Affine2> {
        self.panel.get_interaction_transform()
    }
    fn get_attrib(
        &self,
        _attrib: wlx_common::overlays::BackendAttrib,
    ) -> Option<wlx_common::overlays::BackendAttribValue> {
        None
    }
    fn set_attrib(
        &mut self,
        _app: &mut AppState,
        _value: wlx_common::overlays::BackendAttribValue,
    ) -> bool {
        false
    }
}

struct CheckParams<'a> {
    name: Arc<str>,
    description: Arc<str>,
    logical_pos: Vec2,
    logical_size: Vec2,
    user_wait: u32,
    notify_id: Option<u32>,
    request_id: ScreenCastRequestId,
    last_result: Option<ScreenCastResult>,
    app: &'a mut AppState,
}

fn check(mut par: CheckParams, finalize_fn: ScreenCastFinalizeFn) {
    const POLL_INTERVAL: Duration = Duration::from_millis(100);

    if let Some(screencast_manager) = par.app.screencast_manager.as_mut() {
        let new_result = screencast_manager.check(&par.request_id);
        match new_result {
            ScreenCastResult::Ok(ref pw_result) => {
                log::debug!(
                    "{}: PipeWire result streams: {:?}",
                    par.name,
                    pw_result.streams
                );
                let node_id = pw_result.streams.first().unwrap().node_id; // streams guaranteed to have at least one element
                log::info!("{}: PipeWire node selected: {}", par.name, node_id);

                if let Some(id) = par.notify_id.take() {
                    let _ = DbusConnector::notify_close(id);
                }

                let pw_tokens_copy = par.app.session.pw_tokens.clone();
                if let Some(restore_token) = pw_result.restore_token.as_ref()
                    && par
                        .app
                        .session
                        .pw_tokens
                        .arc_set(par.name.clone(), restore_token.clone())
                {
                    log::info!("Adding Pipewire token for {}", par.name);
                }
                if pw_tokens_copy != par.app.session.pw_tokens {
                    // Token list changed, re-create token config file
                    if let Err(err) = save_pw_token_config(par.app.session.pw_tokens.clone()) {
                        log::error!("Failed to save Pipewire token config: {err}");
                    }
                }
                par.app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
                    OverlaySelector::Name(par.name.clone()),
                    Box::new(move |app, owc| {
                        let capture = new_wlx_capture!(
                            app.gfx_extras.queue_capture,
                            PipewireCapture::new(par.name.clone(), node_id)
                        );

                        owc.backend =
                            finalize_fn(par.name, par.logical_pos, par.logical_size, capture, app);

                        let _ = owc
                            .backend
                            .init(app)
                            .log_err("Failed to init ScreenBackend");
                    }),
                )));
            }
            // TODO: display status to user
            ScreenCastResult::Queued
            | ScreenCastResult::Pending
            | ScreenCastResult::WaitingForUser => {
                let user_wait_add = if matches!(new_result, ScreenCastResult::WaitingForUser) {
                    if par.user_wait == 2 {
                        par.notify_id = DbusConnector::notify_send(
                            "Select screen cast for:",
                            format!("{} {}", par.name, par.description).as_str(),
                            1,
                            30000,
                            0,
                            true,
                        )
                        .ok();
                    }
                    1
                } else {
                    0
                };

                if par
                    .last_result
                    .is_none_or(|last_result| last_result != new_result)
                {
                    update_status_hack(&par.name, &par.description, &new_result, par.app);
                }

                par.app.tasks.enqueue_at(
                    TaskType::Overlay(OverlayTask::Modify(
                        OverlaySelector::Name(par.name.clone()),
                        Box::new({
                            move |app, _owc| {
                                check(
                                    CheckParams {
                                        name: par.name.clone(),
                                        description: par.description.clone(),
                                        logical_pos: par.logical_pos,
                                        logical_size: par.logical_size,
                                        user_wait: par.user_wait + user_wait_add,
                                        notify_id: par.notify_id,
                                        request_id: par.request_id,
                                        last_result: Some(new_result),
                                        app,
                                    },
                                    finalize_fn,
                                );
                            }
                        }),
                    )),
                    Instant::now() + POLL_INTERVAL,
                );
            }
            ScreenCastResult::Failed(ref e) => {
                if let Some(id) = par.notify_id.take() {
                    let _ = DbusConnector::notify_close(id);
                }

                update_status_hack(&par.name, &par.description, &new_result, par.app);

                log::warn!("Failed to create mirror due to PipeWire error: {e:?}");
                Toast::new(
                    ToastTopic::Error,
                    "TOAST.TITLE_SCREENCAST_FAIL".into(),
                    format!("{e}"),
                )
                .submit(par.app);
            }
        }
    }
}

fn update_status_hack(
    name: &Arc<str>,
    description: &Arc<str>,
    result: &ScreenCastResult,
    app: &mut AppState,
) {
    let message = match result {
        ScreenCastResult::Queued => "SCREENCAST.QUEUED",
        ScreenCastResult::WaitingForUser => {
            if description.is_empty() {
                "SCREENCAST.WAITING_FOR_USER"
            } else {
                "SCREENCAST.WAITING_FOR_USER_DESC"
            }
        }
        ScreenCastResult::Failed(_) => "SCREENCAST.FAIL",
        _ => "SCREENCAST.PENDING", // Ok is technically never visible
    };

    app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
        OverlaySelector::Name(name.clone()),
        Box::new({
            move |_app, owc| {
                let backend =
                    std::mem::replace(&mut owc.backend, Box::new(DummyBackend {})) as Box<dyn Any>;
                let mut backend = backend
                    .downcast::<ScreenCastBackend>()
                    .expect("Wrong type to unwrap");
                {
                    let Ok(mut label) = backend
                        .panel
                        .parser_state
                        .fetch_widget_as::<WidgetLabel>(&backend.panel.layout.state, "status")
                        .log_err("element with id=\"status\" not found in screencast.xml!")
                    else {
                        return;
                    };
                    let mut common = CallbackDataCommon {
                        state: &backend.panel.layout.state,
                        alterables: &mut backend.panel.layout.alterables,
                    };
                    label.set_text(&mut common, Translation::from_translation_key(message));
                }
                let _ = std::mem::replace(&mut owc.backend, backend);
            }
        }),
    )));
}
