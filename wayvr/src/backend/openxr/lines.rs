use glam::{Affine3A, Quat, Vec3, Vec3A};
use idmap::IdMap;
use openxr as xr;
use smallvec::SmallVec;
use std::{
    f32::consts::PI,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use wgui::gfx::{
    Buffer, BufferUsage, CommandBufferUsage, Scissor, Vert2Uv,
    cmd::WGfxClearMode,
    pass::WGfxPass,
    pipeline::{WGfxPipeline, WPipelineCreateInfo},
};

use crate::{backend::openxr::helpers, graphics::GpuFutures, state::AppState};

use super::{
    CompositionLayer, XrState,
    swapchain::{SwapchainOpts, WlxSwapchain, create_swapchain},
};

static LINE_AUTO_INCREMENT: AtomicUsize = AtomicUsize::new(1);
pub(super) const LINE_WIDTH: f32 = 0.002;

// TODO customizable colors
static COLORS: [[f32; 4]; 5] = {
    [
        [1., 1., 1., 1.],
        [0., 0.375, 0.5, 1.],
        [0.69, 0.188, 0., 1.],
        [0.375, 0., 0.5, 1.],
        [1., 0., 0., 1.],
    ]
};

pub(super) struct LinePool {
    lines: IdMap<usize, LineContainer>,
    pipeline: Arc<WGfxPipeline<Vert2Uv>>,
}

impl LinePool {
    pub(super) fn new(app: &AppState) -> anyhow::Result<Self> {
        let pipeline = app.gfx.create_pipeline(
            app.gfx_extras.shaders.get("vert_quad").unwrap(), // want panic
            app.gfx_extras.shaders.get("frag_color").unwrap(), // want panic
            WPipelineCreateInfo::new(app.gfx.surface_format()),
        )?;

        Ok(Self {
            lines: IdMap::new(),
            pipeline,
        })
    }

    pub(super) fn allocate(&mut self, xr: &XrState, app: &AppState) -> anyhow::Result<usize> {
        let id = LINE_AUTO_INCREMENT.fetch_add(1, Ordering::Relaxed);

        let buf_color = app
            .gfx
            .empty_buffer(BufferUsage::TRANSFER_DST | BufferUsage::UNIFORM_BUFFER, 4)?;

        let set0 = self.pipeline.buffer(0, buf_color.clone())?;

        let pass = self.pipeline.create_pass(
            [1.0, 1.0],
            [0.0, 0.0],
            app.gfx_extras.quad_verts.clone(),
            0..4,
            0..1,
            vec![set0],
            Scissor::from_viewport([1.0, 1.0], [0.0, 0.0]),
        )?;

        let srd = create_swapchain(xr, app.gfx.clone(), [1, 1], 1, SwapchainOpts::new())?;
        self.lines.insert(
            id,
            LineContainer {
                swapchain: srd,
                buf_color,
                pass,
                maybe_line: None,
            },
        );
        Ok(id)
    }

    pub(super) fn draw_between(
        &mut self,
        id: usize,
        start: Vec3A,
        end: Vec3A,
        color: usize,
        hmd: Affine3A,
    ) {
        let dir = end - start;
        let len = dir.length();

        if len < 0.01 {
            return;
        }

        debug_assert!(color < COLORS.len());

        let Some(line) = self.lines.get_mut(id) else {
            log::warn!("Line {id} not found");
            return;
        };

        let center = (start + end) * 0.5;
        let dir_norm = dir / len;

        let xform = Affine3A::from_rotation_translation(
            Quat::from_rotation_arc(Vec3::Z, dir_norm.into()),
            center.into(),
        );

        let rotation = Affine3A::from_axis_angle(Vec3::X, PI * 1.5);
        let mut transform = xform * rotation;
        let to_hmd = hmd.translation - center;
        let sides = [Vec3A::Z, Vec3A::X, Vec3A::NEG_Z, Vec3A::NEG_X];

        #[allow(clippy::neg_multiply)]
        let rotations = [
            Affine3A::IDENTITY,
            Affine3A::from_axis_angle(Vec3::Y, PI * 0.5),
            Affine3A::from_axis_angle(Vec3::Y, PI * -1.0),
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

        let posef = helpers::transform_to_posef(&transform);

        line.maybe_line = Some(Line {
            color,
            pose: posef,
            length: len,
        });
    }

    pub(super) fn render(
        &mut self,
        app: &AppState,
        futures: &mut GpuFutures,
    ) -> anyhow::Result<()> {
        for line in self.lines.values_mut() {
            if let Some(inner) = line.maybe_line.as_mut() {
                let tgt = line
                    .swapchain
                    .acquire_wait_image()?
                    .views
                    .into_iter()
                    .next()
                    .unwrap();

                line.buf_color.write()?[0..4].copy_from_slice(&COLORS[inner.color]);

                let mut cmd_buffer = app
                    .gfx
                    .create_gfx_command_buffer(CommandBufferUsage::OneTimeSubmit)?;
                cmd_buffer.begin_rendering(tgt, WGfxClearMode::DontCare)?;
                cmd_buffer.run_ref(&line.pass)?;
                cmd_buffer.end_rendering()?;

                futures.execute(cmd_buffer.build()?);
            }
        }

        Ok(())
    }

    pub(super) fn present<'a>(
        &'a mut self,
        xr: &'a XrState,
    ) -> anyhow::Result<SmallVec<[CompositionLayer<'a>; 2]>> {
        let mut quads = SmallVec::new_const();

        for line in self.lines.values_mut() {
            line.swapchain.ensure_image_released()?;

            if let Some(inner) = line.maybe_line.take() {
                let quad = xr::CompositionLayerQuad::new()
                    .pose(inner.pose)
                    .sub_image(line.swapchain.get_subimage(0))
                    .eye_visibility(xr::EyeVisibility::BOTH)
                    .space(&xr.stage)
                    .size(xr::Extent2Df {
                        width: LINE_WIDTH,
                        height: inner.length,
                    });

                quads.push(CompositionLayer::Quad(quad));
            }
        }

        Ok(quads)
    }
}

pub(super) struct Line {
    pub(super) color: usize,
    pub(super) pose: xr::Posef,
    pub(super) length: f32,
}

struct LineContainer {
    swapchain: WlxSwapchain,
    buf_color: Arc<Buffer<f32>>,
    pass: WGfxPass<Vert2Uv>,
    maybe_line: Option<Line>,
}
