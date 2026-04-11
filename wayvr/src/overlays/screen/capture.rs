use std::{
    f32::consts::PI,
    os::fd::AsRawFd,
    sync::{Arc, OnceLock},
};

use glam::{Affine3A, Vec3};
use smallvec::{SmallVec, smallvec};
use vulkano::{
    buffer::{BufferUsage, Subbuffer},
    command_buffer::CommandBufferUsage,
    device::Queue,
    format::Format,
    image::{Image, sampler::Filter, view::ImageView},
    memory::{ExternalMemoryHandleTypes, allocator::MemoryAllocator},
    pipeline::graphics::color_blend::AttachmentBlend,
};
use wgui::{
    gfx::{
        WGfx,
        cmd::WGfxClearMode,
        memory_allocator,
        pass::WGfxPass,
        pipeline::{WGfxPipeline, WPipelineCreateInfo},
    },
    log::LogErr,
};
use wlx_capture::{
    DrmFormat, DrmFourcc, DrmModifier, WlxCapture,
    frame::{self as wlx_frame, DmaExporter, FrameFormat, MouseMeta, WlxFrame},
};
use wlx_common::{config::GeneralConfig, overlays::StereoMode};

use crate::{
    graphics::{
        ExtentExt, Vert2Uv,
        dmabuf::{ExportedDmabufImage, WGfxDmabuf, export_dmabuf_image, fourcc_to_vk},
        upload_quad_vertices,
    },
    state::AppState,
    windowing::backend::{FrameMeta, RenderResources},
};

const CURSOR_SIZE: f32 = 16. / 1440.;

struct BufPass {
    pass: WGfxPass<Vert2Uv>,
    buf_vert: Subbuffer<[Vert2Uv]>,
}

/// A render pipeline that supports mouse + stereo
pub struct ScreenPipeline {
    mouse: BufPass,
    pass: SmallVec<[BufPass; 2]>,
    pipeline: Arc<WGfxPipeline<Vert2Uv>>,
    extentf: [f32; 2],
    source_extentf: [f32; 2],
    offsetf: [f32; 2],
    crop_rect: [f32; 4],
    stereo: StereoMode,
}

impl ScreenPipeline {
    pub fn new(
        meta: &FrameMeta,
        source_extent: [u32; 2],
        app: &mut AppState,
        stereo: StereoMode,
        offsetf: [f32; 2],
        crop_rect: [f32; 4],
    ) -> anyhow::Result<Self> {
        let extentf = [meta.extent[0] as f32, meta.extent[1] as f32];
        let source_extentf = [source_extent[0] as f32, source_extent[1] as f32];

        let pipeline = app.gfx.create_pipeline(
            app.gfx_extras.shaders.get("vert_quad").unwrap(), // want panic
            app.gfx_extras.shaders.get("frag_screen").unwrap(), // want panic
            WPipelineCreateInfo::new(app.gfx.surface_format)
                .use_blend(AttachmentBlend::default())
                .use_updatable_descriptors(smallvec![0]),
        )?;

        let mut me = Self {
            pass: smallvec![Self::create_pass(app, pipeline.clone(), extentf, offsetf,)?],
            mouse: Self::create_mouse_pass(app, pipeline.clone(), extentf, offsetf)?,
            pipeline,
            extentf,
            source_extentf,
            offsetf,
            crop_rect,
            stereo,
        };
        me.ensure_stereo(stereo);
        Ok(me)
    }

    pub fn ensure_stereo(&mut self, stereo: StereoMode) {
        if self.stereo == stereo {
            return;
        }

        self.stereo = stereo;
        self.pass.clear(); // ensure_depth will repopulate
    }

    fn ensure_depth(&mut self, app: &mut AppState, depth: usize) -> anyhow::Result<()> {
        while self.pass.len() < depth {
            self.pass.push(Self::create_pass(
                app,
                self.pipeline.clone(),
                self.extentf,
                self.offsetf,
            )?);
        }

        while self.pass.len() > depth {
            self.pass.pop();
        }

        let stereo = self.stereo;
        let crop_rect = self.crop_rect;
        for (eye, current) in self.pass.iter_mut().enumerate() {
            let verts = cropped_stereo_mode_to_verts(stereo, crop_rect, eye);
            current.buf_vert.write()?.copy_from_slice(&verts);
        }
        Ok(())
    }

