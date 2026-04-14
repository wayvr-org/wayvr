use std::{
    sync::{Arc, LazyLock, atomic::AtomicU64},
    time::Instant,
};

use glam::{Affine2, Vec2, vec2};
use wlx_capture::{WlxCapture, frame::Transform};

use crate::{
    backend::{
        XrBackend,
        input::{HoverResult, PointerHit, PointerMode},
    },
    overlays::screen::capture::MyFirstDmaExporter,
    state::AppState,
    subsystem::hid::{MOUSE_LEFT, MOUSE_MIDDLE, MOUSE_RIGHT, WheelDelta},
    windowing::backend::{
        FrameMeta, OverlayBackend, OverlayEventData, RenderResources, ShouldRender, ui_transform,
    },
};
use wlx_common::{
    config::CaptureMethod,
    overlays::{BackendAttrib, BackendAttribValue, MouseTransform, StereoMode},
};

use super::capture::{ScreenPipeline, WlxCaptureIn, WlxCaptureOut, receive_callback};

static START: LazyLock<Instant> = LazyLock::new(Instant::now);
static NEXT_MOVE: AtomicU64 = AtomicU64::new(0);

fn can_move() -> bool {
    START.elapsed().as_millis() as u64 > NEXT_MOVE.load(std::sync::atomic::Ordering::Relaxed)
}

fn set_next_move(millis_from_now: u64) {
    NEXT_MOVE.store(
        START.elapsed().as_millis() as u64 + millis_from_now,
        std::sync::atomic::Ordering::Relaxed,
    );
}

pub(super) enum CaptureType {
    PipeWire,
    ScreenCopy,
    Xshm,
}

pub struct ScreenBackend {
    name: Arc<str>,
    capture_type: CaptureType,
    capture: Box<dyn WlxCapture<WlxCaptureIn, WlxCaptureOut>>,
    pipeline: Option<ScreenPipeline>,
    cur_frame: Option<WlxCaptureOut>,
    meta: Option<FrameMeta>,
    mouse_transform: Affine2,
    interaction_transform: Option<Affine2>,
    stereo: Option<StereoMode>,
    stereo_full_frame: bool,
    stereo_adjust_mouse: bool,
    crop_rect: [f32; 4],
    crop_dirty: bool,
    pub(super) logical_pos: Vec2,
    pub(super) logical_size: Vec2,
    pub(super) mouse_transform_original: Transform,
    mouse_transform_override: MouseTransform,
    just_resumed: bool,
    source_meta: Option<FrameMeta>,
}

impl ScreenBackend {
    pub(super) fn new_raw(
        name: Arc<str>,
        xr_backend: XrBackend,
        capture_type: CaptureType,
        capture: Box<dyn WlxCapture<WlxCaptureIn, WlxCaptureOut>>,
    ) -> Self {
        Self {
            name,
            capture_type,
            capture,
            pipeline: None,
            cur_frame: None,
            meta: None,
            mouse_transform: Affine2::ZERO,
            interaction_transform: None,
            stereo: if matches!(xr_backend, XrBackend::OpenXR) {
                Some(StereoMode::None)
            } else {
                None
            },
            stereo_full_frame: false,
            stereo_adjust_mouse: false,
            crop_rect: [0.0, 0.0, 1.0, 1.0],
            crop_dirty: false,
            logical_pos: Vec2::ZERO,
            logical_size: Vec2::ZERO,
            mouse_transform_original: Transform::Undefined,
            mouse_transform_override: MouseTransform::Default,
            just_resumed: false,
            source_meta: None,
        }
    }

