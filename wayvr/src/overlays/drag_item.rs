use std::{sync::Arc, sync::LazyLock};

use glam::{Affine2, Affine3A, Quat, Vec2, Vec3, Vec3A};
use slotmap::Key;
use smithay::{
    reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface},
    utils::{Logical, Point},
};
use wgui::gfx::{
    BLEND_ALPHA, BufferUsage, Scissor, Vert2Uv, WGfxPipeline, cmd::WGfxClearMode,
    pipeline::WPipelineCreateInfo, upload_quad_vertices,
};
use wlx_common::{
    common::LeftRight,
    overlays::{BackendAttrib, BackendAttribValue, StereoMode},
    windowing::{OverlayWindowState, Positioning},
};

use crate::{
    backend::{
        input::{HoverResult, PointerHit},
        task::{OverlayTask, TaskType},
        wayvr::hit_test::{
            RenderedSurface, collect_rendered_surface_tree_at, rendered_surfaces_dirty,
        },
    },
    state::AppState,
    subsystem::hid::WheelDelta,
    windowing::{
        OverlayID, OverlaySelector, Z_ORDER_DRAG_ITEM,
        backend::{FrameMeta, OverlayBackend, OverlayEventData, RenderResources, ShouldRender},
        manager::OverlayWindowManager,
        overlay_scale_from_extent,
        window::{OverlayCategory, OverlayWindowConfig},
    },
};

pub static DRAG_ITEM_NAME: LazyLock<Arc<str>> = LazyLock::new(|| "wayvr-drag-item".into());

const FALLBACK_EXTENT: [u32; 2] = [64, 64];
const FALLBACK_DISTANCE: f32 = 0.5;

struct DragItemBackend {
    surface: WlSurface,
    pipeline: Arc<WGfxPipeline<Vert2Uv>>,
    surfaces: Vec<RenderedSurface>,
    origin: Vec2,
    meta: Option<FrameMeta>,
    overlay_id: OverlayID,
    scaled_extent: Option<[u32; 2]>,
    force_render: bool,
}

impl DragItemBackend {
    fn new(
        app: &mut AppState,
        surface: WlSurface,
        initial_extent: Option<[u32; 2]>,
    ) -> anyhow::Result<Self> {
        let pipeline = app.gfx.create_pipeline(
            app.gfx_extras.shaders.get("vert_quad").unwrap(),
            app.gfx_extras.shaders.get("frag_simple").unwrap(),
            WPipelineCreateInfo::new(app.gfx.surface_format()).use_blend(BLEND_ALPHA),
        )?;

        Ok(Self {
            surface,
            pipeline,
            surfaces: Vec::new(),
            origin: Vec2::ZERO,
            meta: None,
            overlay_id: OverlayID::null(),
            scaled_extent: initial_extent,
            force_render: true,
        })
    }

    fn surface_bounds(surfaces: &[RenderedSurface]) -> Option<(Vec2, [u32; 2])> {
        let first = surfaces.first()?;
        let mut min = first.pos;
        let mut max = first.pos + first.size;

        for surface in &surfaces[1..] {
            min = min.min(surface.pos);
            max = max.max(surface.pos + surface.size);
        }

        let origin = Vec2::new(min.x.floor(), min.y.floor());
        let max = Vec2::new(max.x.ceil(), max.y.ceil());
        let size = (max - origin).max(Vec2::ONE);

        Some((origin, [size.x as u32, size.y as u32]))
    }

