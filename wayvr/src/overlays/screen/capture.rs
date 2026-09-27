use std::{
    os::fd::{AsRawFd, BorrowedFd, OwnedFd, RawFd},
    ptr::NonNull,
    sync::{Arc, Mutex},
};

use glam::Affine3A;
use smallvec::{SmallVec, smallvec};
use wgui::{
    gfx::{
        Buffer, BufferUsage, CommandBufferUsage, Filter, Format, Image, ImageView, Scissor,
        Vert2Uv, WGfx,
        cmd::WGfxClearMode,
        pass::WGfxPass,
        pipeline::{WGfxPipeline, WPipelineCreateInfo},
        upload_quad_vertices,
    },
    log::LogErr,
};
use wlx_capture::{
    DrmFormat, DrmFourcc, DrmModifier, WlxCapture,
    frame::{self as wlx_frame, DmaExporter, FrameFormat, MouseMeta, WlxFrame},
};
use wlx_common::{config::GeneralConfig, overlays::StereoMode};

use crate::{
    graphics::dmabuf::{ExportedDmabufImage, WGfxDmabuf, export_dmabuf_image, fourcc_to_vk},
    state::AppState,
    windowing::backend::{FrameMeta, RenderResources},
};

const CURSOR_SIZE: f32 = 16. / 1440.;

struct BufPass {
    pass: WGfxPass<Vert2Uv>,
    buf_vert: Arc<Buffer<Vert2Uv>>,
}

/// A render pipeline that supports mouse + stereo
pub struct ScreenPipeline {
    mouse: BufPass,
    pass: SmallVec<[BufPass; 2]>,
    pipeline: Arc<WGfxPipeline<Vert2Uv>>,
    extentf: [f32; 2],
    offsetf: [f32; 2],
    transform: wlx_frame::Transform,
    stereo: StereoMode,
    stereo_adjust_mouse: bool,
    fallback_image: Arc<ImageView>,
}

