use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use glam::{Affine2, Affine3A, Quat, Vec3};
use slotmap::Key;
use vulkano::buffer::BufferUsage;
use wgui::{
    color::WguiColorName,
    gfx::pipeline::{WGfxPipeline, WPipelineCreateInfo},
};
use wlx_common::{
    overlays::{BackendAttrib, BackendAttribValue},
    windowing::{OverlayWindowState, Positioning},
};

use crate::{
    backend::task::{OverlayTask, TaskType},
    graphics::Vert2Uv,
    state::AppState,
    windowing::{
        OverlayID, OverlaySelector,
        backend::{FrameMeta, OverlayBackend, OverlayEventData, ShouldRender, ui_transform},
        overlay_scale_from_extent,
        window::{OverlayCategory, OverlayWindowConfig},
    },
};

static PASSTHRU_COUNTER: AtomicUsize = AtomicUsize::new(1);

pub fn new_passtrhu_name() -> Arc<str> {
    format!("P-{}", PASSTHRU_COUNTER.fetch_add(1, Ordering::Relaxed)).into()
}

pub fn new_passthru(name: Arc<str>, app: &AppState) -> OverlayWindowConfig {
    OverlayWindowConfig {
        name,
        default_state: OverlayWindowState {
            transform: Affine3A::from_scale_rotation_translation(
                Vec3::ONE * 0.5,
                Quat::IDENTITY,
                Vec3::new(0., 0., -0.5),
            ),
            grabbable: true,
            interactable: true,
            positioning: Positioning::Static,
            ..OverlayWindowState::default()
        },
        global: true,
        show_on_spawn: true,
        category: OverlayCategory::Passthru,
        ..OverlayWindowConfig::from_backend(Box::new(PassthruBackend::new(app)))
    }
}

struct PassthruBackend {
    frame_meta: FrameMeta,
    rendered: bool,
    interaction_transform: Affine2,
    scale: f32,
    overlay_id: OverlayID,
}

const DEFAULT_EXTENT: [u32; 2] = [512, 512];

impl PassthruBackend {
    fn new(app: &AppState) -> Self {
        let (scale, _) = overlay_scale_from_extent(DEFAULT_EXTENT);
        Self {
            frame_meta: FrameMeta {
                extent: DEFAULT_EXTENT,
                format: app.gfx.surface_format,
                ..Default::default()
            },
            interaction_transform: ui_transform(DEFAULT_EXTENT),
            rendered: false,
            scale,
            overlay_id: OverlayID::null(),
        }
    }
}