    pub fn set_extent(
        &mut self,
        app: &mut AppState,
        extentf: [f32; 2],
        offsetf: [f32; 2],
        source_extentf: [f32; 2],
    ) -> anyhow::Result<()> {
        self.extentf = extentf;
        self.source_extentf = source_extentf;
        self.offsetf = offsetf;
        self.pass.clear();

        self.mouse = Self::create_mouse_pass(app, self.pipeline.clone(), extentf, offsetf)?;
        Ok(())
    }

    pub fn set_crop_rect(&mut self, crop_rect: [f32; 4]) -> anyhow::Result<()> {
        self.crop_rect = crop_rect;
        let stereo = self.stereo;
        for (eye, current) in self.pass.iter_mut().enumerate() {
            let verts = cropped_stereo_mode_to_verts(stereo, crop_rect, eye);
            current.buf_vert.write()?.copy_from_slice(&verts);
        }
        Ok(())
    }

    fn create_pass(
        app: &mut AppState,
        pipeline: Arc<WGfxPipeline<Vert2Uv>>,
        extentf: [f32; 2],
        offsetf: [f32; 2],
    ) -> anyhow::Result<BufPass> {
        let set0 = pipeline.uniform_sampler(
            0,
            app.gfx_extras.fallback_image.clone(),
            app.gfx.texture_filter,
        )?;
        let buf_vert = app
            .gfx
            .empty_buffer(BufferUsage::TRANSFER_DST | BufferUsage::VERTEX_BUFFER, 4)?;

        let pass = pipeline.create_pass(
            extentf,
            offsetf,
            buf_vert.clone(),
            0..4,
            0..1,
            vec![set0],
            &Default::default(),
        )?;

        Ok(BufPass { pass, buf_vert })
    }

    fn create_mouse_pass(
        app: &mut AppState,
        pipeline: Arc<WGfxPipeline<Vert2Uv>>,
        extentf: [f32; 2],
        offsetf: [f32; 2],
    ) -> anyhow::Result<BufPass> {
        #[rustfmt::skip]
        let mouse_bytes = [
            0x00, 0x00, 0x00, 0xff,  0x00, 0x00, 0x00, 0xff,  0x00, 0x00, 0x00, 0xff,  0x00, 0x00, 0x00, 0xff,
            0x00, 0x00, 0x00, 0xff,  0xff, 0xff, 0xff, 0xff,  0xff, 0xff, 0xff, 0xff,  0x00, 0x00, 0x00, 0xff,
            0x00, 0x00, 0x00, 0xff,  0xff, 0xff, 0xff, 0xff,  0xff, 0xff, 0xff, 0xff,  0x00, 0x00, 0x00, 0xff,
            0x00, 0x00, 0x00, 0xff,  0x00, 0x00, 0x00, 0xff,  0x00, 0x00, 0x00, 0xff,  0x00, 0x00, 0x00, 0xff,
        ];

        let mut cmd_xfer = app
            .gfx
            .create_xfer_command_buffer(CommandBufferUsage::OneTimeSubmit)?;

        let image =
            cmd_xfer.upload_image(4, 4, vulkano::format::Format::R8G8B8A8_UNORM, &mouse_bytes)?;

        let view = ImageView::new_default(image)?;

        let buf_vert = cmd_xfer
            .graphics
            .empty_buffer(BufferUsage::TRANSFER_DST | BufferUsage::VERTEX_BUFFER, 4)?;

        let set0 = pipeline.uniform_sampler(0, view, Filter::Nearest)?;
        let pass = pipeline.create_pass(
            extentf,
            offsetf,
            buf_vert.clone(),
            0..4,
            0..1,
            vec![set0],
            &Default::default(),
        )?;

        cmd_xfer.build_and_execute_now()?;
        Ok(BufPass { pass, buf_vert })
    }