    pub(super) fn apply_mouse_transform_with_override(&mut self, override_transform: Transform) {
        let crop_origin = Vec2::new(self.crop_rect[0], self.crop_rect[1]);
        let crop_size = Vec2::new(self.crop_rect[2], self.crop_rect[3]);
        let mut size = self.logical_size * crop_size;
        let pos = self.logical_pos + self.logical_size * crop_origin;

        if self.stereo_adjust_mouse
            && let Some(stereo) = self.stereo.as_ref()
        {
            match stereo {
                StereoMode::LeftRight | StereoMode::RightLeft => size.x *= 0.5,
                StereoMode::TopBottom | StereoMode::BottomTop => size.y *= 0.5,
                _ => {}
            }
        }

        let transform = match override_transform {
            Transform::Undefined => self.mouse_transform_original,
            other => other,
        };

        self.mouse_transform = match transform {
            Transform::Normal | Transform::Undefined => {
                Affine2::from_cols(vec2(size.x, 0.), vec2(0., size.y), pos)
            }
            Transform::Rotated90 => Affine2::from_cols(
                vec2(0., size.y),
                vec2(-size.x, 0.),
                vec2(pos.x + size.x, pos.y),
            ),
            Transform::Rotated180 => Affine2::from_cols(
                vec2(-size.x, 0.),
                vec2(0., -size.y),
                vec2(pos.x + size.x, pos.y + size.y),
            ),
            Transform::Rotated270 => Affine2::from_cols(
                vec2(0., -size.y),
                vec2(size.x, 0.),
                vec2(pos.x, pos.y + size.y),
            ),
            Transform::Flipped => Affine2::from_cols(
                vec2(-size.x, 0.),
                vec2(0., size.y),
                vec2(pos.x + size.x, pos.y),
            ),
            Transform::Flipped90 => {
                Affine2::from_cols(vec2(0., size.y), vec2(size.x, 0.), vec2(pos.x, pos.y))
            }
            Transform::Flipped180 => Affine2::from_cols(
                vec2(size.x, 0.),
                vec2(0., -size.y),
                vec2(pos.x, pos.y + size.y),
            ),
            Transform::Flipped270 => Affine2::from_cols(
                vec2(0., -size.y),
                vec2(-size.x, 0.),
                vec2(pos.x + size.x, pos.y + size.y),
            ),
        };
    }

    fn effective_extent(&self, source_extent: [u32; 2]) -> [u32; 2] {
        [
            (source_extent[0] as f32 * self.crop_rect[2])
                .round()
                .max(1.0) as u32,
            (source_extent[1] as f32 * self.crop_rect[3])
                .round()
                .max(1.0) as u32,
        ]
    }

    fn apply_meta_state(
        &mut self,
        app: &mut AppState,
        mut source_meta: FrameMeta,
        stereo: StereoMode,
    ) -> anyhow::Result<()> {
        let source_extent = source_meta.extent;
        let effective_extent = self.effective_extent(source_extent);
        source_meta.extent = effective_extent;

        if let Some(pipeline) = self.pipeline.as_mut() {
            if self
                .meta
                .is_some_and(|old| old.extent != source_meta.extent)
            {
                pipeline.set_extent(
                    app,
                    [source_meta.extent[0] as f32, source_meta.extent[1] as f32],
                    [0.0, 0.0],
                    [source_extent[0] as f32, source_extent[1] as f32],
                )?;
                self.interaction_transform = Some(ui_transform(source_meta.extent));
            }
            if self.crop_dirty {
                pipeline.set_crop_rect(self.crop_rect)?;
            }
            pipeline.ensure_stereo(stereo);
        } else {
            let pipeline = ScreenPipeline::new(
                &source_meta,
                source_extent,
                app,
                stereo,
                [0.0, 0.0],
                self.crop_rect,
            )?;
            self.pipeline = Some(pipeline);
            self.interaction_transform = Some(ui_transform(source_meta.extent));
        }

        self.meta = Some(source_meta);
        self.crop_dirty = false;
        let frame_transform = mouse_transform_to_transform(self.mouse_transform_override);
        self.apply_mouse_transform_with_override(frame_transform);
        Ok(())
    }
}