    fn update_overlay_scale(&mut self, app: &mut AppState, extent: [u32; 2]) {
        if self.scaled_extent == Some(extent) || self.overlay_id.is_null() {
            return;
        }
        self.scaled_extent = Some(extent);

        let new_scale = overlay_scale_from_extent(extent).0;
        let overlay_id = self.overlay_id;

        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
            OverlaySelector::Id(overlay_id),
            Box::new(move |_app, config| {
                fn apply_scale(transform: &mut Affine3A, new_scale: f32) {
                    let current_scale = transform.x_axis.length();
                    if current_scale > f32::EPSILON {
                        transform.matrix3 = transform.matrix3.mul_scalar(new_scale / current_scale);
                    }
                }

                apply_scale(&mut config.default_state.transform, new_scale);

                if let Some(state) = config.active_state.as_mut() {
                    apply_scale(&mut state.transform, new_scale);
                    if let Some(saved) = state.saved_transform.as_mut() {
                        apply_scale(saved, new_scale);
                    }
                }
                config.dirty = true;
            }),
        )));
    }

    fn render_surface(
        &self,
        app: &mut AppState,
        rdr: &mut RenderResources,
        surface: &RenderedSurface,
    ) -> anyhow::Result<()> {
        let meta = self.meta.as_ref().unwrap();
        let extent = [meta.extent[0] as f32, meta.extent[1] as f32];
        let pos = surface.pos - self.origin;

        let vertices = app
            .gfx
            .empty_buffer(BufferUsage::TRANSFER_DST | BufferUsage::VERTEX_BUFFER, 4)?;
        upload_quad_vertices(
            &vertices,
            extent[0],
            extent[1],
            pos.x,
            pos.y,
            surface.size.x,
            surface.size.y,
        )?;

        let set0 =
            self.pipeline
                .uniform_sampler(0, surface.image.clone(), app.gfx.texture_filter())?;
        let pass = self.pipeline.create_pass(
            extent,
            [0.0, 0.0],
            vertices,
            0..4,
            0..1,
            vec![set0],
            Scissor::from_viewport(extent, [0.0, 0.0]),
        )?;

        for cmd in &mut rdr.cmd_bufs {
            cmd.run_ref(&pass)?;
        }

        Ok(())
    }
}

impl OverlayBackend for DragItemBackend {
    fn init(&mut self, _app: &mut AppState) -> anyhow::Result<()> {
        Ok(())
    }

    fn pause(&mut self, _app: &mut AppState) -> anyhow::Result<()> {
        Ok(())
    }

    fn resume(&mut self, _app: &mut AppState) -> anyhow::Result<()> {
        self.force_render = true;
        Ok(())
    }

    fn should_render(&mut self, app: &mut AppState) -> anyhow::Result<ShouldRender> {
        let surfaces = collect_rendered_surface_tree_at(
            &self.surface,
            Point::<i32, Logical>::from((0, 0)),
            true,
        );
        let Some((origin, extent)) = Self::surface_bounds(&surfaces) else {
            self.meta = None;
            self.surfaces.clear();
            return Ok(ShouldRender::Unable);
        };

        let mut protocol_dirty = false;
        if let Some(wvr_server) = app.wvr_server.as_mut() {
            let state = &mut wvr_server.manager.state;
            let root_id = self.surface.id();
            protocol_dirty |= state.take_redraw_request(&root_id);
            protocol_dirty |= state.has_pending_frame_callbacks(&root_id);

            for surface in &surfaces {
                if surface.surface_id == root_id {
                    continue;
                }
                protocol_dirty |= state.take_redraw_request(&surface.surface_id);
                protocol_dirty |= state.has_pending_frame_callbacks(&surface.surface_id);
            }
        }

        let tree_dirty = rendered_surfaces_dirty(&self.surfaces, &surfaces);
        let extent_dirty = self.meta.as_ref().is_none_or(|meta| meta.extent != extent);
        let should_render = protocol_dirty || tree_dirty || extent_dirty || self.force_render;

        self.origin = origin;
        self.meta = Some(FrameMeta {
            extent,
            format: surfaces[0].image.format(),
            clear: WGfxClearMode::Clear([0.0, 0.0, 0.0, 0.0]),
            stereo: StereoMode::None,
            ..Default::default()
        });
        self.surfaces = surfaces;
        self.force_render = false;
        self.update_overlay_scale(app, extent);

        Ok(if should_render {
            ShouldRender::Should
        } else {
            ShouldRender::Can
        })
    }

    fn render(&mut self, app: &mut AppState, rdr: &mut RenderResources) -> anyhow::Result<()> {
        for surface in &self.surfaces {
            self.render_surface(app, rdr, surface)?;
        }

        if let Some(wvr_server) = app.wvr_server.as_mut() {
            let state = &mut wvr_server.manager.state;
            state.send_frame_callbacks_for_surface_id(&self.surface.id());
            for surface in &self.surfaces {
                state.send_frame_callbacks_for_surface_id(&surface.surface_id);
            }
        }

        Ok(())
    }