    pub fn render(
        &mut self,
        image: Arc<ImageView>,
        mouse: Option<&MouseMeta>,
        app: &mut AppState,
        rdr: &mut RenderResources,
    ) -> anyhow::Result<()> {
        self.ensure_depth(app, rdr.cmd_bufs.len())?;

        for (eye, cmd_buf) in rdr.cmd_bufs.iter_mut().enumerate() {
            let current = &mut self.pass[eye];

            current
                .pass
                .update_sampler(0, image.clone(), app.gfx.texture_filter)?;

            cmd_buf.run_ref(&current.pass)?;

            if let Some(mouse) = mouse.as_ref() {
                let [crop_x, crop_y, crop_w, crop_h] = self.crop_rect;
                if mouse.x < crop_x
                    || mouse.x > crop_x + crop_w
                    || mouse.y < crop_y
                    || mouse.y > crop_y + crop_h
                {
                    continue;
                }

                let local_mouse_x = (mouse.x - crop_x) / crop_w;
                let local_mouse_y = (mouse.y - crop_y) / crop_h;
                let size = CURSOR_SIZE * self.source_extentf[1] / crop_h.max(0.01);
                let half_size = size * 0.5;

                upload_quad_vertices(
                    &mut self.mouse.buf_vert,
                    self.extentf[0],
                    self.extentf[1],
                    local_mouse_x.mul_add(self.extentf[0], -half_size),
                    local_mouse_y.mul_add(self.extentf[1], -half_size),
                    size,
                    size,
                )?;

                cmd_buf.run_ref(&self.mouse.pass)?;
            }
        }

        Ok(())
    }
}

fn cropped_stereo_mode_to_verts(
    stereo: StereoMode,
    crop_rect: [f32; 4],
    array_index: usize,
) -> [Vert2Uv; 4] {
    let [crop_x, crop_y, crop_w, crop_h] = crop_rect;
    stereo_mode_to_verts(stereo, array_index).map(|vert| Vert2Uv {
        in_pos: vert.in_pos,
        in_uv: [
            crop_x + vert.in_uv[0] * crop_w,
            crop_y + vert.in_uv[1] * crop_h,
        ],
    })
}

fn stereo_mode_to_verts(stereo: StereoMode, array_index: usize) -> [Vert2Uv; 4] {
    let eye = match stereo {
        StereoMode::RightLeft | StereoMode::BottomTop => (1 - array_index) as f32,
        _ => array_index as f32,
    };

    match stereo {
        StereoMode::None => [
            Vert2Uv {
                in_pos: [0., 0.],
                in_uv: [0., 0.],
            },
            Vert2Uv {
                in_pos: [1., 0.],
                in_uv: [1., 0.],
            },
            Vert2Uv {
                in_pos: [0., 1.],
                in_uv: [0., 1.],
            },
            Vert2Uv {
                in_pos: [1., 1.],
                in_uv: [1., 1.],
            },
        ],
        StereoMode::LeftRight | StereoMode::RightLeft => [
            Vert2Uv {
                in_pos: [0., 0.],
                in_uv: [eye * 0.5, 0.],
            },
            Vert2Uv {
                in_pos: [1., 0.],
                in_uv: [0.5 + eye * 0.5, 0.],
            },
            Vert2Uv {
                in_pos: [0., 1.],
                in_uv: [eye * 0.5, 1.],
            },
            Vert2Uv {
                in_pos: [1., 1.],
                in_uv: [0.5 + eye * 0.5, 1.],
            },
        ],
        StereoMode::TopBottom | StereoMode::BottomTop => [
            Vert2Uv {
                in_pos: [0., 0.],
                in_uv: [0., eye * 0.5],
            },
            Vert2Uv {
                in_pos: [1., 0.],
                in_uv: [1., eye * 0.5],
            },
            Vert2Uv {
                in_pos: [0., 1.],
                in_uv: [0., 0.5 + eye * 0.5],
            },
            Vert2Uv {
                in_pos: [1., 1.],
                in_uv: [1., 0.5 + eye * 0.5],
            },
        ],
    }
}

static DMA_ALLOCATOR: OnceLock<Arc<dyn MemoryAllocator>> = OnceLock::new();