impl OverlayBackend for PassthruBackend {
    fn init(&mut self, _app: &mut AppState) -> anyhow::Result<()> {
        Ok(())
    }
    fn pause(&mut self, _app: &mut AppState) -> anyhow::Result<()> {
        Ok(())
    }
    fn resume(&mut self, _app: &mut AppState) -> anyhow::Result<()> {
        Ok(())
    }
    fn should_render(&mut self, _app: &mut AppState) -> anyhow::Result<ShouldRender> {
        Ok(if self.rendered {
            ShouldRender::Can
        } else {
            ShouldRender::Should
        })
    }
    fn render(
        &mut self,
        app: &mut AppState,
        rdr: &mut crate::windowing::backend::RenderResources,
    ) -> anyhow::Result<()> {
        // this is heavy, but only done once
        let pipeline: Arc<WGfxPipeline<Vert2Uv>> = app.gfx.create_pipeline(
            app.gfx_extras.shaders.get("vert_quad").unwrap(), // want panic
            app.gfx_extras.shaders.get("frag_color").unwrap(), // want panic
            WPipelineCreateInfo::new(app.gfx.surface_format),
        )?;

        let color = WguiColorName::Primary
            .to_wgui_color()
            .resolve(&app.wgui_globals.get().palette);

        let buf_color = app.gfx.new_buffer(
            BufferUsage::TRANSFER_DST | BufferUsage::UNIFORM_BUFFER,
            color.with_alpha(1.0).as_arr().iter(),
        )?;

        let set0 = pipeline.buffer(0, buf_color.clone())?;
        let extentf32 = [
            self.frame_meta.extent[0] as f32,
            self.frame_meta.extent[1] as f32,
        ];

        let pass = pipeline.create_pass(
            extentf32,
            [0.0, 0.0],
            app.gfx_extras.quad_verts.clone(),
            0..4,
            0..1,
            vec![set0],
            &Default::default(),
        )?;

        rdr.cmd_buf_single().run_ref(&pass)?;

        let buf_color = app.gfx.new_buffer(
            BufferUsage::TRANSFER_DST | BufferUsage::UNIFORM_BUFFER,
            [0.0, 0.0, 0.0, 1.0].iter(),
        )?;

        let set0 = pipeline.buffer(0, buf_color.clone())?;

        let pass = pipeline.create_pass(
            [extentf32[0] - 8.0, extentf32[1] - 8.0],
            [4.0, 4.0],
            app.gfx_extras.quad_verts.clone(),
            0..4,
            0..1,
            vec![set0],
            &Default::default(),
        )?;

        rdr.cmd_buf_single().run_ref(&pass)?;

        //self.rendered = true;
        Ok(())
    }
    fn frame_meta(&mut self) -> Option<FrameMeta> {
        Some(self.frame_meta)
    }
    fn notify(&mut self, app: &mut AppState, event_data: OverlayEventData) -> anyhow::Result<()> {
        match event_data {
            OverlayEventData::IdAssigned(id) => self.overlay_id = id,
            OverlayEventData::ResizeRequest(new_size) => {
                self.frame_meta.extent =
                    [new_size[0].clamp(256, 1024), new_size[1].clamp(256, 1024)];
                self.interaction_transform = ui_transform(self.frame_meta.extent);

                let old_scale = self.scale;
                let (new_scale, _) = overlay_scale_from_extent(self.frame_meta.extent);
                self.scale = new_scale;

                if new_scale.abs() > f32::EPSILON {
                    let scale_delta = new_scale / old_scale;

                    if (scale_delta - 1.0).abs() > f32::EPSILON {
                        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
                            OverlaySelector::Id(self.overlay_id),
                            Box::new(move |_app, owc| {
                                if let Some(state) = owc.active_state.as_mut() {
                                    if let Some(saved) = state.saved_transform.as_mut() {
                                        saved.matrix3 = saved.matrix3.mul_scalar(scale_delta);
                                    }

                                    state.transform.matrix3 =
                                        state.transform.matrix3.mul_scalar(scale_delta);
                                }
                            }),
                        )));
                    }
                }
                self.rendered = false;
            }
            _ => {}
        }
        Ok(())
    }
    fn on_hover(
        &mut self,
        _app: &mut AppState,
        _hit: &crate::backend::input::PointerHit,
    ) -> crate::backend::input::HoverResult {
        crate::backend::input::HoverResult::consume()
    }
    fn on_left(&mut self, _app: &mut AppState, _pointer: usize) {}
    fn on_pointer(
        &mut self,
        _app: &mut AppState,
        _hit: &crate::backend::input::PointerHit,
        _pressed: bool,
    ) {
    }
    fn on_scroll(
        &mut self,
        _app: &mut AppState,
        _hit: &crate::backend::input::PointerHit,
        _delta: crate::subsystem::hid::WheelDelta,
    ) {
    }
    fn get_interaction_transform(&mut self) -> Option<Affine2> {
        Some(self.interaction_transform)
    }
    fn get_attrib(
        &self,
        attrib: wlx_common::overlays::BackendAttrib,
    ) -> Option<wlx_common::overlays::BackendAttribValue> {
        match attrib {
            BackendAttrib::Resizable => Some(BackendAttribValue::Resizable(true)),
            _ => None,
        }
    }
    fn set_attrib(
        &mut self,
        _app: &mut AppState,
        _value: wlx_common::overlays::BackendAttribValue,
    ) -> bool {
        false
    }
}