impl OverlayBackend for ScreenBackend {
    fn init(&mut self, _app: &mut AppState) -> anyhow::Result<()> {
        Ok(())
    }
    fn should_render(&mut self, app: &mut AppState) -> anyhow::Result<ShouldRender> {
        if !self.capture.is_ready() {
            let supports_dmabuf = app
                .gfx
                .device
                .enabled_extensions()
                .ext_external_memory_dma_buf
                && self.capture.supports_dmbuf();

            let capture_method = app.session.config.capture_method;

            let allow_dmabuf = !matches!(
                capture_method,
                CaptureMethod::PipeWireCpu | CaptureMethod::ScreenCopyCpu
            );

            let (dmabuf_formats, dma_exporter) = if !supports_dmabuf {
                log::info!("Capture method does not support DMA-buf");
                if app.gfx_extras.queue_capture.is_none() {
                    log::warn!(
                        "Current GPU does not support multiple queues. Software capture will take place on the main thread. Expect degraded performance."
                    );
                }
                ([].as_slice(), None)
            } else if !allow_dmabuf {
                log::info!(
                    "Not using DMA-buf capture due to {}",
                    capture_method.as_ref()
                );
                if app.gfx_extras.queue_capture.is_none() {
                    log::warn!(
                        "Current GPU does not support multiple queues. Software capture will take place on the main thread. Expect degraded performance."
                    );
                }
                ([].as_slice(), None)
            } else {
                log::warn!(
                    "Using GPU capture. If you're having issues with screens, go to the Dashboard's Settings tab and switch 'Wayland capture method' to a CPU option!"
                );

                let dma_exporter = if matches!(self.capture_type, CaptureType::ScreenCopy) {
                    Some(MyFirstDmaExporter::new(
                        app.gfx.clone(),
                        app.gfx_extras.drm_formats.clone(),
                    ))
                } else {
                    None
                };

                (&*app.gfx_extras.drm_formats, dma_exporter)
            };

            let user_data = WlxCaptureIn::new(self.name.clone(), app, dma_exporter);
            self.capture
                .init(dmabuf_formats, user_data, receive_callback);
            self.capture.request_new_frame();
            return Ok(ShouldRender::Unable);
        }

        if let Some(frame) = self.capture.receive() {
            let stereo = self.stereo.unwrap_or(StereoMode::None);
            let mut meta = frame.get_frame_meta(&app.session.config, stereo);

            if let Some(stereo) = self.stereo {
                // Apply stereo full frame logic
                if self.stereo_full_frame {
                    match stereo {
                        StereoMode::LeftRight | StereoMode::RightLeft => {
                            meta.extent[0] /= 2;
                        }
                        StereoMode::TopBottom | StereoMode::BottomTop => {
                            meta.extent[1] /= 2;
                        }
                        _ => {}
                    }
                }
            }

            self.source_meta = Some(meta);
            self.apply_meta_state(app, meta, stereo)?;
            self.cur_frame = Some(frame);

            Ok(ShouldRender::Should)
        } else if self.cur_frame.is_some() {
            if self.crop_dirty
                && let Some(source_meta) = self.source_meta
            {
                let stereo = self.stereo.unwrap_or(StereoMode::None);
                self.apply_meta_state(app, source_meta, stereo)?;
                return Ok(ShouldRender::Should);
            }
            if self.just_resumed {
                self.just_resumed = false;
                Ok(ShouldRender::Should)
            } else {
                Ok(ShouldRender::Can)
            }
        } else {
            log::trace!("{}: backend ready, but no image received.", self.name);
            Ok(ShouldRender::Unable)
        }
    }
    fn render(&mut self, app: &mut AppState, rdr: &mut RenderResources) -> anyhow::Result<()> {
        // want panic; must be some if should_render was not Unable
        let capture = self.cur_frame.as_ref().unwrap();
        let image = capture.image.clone();

        // want panic; must be Some if cur_frame is also Some
        self.pipeline
            .as_mut()
            .unwrap()
            .render(image, capture.mouse.as_ref(), app, rdr)?;
        self.capture.request_new_frame();
        Ok(())
    }
    fn pause(&mut self, _app: &mut AppState) -> anyhow::Result<()> {
        self.capture.pause();
        Ok(())
    }
    fn resume(&mut self, _app: &mut AppState) -> anyhow::Result<()> {
        self.just_resumed = true;
        self.capture.resume();
        Ok(())
    }
    fn frame_meta(&mut self) -> Option<FrameMeta> {
        self.meta
    }