pub(super) struct MyFirstDmaExporter {
    gfx: Arc<WGfx>,
    drm_formats: Arc<[DrmFormat]>,
    images: SmallVec<[ExportedDmabufImage; 2]>,
    fourcc: DrmFourcc,
    current: usize,
}

impl MyFirstDmaExporter {
    pub(super) fn new(gfx: Arc<WGfx>, drm_formats: Arc<[DrmFormat]>) -> Self {
        Self {
            gfx,
            drm_formats,
            images: smallvec![],
            fourcc: DrmFourcc::Argb8888,
            current: 0,
        }
    }

    fn get_current(&self) -> Option<(Arc<ImageView>, FrameFormat)> {
        let image = self.images.get(self.current)?;
        let extent = image.view.extent_u32arr();
        Some((
            image.view.clone(),
            FrameFormat {
                width: extent[0],
                height: extent[1],
                drm_format: DrmFormat {
                    code: self.fourcc,
                    modifier: image.modifier,
                },
                transform: wlx_frame::Transform::Undefined,
            },
        ))
    }

    fn set_format(
        &mut self,
        width: u32,
        height: u32,
        fourcc: wlx_capture::DrmFourcc,
    ) -> Option<()> {
        if let Some(image) = self.images.first() {
            let extent = image.view.image().extent();
            if self.fourcc == fourcc && extent[0] == width && extent[1] == height {
                return Some(());
            }
        }
        self.images.clear();

        let Some(modifier) = self
            .drm_formats
            .iter()
            .filter(|f| f.code == fourcc)
            .map(|f| f.modifier)
            .next()
        else {
            log::error!("Unsupported format requested: {fourcc}");
            return None;
        };

        let format = fourcc_to_vk(fourcc)
            .log_err("Could not export new dmabuf due to invalid format")
            .ok()?;

        let allocator = DMA_ALLOCATOR.get_or_init(|| {
            memory_allocator(
                self.gfx.device.clone(),
                Some(ExternalMemoryHandleTypes::DMA_BUF),
            )
        });

        for _ in 0..2 {
            let image =
                export_dmabuf_image(allocator.clone(), [width, height, 1], format, modifier)
                    .log_err(&format!(
                        "Could not export DMA-buf image {width}x{height} {fourcc} {modifier:?}"
                    ))
                    .ok()?;

            self.images.push(image);
        }

        Some(())
    }

    fn next_frame(&mut self) -> Option<(wlx_frame::FramePlane, DrmModifier)> {
        self.current = 1 - self.current;
        let image = self.images.get(self.current)?;

        Some((
            wlx_frame::FramePlane {
                fd: Some(image.fd.as_raw_fd()),
                offset: image.offset,
                stride: image.stride,
            },
            image.modifier,
        ))
    }
}

pub struct WlxCaptureIn {
    name: Arc<str>,
    gfx: Arc<WGfx>,
    queue: Arc<Queue>,
    dma_exporter: Option<MyFirstDmaExporter>,
}

impl WlxCaptureIn {
    pub(super) fn new(
        name: Arc<str>,
        app: &AppState,
        dma_exporter: Option<MyFirstDmaExporter>,
    ) -> Self {
        Self {
            name,
            gfx: app.gfx.clone(),
            queue: app
                .gfx_extras
                .queue_capture
                .as_ref()
                .unwrap_or_else(|| &app.gfx.queue_xfer)
                .clone(),
            dma_exporter,
        }
    }
}

impl DmaExporter for WlxCaptureIn {
    fn next_frame(
        &mut self,
        width: u32,
        height: u32,
        fourcc: DrmFourcc,
    ) -> Option<(wlx_frame::FramePlane, DrmModifier)> {
        let dma_exporter = self.dma_exporter.as_mut()?;
        dma_exporter.set_format(width, height, fourcc)?;
        dma_exporter.next_frame()
    }
}

#[derive(Clone)]
pub(super) struct WlxCaptureOut {
    pub(super) image: Arc<ImageView>,
    pub(super) format: FrameFormat,
    pub(super) mouse: Option<MouseMeta>,
}

