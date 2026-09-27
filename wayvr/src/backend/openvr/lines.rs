use std::f32::consts::PI;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use glam::{Affine3A, Quat, Vec3, Vec3A, Vec4};
use idmap::IdMap;
use ovr_overlay::overlay::OverlayManager;
use ovr_overlay::sys::ETrackingUniverseOrigin;
use wgui::gfx::{CommandBufferUsage, Format, ImageView, WGfx};
use wlx_common::overlays::{BackendAttrib, BackendAttribValue};

use crate::backend::input::{HoverResult, PointerHit};
use crate::state::AppState;
use crate::subsystem::hid::WheelDelta;
use crate::windowing::Z_ORDER_LINES;
use crate::windowing::backend::{
    FrameMeta, OverlayBackend, OverlayEventData, RenderResources, ShouldRender,
};
use crate::windowing::window::{OverlayWindowConfig, OverlayWindowData};

use super::overlay::OpenVrOverlayData;

static LINE_AUTO_INCREMENT: AtomicUsize = AtomicUsize::new(1);

pub(super) struct LinePool {
    lines: IdMap<usize, OverlayWindowData<OpenVrOverlayData>>,
    view: Arc<ImageView>,
    colors: [Vec4; 5],
}

impl LinePool {
    pub fn new(graphics: Arc<WGfx>) -> anyhow::Result<Self> {
        let mut command_buffer =
            graphics.create_xfer_command_buffer(CommandBufferUsage::OneTimeSubmit)?;

        let buf = vec![255; 16];

        let texture = command_buffer.upload_image(2, 2, Format::R8G8B8A8_UNORM, &buf)?;
        command_buffer.build_and_execute_now()?;

        let view = graphics.create_image_view(texture)?;

        Ok(Self {
            lines: IdMap::new(),
            view,
            colors: [
                Vec4::new(1., 1., 1., 1.),
                Vec4::new(0., 0.375, 0.5, 1.),
                Vec4::new(0.69, 0.188, 0., 1.),
                Vec4::new(0.375, 0., 0.5, 1.),
                Vec4::new(1., 0., 0., 1.),
            ],
        })
    }

    pub fn allocate(&mut self) -> usize {
        let id = LINE_AUTO_INCREMENT.fetch_add(1, Ordering::Relaxed);

        let data = OverlayWindowData::<OpenVrOverlayData> {
            data: OpenVrOverlayData {
                width: 0.002,
                override_width: true,
                image_view: Some(self.view.clone()),
                image_dirty: true,
                ..Default::default()
            },
            ..OverlayWindowData::from_config(OverlayWindowConfig {
                name: Arc::from(format!("wlx-line{id}")),
                default_state: Default::default(),
                active_state: Some(Default::default()),
                z_order: Z_ORDER_LINES,
                global: true,
                ..OverlayWindowConfig::from_backend(Box::new(LineBackend {
                    view: self.view.clone(),
                }))
            })
        };

        self.lines.insert(id, data);
        id
    }

    pub(super) fn draw_between(
        &mut self,
        id: usize,
        start: Vec3A,
        end: Vec3A,
        color: usize,
        hmd: &Affine3A,
    ) {
        let dir = end - start;
        let len = dir.length();

        if len < 0.01 {
            return;
        }

        debug_assert!(color < self.colors.len());

        let center = (start + end) * 0.5;
        let dir_norm = dir / len;

        let xform = Affine3A::from_rotation_translation(
            Quat::from_rotation_arc(Vec3::Z, dir_norm.into()),
            center.into(),
        );

        let rotation = Affine3A::from_axis_angle(Vec3::X, -PI * 0.5);
        let mut transform = xform * rotation * Affine3A::from_scale(Vec3::new(1., len / 0.002, 1.));
        let to_hmd = hmd.translation - center;
        let sides = [Vec3A::Z, Vec3A::X, Vec3A::NEG_Z, Vec3A::NEG_X];

        let rotations = [
            Affine3A::IDENTITY,
            Affine3A::from_axis_angle(Vec3::Y, PI * 0.5),
            Affine3A::from_axis_angle(Vec3::Y, PI * 1.0),
            Affine3A::from_axis_angle(Vec3::Y, PI * 1.5),
        ];

        let mut closest = (0, 0.0);
        for (i, &side) in sides.iter().enumerate() {
            let dot = to_hmd.dot(transform.transform_vector3a(side));
            if i == 0 || dot > closest.1 {
                closest = (i, dot);
            }
        }

        transform *= rotations[closest.0];

        self.draw_transform(id, transform, self.colors[color]);
    }

    fn draw_transform(&mut self, id: usize, transform: Affine3A, color: Vec4) {
        if let Some(data) = self.lines.get_mut(id) {
            let state = data.config.active_state.as_mut().unwrap();
            state.alpha = 1.0;
            state.transform = transform;
            data.data.color = color;
        } else {
            log::warn!("Line {id} does not exist");
        }
    }

    pub fn update(
        &mut self,
        universe: ETrackingUniverseOrigin,
        overlay: &mut OverlayManager,
        app: &mut AppState,
    ) -> anyhow::Result<()> {
        for data in self.lines.values_mut() {
            data.after_input(overlay, app)?;
            let state = data.config.active_state.as_mut().unwrap();
            if state.alpha > 0.05 {
                if data.config.dirty {
                    data.upload_texture(overlay, &app.gfx);
                    data.config.dirty = false;
                }

                data.upload_transform(universe.clone(), overlay);
                data.upload_color(overlay);
            }
        }
        Ok(())
    }

    pub fn mark_dirty(&mut self) {
        for data in self.lines.values_mut() {
            data.config.dirty = true;
        }
    }

    pub fn reset(&mut self) {
        for data in self.lines.values_mut() {
            let state = data.config.active_state.as_mut().unwrap();
            state.alpha = 0.0;
        }
    }
}

struct LineBackend {
    view: Arc<ImageView>,
}

impl OverlayBackend for LineBackend {
    fn init(&mut self, _: &mut AppState) -> anyhow::Result<()> {
        Ok(())
    }
    fn pause(&mut self, _: &mut AppState) -> anyhow::Result<()> {
        Ok(())
    }
    fn resume(&mut self, _: &mut AppState) -> anyhow::Result<()> {
        Ok(())
    }
    fn should_render(&mut self, _: &mut AppState) -> anyhow::Result<ShouldRender> {
        Ok(ShouldRender::Unable)
    }
    fn render(&mut self, _: &mut AppState, _: &mut RenderResources) -> anyhow::Result<()> {
        unreachable!()
    }
    fn frame_meta(&mut self) -> Option<FrameMeta> {
        Some(FrameMeta {
            extent: self.view.extent_2d(),
            ..Default::default()
        })
    }

    fn notify(&mut self, _: &mut AppState, _: OverlayEventData) -> anyhow::Result<()> {
        unreachable!()
    }

    fn on_hover(&mut self, _: &mut AppState, _: &PointerHit) -> HoverResult {
        HoverResult::default()
    }
    fn on_left(&mut self, _: &mut AppState, _: usize) {}
    fn on_pointer(&mut self, _: &mut AppState, _: &PointerHit, _: bool) {}
    fn on_scroll(&mut self, _: &mut AppState, _: &PointerHit, _delta: WheelDelta) {}
    fn get_interaction_transform(&mut self) -> Option<glam::Affine2> {
        None
    }
    fn get_attrib(&self, _attrib: BackendAttrib) -> Option<BackendAttribValue> {
        None
    }
    fn set_attrib(&mut self, _: &mut AppState, _value: BackendAttribValue) -> bool {
        false
    }
}