impl ScreenPipeline {
    pub fn new(
        meta: &FrameMeta,
        app: &mut AppState,
        stereo: StereoMode,
        offsetf: [f32; 2],
        transform: wlx_frame::Transform,
    ) -> anyhow::Result<Self> {
        let extentf = [meta.extent[0] as f32, meta.extent[1] as f32];

        let pipeline = app.gfx.create_pipeline(
            app.gfx_extras.shaders.get("vert_quad").unwrap(), // want panic
            app.gfx_extras.shaders.get("frag_screen").unwrap(), // want panic
            WPipelineCreateInfo::new(app.gfx.surface_format()).use_updatable_descriptors([0]),
        )?;

        let mut cmd_xfer = app
            .gfx
            .create_xfer_command_buffer(CommandBufferUsage::OneTimeSubmit)?;
        let fallback_image =
            cmd_xfer.upload_image(1, 1, Format::R8G8B8A8_SRGB, &[255, 0, 255, 255])?;
        cmd_xfer.build_and_execute_now()?;
        let fallback_image = app.gfx.create_image_view(fallback_image)?;

        let mut me = Self {
            pass: smallvec![Self::create_pass(
                app,
                pipeline.clone(),
                fallback_image.clone(),
                extentf,
                offsetf,
            )?],
            mouse: Self::create_mouse_pass(app, pipeline.clone(), extentf, offsetf)?,
            pipeline,
            extentf,
            offsetf,
            transform,
            stereo,
            stereo_adjust_mouse: false,
            fallback_image,
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

    pub const fn set_stereo_adjust_mouse(&mut self, adjust: bool) {
        self.stereo_adjust_mouse = adjust;
    }

    pub const fn transform(&self) -> wlx_frame::Transform {
        self.transform
    }

    fn ensure_depth(&mut self, app: &mut AppState, depth: usize) -> anyhow::Result<()> {
        while self.pass.len() < depth {
            self.pass.push(Self::create_pass(
                app,
                self.pipeline.clone(),
                self.fallback_image.clone(),
                self.extentf,
                self.offsetf,
            )?);
        }

        while self.pass.len() > depth {
            self.pass.pop();
        }

        for (eye, current) in self.pass.iter_mut().enumerate() {
            let verts = stereo_mode_to_verts(self.stereo, eye, self.transform);
            current.buf_vert.write()?.copy_from_slice(&verts);
        }
        Ok(())
    }

    pub fn set_layout(
        &mut self,
        app: &mut AppState,
        extentf: [f32; 2],
        offsetf: [f32; 2],
        transform: wlx_frame::Transform,
    ) -> anyhow::Result<()> {
        self.extentf = extentf;
        self.offsetf = offsetf;
        self.transform = transform;
        self.pass.clear();

        self.mouse = Self::create_mouse_pass(app, self.pipeline.clone(), extentf, offsetf)?;
        Ok(())
    }

    fn create_pass(
        app: &mut AppState,
        pipeline: Arc<WGfxPipeline<Vert2Uv>>,
        fallback_image: Arc<ImageView>,
        extentf: [f32; 2],
        offsetf: [f32; 2],
    ) -> anyhow::Result<BufPass> {
        let set0 = pipeline.uniform_sampler(0, fallback_image, app.gfx.texture_filter())?;
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
            Scissor::from_viewport(extentf, offsetf),
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

        let image = cmd_xfer.upload_image(4, 4, Format::R8G8B8A8_UNORM, &mouse_bytes)?;

        let view = app.gfx.create_image_view(image)?;

        let buf_vert = app
            .gfx
            .empty_buffer(BufferUsage::TRANSFER_DST | BufferUsage::VERTEX_BUFFER, 4)?;

        let set0 = pipeline.uniform_sampler(0, view, Filter::Nearest)?;
        let pass = pipeline.create_pass(
            extentf,
            offsetf,
            buf_vert.clone(),
            0..4,
            0..1,
            vec![set0],
            Scissor::from_viewport(extentf, offsetf),
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
        self.render_screen(image, app, rdr)?;
        if let Some(mouse) = mouse {
            self.render_mouse(mouse, rdr)?;
        }
        Ok(())
    }

    pub fn render_screen(
        &mut self,
        image: Arc<ImageView>,
        app: &mut AppState,
        rdr: &mut RenderResources,
    ) -> anyhow::Result<()> {
        self.ensure_depth(app, rdr.cmd_bufs.len())?;

        for (eye, cmd_buf) in rdr.cmd_bufs.iter_mut().enumerate() {
            let current = &mut self.pass[eye];

            current
                .pass
                .update_sampler(0, image.clone(), app.gfx.texture_filter())?;

            cmd_buf.run_ref(&current.pass)?;
        }

        Ok(())
    }

    pub fn render_mouse(
        &mut self,
        mouse: &MouseMeta,
        rdr: &mut RenderResources,
    ) -> anyhow::Result<()> {
        for cmd_buf in &mut rdr.cmd_bufs {
            let size = CURSOR_SIZE * self.extentf[1];
            let half_size = size * 0.5;

            let (x_scale, y_scale) = if self.stereo_adjust_mouse {
                match self.stereo {
                    StereoMode::LeftRight | StereoMode::RightLeft => (2.0, 1.0),
                    StereoMode::TopBottom | StereoMode::BottomTop => (1.0, 2.0),
                    _ => (1.0, 1.0),
                }
            } else {
                (1.0, 1.0)
            };

            upload_quad_vertices(
                &self.mouse.buf_vert,
                self.extentf[0],
                self.extentf[1],
                mouse.x.mul_add(self.extentf[0] * x_scale, -half_size),
                mouse.y.mul_add(self.extentf[1] * y_scale, -half_size),
                size,
                size,
            )?;

            cmd_buf.run_ref(&self.mouse.pass)?;
        }

        Ok(())
    }
}

fn transform_uv(uv: [f32; 2], transform: wlx_frame::Transform) -> [f32; 2] {
    let [u, v] = uv;
    match transform {
        wlx_frame::Transform::Normal | wlx_frame::Transform::Undefined => [u, v],
        wlx_frame::Transform::Rotated90 => [v, 1.0 - u],
        wlx_frame::Transform::Rotated180 => [1.0 - u, 1.0 - v],
        wlx_frame::Transform::Rotated270 => [1.0 - v, u],
        wlx_frame::Transform::Flipped => [1.0 - u, v],
        wlx_frame::Transform::Flipped90 => [v, u],
        wlx_frame::Transform::Flipped180 => [u, 1.0 - v],
        wlx_frame::Transform::Flipped270 => [1.0 - v, 1.0 - u],
    }
}

fn stereo_mode_to_verts(
    stereo: StereoMode,
    array_index: usize,
    transform: wlx_frame::Transform,
) -> [Vert2Uv; 4] {
    let eye = match stereo {
        StereoMode::RightLeft | StereoMode::BottomTop => (1 - array_index) as f32,
        _ => array_index as f32,
    };

    let mut verts = match stereo {
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
    };

    for vert in &mut verts {
        vert.in_uv = transform_uv(vert.in_uv, transform);
    }

    verts
}

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
        let extent = image.view.extent_2d();
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

        for _ in 0..2 {
            let image = export_dmabuf_image(&self.gfx, [width, height, 1], format, modifier)
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
    dma_exporter: Option<Arc<Mutex<MyFirstDmaExporter>>>,
    use_capture_queue: bool,
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
            dma_exporter: dma_exporter.map(|exporter| Arc::new(Mutex::new(exporter))),
            use_capture_queue: app.gfx.has_capture_queue(),
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
        let mut dma_exporter = self.dma_exporter.as_ref()?.lock().ok()?;
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
            transform: Affine3A::IDENTITY,
            format: self.image.format(),
            stereo,
        }
    }
}

struct MappedMemFd {
    base: NonNull<libc::c_void>,
    map_len: usize,
    data_offset: usize,
    data_len: usize,
}

impl MappedMemFd {
    fn new(fd: RawFd, offset: u32, len: usize) -> Option<Self> {
        if len == 0 {
            log::error!("Refusing to mmap an empty CPU capture frame");
            return None;
        }

        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page_size <= 0 {
            log::error!("Could not query system page size for CPU capture mmap");
            return None;
        }
        let page_size = usize::try_from(page_size).ok()?;
        let offset = offset as usize;
        let map_offset = offset / page_size * page_size;
        let data_offset = offset - map_offset;
        let map_len = data_offset.checked_add(len)?;
        let map_offset = libc::off_t::try_from(map_offset).ok()?;

        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                map_len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                fd,
                map_offset,
            )
        };
        if ptr == libc::MAP_FAILED {
            log::error!(
                "CPU capture mmap failed for fd {fd}: {}",
                std::io::Error::last_os_error()
            );
            return None;
        }