impl WlxCaptureOut {
    pub(super) fn get_frame_meta(&self, config: &GeneralConfig, stereo: StereoMode) -> FrameMeta {
        FrameMeta {
            clear: WGfxClearMode::DontCare,
            extent: extent_from_format(self.format, config),
            transform: affine_from_format(&self.format),
            format: self.image.format(),
            stereo,
        }
    }
}

fn upload_image(
    me: &WlxCaptureIn,
    width: u32,
    height: u32,
    format: Format,
    data: &[u8],
) -> Option<Arc<Image>> {
    let mut cmd_xfer = match me
        .gfx
        .create_xfer_command_buffer_with_queue(me.queue.clone(), CommandBufferUsage::OneTimeSubmit)
    {
        Ok(x) => x,
        Err(e) => {
            log::error!("{}: Could not create vkCommandBuffer: {:?}", me.name, e);
            return None;
        }
    };
    let image = match cmd_xfer.upload_image(width, height, format, data) {
        Ok(x) => x,
        Err(e) => {
            log::error!("{}: Could not create vkImage: {:?}", me.name, e);
            return None;
        }
    };

    if let Err(e) = cmd_xfer.build_and_execute_now() {
        log::error!("{}: Could not execute upload: {:?}", me.name, e);
        return None;
    }

    Some(image)
}

pub(super) fn receive_callback(me: &WlxCaptureIn, frame: WlxFrame) -> Option<WlxCaptureOut> {
    match frame {
        WlxFrame::Dmabuf(frame) => {
            if !frame.is_valid() {
                log::error!("{}: Invalid frame", me.name);
                return None;
            }
            log::trace!("{}: New DMA-buf frame", me.name);
            let format = frame.format;
            match me.gfx.dmabuf_texture(frame) {
                Ok(image) => Some(WlxCaptureOut {
                    image: ImageView::new_default(image).ok()?,
                    format,
                    mouse: None,
                }),
                Err(e) => {
                    log::error!("{}: Failed to create DMA-buf vkImage: {}", me.name, e);
                    None
                }
            }
        }
        WlxFrame::MemFd(frame) => {
            let Some(fd) = frame.plane.fd else {
                log::error!("{}: No fd in MemFd frame", me.name);
                return None;
            };

            let format = match fourcc_to_vk(frame.format.drm_format.code) {
                Ok(x) => x,
                Err(e) => {
                    log::error!("{}: {}", me.name, e);
                    return None;
                }
            };

            let len = frame.plane.stride as usize * frame.format.height as usize;
            let offset = i64::from(frame.plane.offset);

            let map = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    len,
                    libc::PROT_READ,
                    libc::MAP_SHARED,
                    fd,
                    offset,
                )
            } as *const u8;

            let data = unsafe { std::slice::from_raw_parts(map, len) };

            let image = {
                let maybe_image =
                    upload_image(me, frame.format.width, frame.format.height, format, data);

                unsafe { libc::munmap(map as *mut _, len) };
                maybe_image
            }?;

            Some(WlxCaptureOut {
                image: ImageView::new_default(image).ok()?,
                format: frame.format,
                mouse: None,
            })
        }
        WlxFrame::MemPtr(frame) => {
            log::trace!("{}: New MemPtr frame", me.name);

            let format = match fourcc_to_vk(frame.format.drm_format.code) {
                Ok(x) => x,
                Err(e) => {
                    log::error!("{}: {}", me.name, e);
                    return None;
                }
            };

            let data = unsafe { std::slice::from_raw_parts(frame.ptr as *const u8, frame.size) };
            let image = upload_image(me, frame.format.width, frame.format.height, format, data)?;

            Some(WlxCaptureOut {
                image: ImageView::new_default(image).ok()?,
                format: frame.format,
                mouse: frame.mouse,
            })
        }
        WlxFrame::Implicit => {
            log::trace!("{}: New Implicit frame", me.name);

            let Some((image, format)) = me.dma_exporter.as_ref().unwrap().get_current() else {
                log::error!("{}: Implicit frame is missing!", me.name);
                return None;
            };

            Some(WlxCaptureOut {
                image,
                format,
                mouse: None,
            })
        }
    }
}

