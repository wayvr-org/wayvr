use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use glam::{Affine2, Affine3A, Quat, Vec2, Vec3, vec3};
use wlx_capture::{WlxCapture, pipewire::ScreenCastParams};
use wlx_common::{
    overlays::{BackendAttrib, BackendAttribValue},
    windowing::OverlayWindowState,
};

use crate::{
    backend::input::{HoverResult, PointerHit},
    config::none_if_0,
    overlays::screen::{
        backend::CaptureType,
        capture::{WlxCaptureIn, WlxCaptureOut},
        pw::ScreenCastBackend,
    },
    state::AppState,
    subsystem::hid::WheelDelta,
    windowing::{
        backend::{FrameMeta, OverlayBackend, OverlayEventData, RenderResources, ShouldRender},
        window::{OverlayCategory, OverlayWindowConfig},
    },
};

use super::backend::ScreenBackend;
static MIRROR_COUNTER: AtomicUsize = AtomicUsize::new(1);

pub struct MirrorBackend(ScreenBackend);

impl OverlayBackend for MirrorBackend {
    fn init(&mut self, _app: &mut AppState) -> anyhow::Result<()> {
        Ok(())
    }
    fn should_render(&mut self, app: &mut AppState) -> anyhow::Result<ShouldRender> {
        self.0.should_render(app)
    }
    fn render(&mut self, app: &mut AppState, rdr: &mut RenderResources) -> anyhow::Result<()> {
        self.0.render(app, rdr)?;
        Ok(())
    }
    fn pause(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        self.0.pause(app)
    }
    fn resume(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        self.0.resume(app)
    }

    fn frame_meta(&mut self) -> Option<FrameMeta> {
        self.0.frame_meta()
    }

    fn notify(&mut self, app: &mut AppState, event_data: OverlayEventData) -> anyhow::Result<()> {
        self.0.notify(app, event_data)
    }

    fn on_hover(&mut self, _: &mut AppState, _: &PointerHit) -> HoverResult {
        HoverResult::consume()
    }
    fn on_left(&mut self, _: &mut AppState, _: usize) {}
    fn on_pointer(&mut self, _: &mut AppState, _: &PointerHit, _: bool) {}
    fn on_scroll(&mut self, _: &mut AppState, _: &PointerHit, _delta: WheelDelta) {}
    fn get_interaction_transform(&mut self) -> Option<Affine2> {
        self.0.get_interaction_transform()
    }
    fn get_attrib(&self, attrib: BackendAttrib) -> Option<BackendAttribValue> {
        self.0.get_attrib(attrib)
    }
    fn set_attrib(&mut self, app: &mut AppState, value: BackendAttribValue) -> bool {
        self.0.set_attrib(app, value)
    }
}

pub fn new_mirror_name() -> Arc<str> {
    format!("M-{}", MIRROR_COUNTER.fetch_add(1, Ordering::Relaxed)).into()
}

pub fn new_mirror(name: Arc<str>, app: &mut AppState) -> anyhow::Result<OverlayWindowConfig> {
    fn finalize_fn(
        name: Arc<str>,
        _: Vec2,
        _: Vec2,
        capture: Box<dyn WlxCapture<WlxCaptureIn, WlxCaptureOut>>,
        app: &mut AppState,
    ) -> Box<dyn OverlayBackend> {
        let renderer = ScreenBackend::new_raw(
            name.clone(),
            app.feats.xr_backend,
            CaptureType::PipeWire,
            capture,
        );

        let backend = MirrorBackend(renderer);

        Box::new(backend)
    }

    let params = ScreenCastParams {
        token: None,
        embed_mouse: true,
        screens_only: false,
        persist: false,
        allow_multiple: false,
    };

    let backend = ScreenCastBackend::new_raw(
        name.clone(),
        "".into(),
        Vec2::ZERO,
        Vec2::ZERO,
        params,
        app,
        finalize_fn,
    )?;

    Ok(OverlayWindowConfig {
        name: name.clone(),
        category: OverlayCategory::Mirror,
        show_on_spawn: true,
        default_state: OverlayWindowState {
            positioning: app.session.config.default_positioning.into(),
            curvature: none_if_0(app.session.config.default_curvature),
            alpha: app.session.config.default_opacity,
            interactable: true,
            grabbable: true,
            transform: Affine3A::from_scale_rotation_translation(
                Vec3::ONE * app.session.config.default_overlay_scale,
                Quat::IDENTITY,
                vec3(0.0, 0.2, -0.35),
            ),
            ..OverlayWindowState::default()
        },
        ..OverlayWindowConfig::from_backend(Box::new(backend))
    })
}