        Some(Self {
            base: NonNull::new(ptr)?,
            map_len,
            data_offset,
            data_len: len,
        })
    }

    const fn as_slice(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.base.as_ptr().cast::<u8>().add(self.data_offset),
                self.data_len,
            )
        }
    }
}

impl Drop for MappedMemFd {
    fn drop(&mut self) {
        if unsafe { libc::munmap(self.base.as_ptr(), self.map_len) } != 0 {
            log::error!(
                "CPU capture munmap failed: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}

fn memfd_frame_len(frame: &wlx_frame::MemFdFrame) -> Option<usize> {
    let Ok(stride) = usize::try_from(frame.plane.stride) else {
        log::error!(
            "CPU capture frame has a negative stride: {}",
            frame.plane.stride
        );
        return None;
    };
    if let Some(len) = stride.checked_mul(frame.format.height as usize) {
        Some(len)
    } else {
        log::error!("CPU capture frame byte size overflow");
        None
    }
}

fn upload_image(
    me: &WlxCaptureIn,
    width: u32,
    height: u32,
    format: Format,
    data: &[u8],
) -> Option<Arc<Image>> {
    let cmd_result = if me.use_capture_queue {
        me.gfx
            .create_capture_command_buffer(CommandBufferUsage::OneTimeSubmit)
    } else {
        me.gfx
            .create_xfer_command_buffer(CommandBufferUsage::OneTimeSubmit)
    };
    let mut cmd_xfer = match cmd_result {
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
                    image: me.gfx.create_image_view(image).ok()?,
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

            let len = memfd_frame_len(&frame)?;
            let map = MappedMemFd::new(fd, frame.plane.offset, len)?;
            let image = upload_image(
                me,
                frame.format.width,
                frame.format.height,
                format,
                map.as_slice(),
            )?;

            Some(WlxCaptureOut {
                image: me.gfx.create_image_view(image).ok()?,
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
                image: me.gfx.create_image_view(image).ok()?,
                format: frame.format,
                mouse: frame.mouse,
            })
        }
        WlxFrame::Implicit(transform) => {
            log::trace!("{}: New Implicit frame", me.name);

            let Some(dma_exporter) = me.dma_exporter.as_ref() else {
                log::error!("{}: Implicit frame is missing DMA exporter!", me.name);
                return None;
            };
            let Ok(dma_exporter) = dma_exporter.lock() else {
                log::error!("{}: DMA exporter lock is poisoned", me.name);
                return None;
            };
            let Some((image, mut format)) = dma_exporter.get_current() else {
                log::error!("{}: Implicit frame is missing!", me.name);
                return None;
            };
            format.transform = transform;

            Some(WlxCaptureOut {
                image,
                format,
                mouse: None,
            })
        }
    }
}

pub(super) struct DmaExporterProxy(Option<Arc<Mutex<MyFirstDmaExporter>>>);

impl DmaExporter for DmaExporterProxy {
    fn next_frame(
        &mut self,
        width: u32,
        height: u32,
        fourcc: DrmFourcc,
    ) -> Option<(wlx_frame::FramePlane, DrmModifier)> {
        let mut exporter = self.0.as_ref()?.lock().ok()?;
        exporter.set_format(width, height, fourcc)?;
        exporter.next_frame()
    }
}

pub(super) enum MainThreadFrame {
    Cpu {
        format: FrameFormat,
        data: Vec<u8>,
        mouse: Option<MouseMeta>,
    },
    Dmabuf {
        frame: wlx_frame::DmabufFrame,
        fds: Vec<OwnedFd>,
    },
    Implicit(wlx_frame::Transform),
}

pub(super) struct MainThreadWlxCapture<T>
where
    T: WlxCapture<DmaExporterProxy, MainThreadFrame>,
{
    inner: T,
    data: Option<WlxCaptureIn>,
}

impl<T> MainThreadWlxCapture<T>
where
    T: WlxCapture<DmaExporterProxy, MainThreadFrame>,
{
    pub const fn new(inner: T) -> Self {
        Self { inner, data: None }
    }
}

impl<T> WlxCapture<WlxCaptureIn, WlxCaptureOut> for MainThreadWlxCapture<T>
where
    T: WlxCapture<DmaExporterProxy, MainThreadFrame>,
{
    fn init(
        &mut self,
        dmabuf_formats: &[DrmFormat],
        user_data: WlxCaptureIn,
        _: fn(&WlxCaptureIn, WlxFrame) -> Option<WlxCaptureOut>,
    ) {
        let dma_exporter = DmaExporterProxy(user_data.dma_exporter.clone());
        self.data = Some(user_data);
        self.inner
            .init(dmabuf_formats, dma_exporter, receive_callback_dummy);
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
        let frame = self.inner.receive()?;
        match frame {
            MainThreadFrame::Cpu {
                format,
                data,
                mouse,
            } => {
                let frame = wlx_frame::MemPtrFrame {
                    format,
                    ptr: data.as_ptr() as usize,
                    size: data.len(),
                    mouse,
                };
                receive_callback(
                    self.data.as_ref().expect("capture must be initialized"),
                    WlxFrame::MemPtr(frame),
                )
            }
            MainThreadFrame::Dmabuf { frame, fds: _fds } => receive_callback(
                self.data.as_ref().expect("capture must be initialized"),
                WlxFrame::Dmabuf(frame),
            ),
            MainThreadFrame::Implicit(transform) => receive_callback(
                self.data.as_ref().expect("capture must be initialized"),
                WlxFrame::Implicit(transform),
            ),
        }
    }
    fn supports_dmbuf(&self) -> bool {
        self.inner.supports_dmbuf()
    }
}

fn receive_callback_dummy(_: &DmaExporterProxy, frame: WlxFrame) -> Option<MainThreadFrame> {
    match frame {
        WlxFrame::MemFd(frame) => {
            let Some(fd) = frame.plane.fd else {
                log::error!("Main-thread CPU capture received a MemFd frame without an fd");
                return None;
            };
            let len = memfd_frame_len(&frame)?;
            let map = MappedMemFd::new(fd, frame.plane.offset, len)?;
            Some(MainThreadFrame::Cpu {
                format: frame.format,
                data: map.as_slice().to_vec(),
                mouse: frame.mouse,
            })
        }
        WlxFrame::MemPtr(frame) => {
            let data = if frame.size == 0 {
                Vec::new()
            } else {
                if frame.ptr == 0 {
                    log::error!("Main-thread CPU capture received a null MemPtr frame");
                    return None;
                }
                unsafe { std::slice::from_raw_parts(frame.ptr as *const u8, frame.size) }.to_vec()
            };
            Some(MainThreadFrame::Cpu {
                format: frame.format,
                data,
                mouse: frame.mouse,
            })
        }
        WlxFrame::Dmabuf(mut frame) => {
            if frame.num_planes > frame.planes.len() || !frame.is_valid() {
                log::error!("Main-thread capture received an invalid DMA-buf frame");
                return None;
            }

            let mut fds = Vec::with_capacity(frame.num_planes);
            for plane in &mut frame.planes[..frame.num_planes] {
                let fd = plane.fd?;
                let owned = match unsafe { BorrowedFd::borrow_raw(fd) }.try_clone_to_owned() {
                    Ok(fd) => fd,
                    Err(e) => {
                        log::error!("Failed to duplicate DMA-buf fd for main-thread capture: {e}");
                        return None;
                    }
                };
                plane.fd = Some(owned.as_raw_fd());
                fds.push(owned);
            }
            Some(MainThreadFrame::Dmabuf { frame, fds })
        }
        WlxFrame::Implicit(transform) => Some(MainThreadFrame::Implicit(transform)),
    }
}

fn extent_from_format(fmt: FrameFormat, config: &GeneralConfig) -> [u32; 2] {
    let (width, height) = match fmt.transform {
        wlx_frame::Transform::Rotated90
        | wlx_frame::Transform::Rotated270
        | wlx_frame::Transform::Flipped90
        | wlx_frame::Transform::Flipped270 => (fmt.height, fmt.width),
        _ => (fmt.width, fmt.height),
    };

    // screens above a certain resolution will have severe aliasing
    let height_limit = if config.screen_render_down {
        u32::from(config.screen_max_height.min(2560))
    } else {
        2560
    };

    let h = height.min(height_limit);
    let w = (width as f32 / height as f32 * h as f32) as u32;
    [w, h]
}

macro_rules! new_wlx_capture {
    ($app:expr, $capture:expr) => {{
        if $app.gfx.has_capture_queue() {
            Box::new($capture) as Box<dyn wlx_capture::WlxCapture<_, _>>
        } else {
            Box::new($crate::overlays::screen::capture::MainThreadWlxCapture::new($capture))
                as Box<dyn wlx_capture::WlxCapture<_, _>>
        }
    }};
}

pub(super) use new_wlx_capture;