/// DmaExporter is not used for SHM capture
pub(super) struct DummyDrmExporter;
impl DmaExporter for DummyDrmExporter {
    fn next_frame(
        &mut self,
        _: u32,
        _: u32,
        _: DrmFourcc,
    ) -> Option<(wlx_frame::FramePlane, DrmModifier)> {
        None
    }
}

// Used when a separate GPU queue is not available
// In this case, receive_callback needs to run on the main thread
pub(super) struct MainThreadWlxCapture<T>
where
    T: WlxCapture<DummyDrmExporter, WlxFrame>,
{
    inner: T,
    data: Option<WlxCaptureIn>,
}

impl<T> MainThreadWlxCapture<T>
where
    T: WlxCapture<DummyDrmExporter, WlxFrame>,
{
    pub const fn new(inner: T) -> Self {
        Self { inner, data: None }
    }
}

impl<T> WlxCapture<WlxCaptureIn, WlxCaptureOut> for MainThreadWlxCapture<T>
where
    T: WlxCapture<DummyDrmExporter, WlxFrame>,
{
    fn init(
        &mut self,
        dmabuf_formats: &[DrmFormat],
        user_data: WlxCaptureIn,
        _: fn(&WlxCaptureIn, WlxFrame) -> Option<WlxCaptureOut>,
    ) {
        self.data = Some(user_data);
        self.inner
            .init(dmabuf_formats, DummyDrmExporter, receive_callback_dummy);
    }
    fn is_ready(&self) -> bool {
        self.inner.is_ready()
    }
    fn request_new_frame(&mut self) {
        self.inner.request_new_frame();
    }
    fn pause(&mut self) {
        self.inner.pause();
    }
    fn resume(&mut self) {
        self.inner.resume();
    }
    fn receive(&mut self) -> Option<WlxCaptureOut> {
        self.inner
            .receive()
            .and_then(|frame| receive_callback(self.data.as_ref().unwrap(), frame))
    }
    fn supports_dmbuf(&self) -> bool {
        self.inner.supports_dmbuf()
    }
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::unnecessary_wraps)]
const fn receive_callback_dummy(_: &DummyDrmExporter, frame: WlxFrame) -> Option<WlxFrame> {
    Some(frame)
}

fn extent_from_format(fmt: FrameFormat, config: &GeneralConfig) -> [u32; 2] {
    // screens above a certain resolution will have severe aliasing
    let height_limit = if config.screen_render_down {
        u32::from(config.screen_max_height.min(2560))
    } else {
        2560
    };

    let h = fmt.height.min(height_limit);
    let w = (fmt.width as f32 / fmt.height as f32 * h as f32) as u32;
    [w, h]
}

fn affine_from_format(format: &FrameFormat) -> Affine3A {
    const FLIP_X: Vec3 = Vec3 {
        x: -1.0,
        y: 1.0,
        z: 1.0,
    };

    match format.transform {
        wlx_frame::Transform::Rotated90 => Affine3A::from_rotation_z(-PI / 2.0),
        wlx_frame::Transform::Rotated180 => Affine3A::from_rotation_z(PI),
        wlx_frame::Transform::Rotated270 => Affine3A::from_rotation_z(PI / 2.0),
        wlx_frame::Transform::Flipped => Affine3A::from_scale(FLIP_X),
        wlx_frame::Transform::Flipped90 => {
            Affine3A::from_scale(FLIP_X) * Affine3A::from_rotation_z(-PI / 2.0)
        }
        wlx_frame::Transform::Flipped180 => {
            Affine3A::from_scale(FLIP_X) * Affine3A::from_rotation_z(PI)
        }
        wlx_frame::Transform::Flipped270 => {
            Affine3A::from_scale(FLIP_X) * Affine3A::from_rotation_z(PI / 2.0)
        }
        _ => Affine3A::IDENTITY,
    }
}

macro_rules! new_wlx_capture {
    ($capture_queue:expr, $capture:expr) => {
        if $capture_queue.is_none() {
            Box::new(MainThreadWlxCapture::new($capture)) as Box<dyn WlxCapture<_, _>>
        } else {
            Box::new($capture) as Box<dyn WlxCapture<_, _>>
        }
    };
}

pub(super) use new_wlx_capture;