    fn notify(&mut self, _app: &mut AppState, _event_data: OverlayEventData) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_hover(&mut self, app: &mut AppState, hit: &PointerHit) -> HoverResult {
        #[cfg(debug_assertions)]
        log::trace!("Hover: {:?}", hit.uv);
        if can_move()
            && (!app.session.config.focus_follows_mouse_mode
                || app.input_state.pointers[hit.pointer].now.move_mouse)
        {
            let pos = self.mouse_transform.transform_point2(hit.uv);
            app.hid_provider.inner.mouse_move(pos);
            set_next_move(app.session.config.mouse_move_interval_ms as _);
        }
        HoverResult {
            consume: true,
            ..HoverResult::default()
        }
    }
    fn on_pointer(&mut self, app: &mut AppState, hit: &PointerHit, pressed: bool) {
        let mut btn = match hit.mode {
            PointerMode::Right => MOUSE_RIGHT,
            PointerMode::Middle => MOUSE_MIDDLE,
            _ => MOUSE_LEFT,
        };

        // Swap left and right buttons if left-handed mode is enabled
        if app.session.config.left_handed_mouse {
            btn = match btn {
                MOUSE_LEFT => MOUSE_RIGHT,
                MOUSE_RIGHT => MOUSE_LEFT,
                other => other,
            };
        }

        if pressed {
            set_next_move(app.session.config.click_freeze_time_ms as _);
        }

        app.hid_provider.inner.send_button(btn, pressed);

        if !pressed {
            return;
        }
        let pos = self.mouse_transform.transform_point2(hit.uv);
        app.hid_provider.inner.mouse_move(pos);
    }

    fn on_scroll(&mut self, app: &mut AppState, _hit: &PointerHit, delta: WheelDelta) {
        app.hid_provider.inner.wheel(delta);
    }

    fn on_left(&mut self, _app: &mut AppState, _hand: usize) {}

    fn get_interaction_transform(&mut self) -> Option<Affine2> {
        self.interaction_transform
    }
    fn get_attrib(&self, attrib: BackendAttrib) -> Option<BackendAttribValue> {
        match attrib {
            BackendAttrib::Stereo => self.stereo.map(BackendAttribValue::Stereo),
            BackendAttrib::MouseTransform => Some(BackendAttribValue::MouseTransform(
                self.mouse_transform_override,
            )),
            BackendAttrib::StereoFullFrame => {
                Some(BackendAttribValue::StereoFullFrame(self.stereo_full_frame))
            }
            BackendAttrib::StereoAdjustMouse => Some(BackendAttribValue::StereoAdjustMouse(
                self.stereo_adjust_mouse,
            )),
            BackendAttrib::CropRect => Some(BackendAttribValue::CropRect(self.crop_rect)),
            _ => None,
        }
    }
    fn set_attrib(&mut self, _app: &mut AppState, value: BackendAttribValue) -> bool {
        match value {
            BackendAttribValue::Stereo(new) => {
                if let Some(stereo) = self.stereo.as_mut() {
                    log::debug!("{}: stereo: {stereo:?} → {new:?}", self.name);
                    *stereo = new;
                    if let Some(pipeline) = self.pipeline.as_mut() {
                        pipeline.ensure_stereo(new);
                    }
                    true
                } else {
                    false
                }
            }
            BackendAttribValue::MouseTransform(new) => {
                self.mouse_transform_override = new;
                let frame_transform = mouse_transform_to_transform(new);
                self.apply_mouse_transform_with_override(frame_transform);
                true
            }
            BackendAttribValue::StereoFullFrame(new) => {
                self.stereo_full_frame = new;
                true
            }
            BackendAttribValue::StereoAdjustMouse(new) => {
                self.stereo_adjust_mouse = new;
                let frame_transform = mouse_transform_to_transform(self.mouse_transform_override);
                self.apply_mouse_transform_with_override(frame_transform);
                true
            }
            BackendAttribValue::CropRect(new) => {
                self.crop_rect = new;
                self.crop_dirty = true;
                let frame_transform = mouse_transform_to_transform(self.mouse_transform_override);
                self.apply_mouse_transform_with_override(frame_transform);
                true
            }
            _ => false,
        }
    }
}

const fn mouse_transform_to_transform(mouse_transform: MouseTransform) -> Transform {
    match mouse_transform {
        MouseTransform::Default => Transform::Undefined,
        MouseTransform::Normal => Transform::Normal,
        MouseTransform::Rotated90 => Transform::Rotated90,
        MouseTransform::Rotated180 => Transform::Rotated180,
        MouseTransform::Rotated270 => Transform::Rotated270,
        MouseTransform::Flipped => Transform::Flipped,
        MouseTransform::Flipped90 => Transform::Flipped90,
        MouseTransform::Flipped180 => Transform::Flipped180,
        MouseTransform::Flipped270 => Transform::Flipped270,
    }
}