    fn frame_meta(&mut self) -> Option<FrameMeta> {
        self.meta
    }

    fn notify(&mut self, _app: &mut AppState, event_data: OverlayEventData) -> anyhow::Result<()> {
        if let OverlayEventData::IdAssigned(id) = event_data {
            self.overlay_id = id;
        }
        Ok(())
    }

    fn on_hover(&mut self, _app: &mut AppState, _hit: &PointerHit) -> HoverResult {
        HoverResult::default()
    }

    fn on_left(&mut self, _app: &mut AppState, _pointer: usize) {}

    fn on_pointer(&mut self, _app: &mut AppState, _hit: &PointerHit, _pressed: bool) {}

    fn on_scroll(&mut self, _app: &mut AppState, _hit: &PointerHit, _delta: WheelDelta) {}

    fn get_interaction_transform(&mut self) -> Option<Affine2> {
        None
    }

    fn get_attrib(&self, _attrib: BackendAttrib) -> Option<BackendAttribValue> {
        None
    }

    fn set_attrib(&mut self, _app: &mut AppState, _value: BackendAttribValue) -> bool {
        false
    }
}

pub fn create_drag_item(
    app: &mut AppState,
    surface: WlSurface,
    pointer: usize,
) -> anyhow::Result<OverlayWindowConfig> {
    let pointer = pointer.min(app.input_state.pointers.len() - 1);
    let hand = match pointer {
        0 => LeftRight::Left,
        _ => LeftRight::Right,
    };

    let surfaces =
        collect_rendered_surface_tree_at(&surface, Point::<i32, Logical>::from((0, 0)), true);
    let initial_extent = DragItemBackend::surface_bounds(&surfaces).map(|(_, extent)| extent);
    let extent = initial_extent.unwrap_or(FALLBACK_EXTENT);
    let scale = overlay_scale_from_extent(extent).0;
    let distance = app.input_state.pointers[pointer]
        .last_wvr_hit_distance
        .unwrap_or(FALLBACK_DISTANCE)
        .max(0.05);

    Ok(OverlayWindowConfig {
        name: DRAG_ITEM_NAME.clone(),
        default_state: OverlayWindowState {
            positioning: Positioning::FollowHand { hand, lerp: 1.0 },
            transform: Affine3A::from_scale_rotation_translation(
                Vec3::ONE * scale,
                Quat::IDENTITY,
                Vec3::NEG_Z * distance,
            ),
            alpha: 1.0,
            grabbable: false,
            interactable: false,
            block_input: false,
            align_to_hmd: true,
            ..OverlayWindowState::default()
        },
        category: OverlayCategory::Internal,
        show_on_spawn: true,
        global: true,
        z_order: Z_ORDER_DRAG_ITEM,
        ..OverlayWindowConfig::from_backend(Box::new(DragItemBackend::new(
            app,
            surface,
            initial_extent,
        )?))
    })
}

pub fn update_position<O>(overlays: &mut OverlayWindowManager<O>, app: &AppState)
where
    O: Default,
{
    let Some(overlay) = overlays.mut_by_selector(&OverlaySelector::Name(DRAG_ITEM_NAME.clone()))
    else {
        return;
    };
    let Some(state) = overlay.config.active_state.as_ref() else {
        return;
    };
    let Positioning::FollowHand { hand, .. } = state.positioning else {
        return;
    };

    let pointer = &app.input_state.pointers[hand as usize];
    if !pointer.tracked {
        return;
    }
    let Some(distance) = pointer.last_wvr_hit_distance else {
        return;
    };

    let local_target = Vec3A::NEG_Z * distance;
    overlay.config.default_state.transform.translation = local_target;

    let state = overlay.config.active_state.as_mut().unwrap();
    if let Some(saved) = state.saved_transform.as_mut() {
        saved.translation = local_target;
    }

    let target = pointer.pose.transform_point3a(local_target);
    if state.transform.translation.distance_squared(target) > 1.0e-8 {
        state.transform.translation = target;
        overlay.config.dirty = true;
    }
}
