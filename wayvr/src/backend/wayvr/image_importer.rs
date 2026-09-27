use std::{collections::HashMap, os::fd::AsRawFd, sync::Arc};

use anyhow::Context;
use smithay::{
    backend::allocator::{
        Buffer,
        dmabuf::{Dmabuf, WeakDmabuf},
    },
    wayland::{
        shm::{BufferData, shm_format_to_fourcc},
        single_pixel_buffer::SinglePixelBufferUserData,
    },
};
use wgui::gfx::{CommandBufferUsage, Format, ImageView, WGfx};
use wlx_capture::frame::{DmabufFrame, FrameFormat, Transform};

use crate::graphics::dmabuf::{WGfxDmabuf, fourcc_to_vk};

pub struct ImageImporter {
    gfx: Arc<WGfx>,
    dmabufs: HashMap<WeakDmabuf, Arc<ImageView>>,
}

impl ImageImporter {
    pub fn new(gfx: Arc<WGfx>) -> Self {
        Self {
            gfx,
            dmabufs: HashMap::new(),
        }
    }

    pub fn import_spb(
        &mut self,
        spb: &SinglePixelBufferUserData,
    ) -> anyhow::Result<Arc<ImageView>> {
        let mut cmd_buf = self
            .gfx
            .create_xfer_command_buffer(CommandBufferUsage::OneTimeSubmit)?;

        let rgba = spb.rgba8888();
        let image = cmd_buf.upload_image(1, 1, Format::R8G8B8A8_UNORM, &rgba)?;

        cmd_buf.build_and_execute_now()?; //TODO: async

        self.gfx.create_image_view(image)
    }

    pub fn import_shm(
        &mut self,
        data: *const u8,
        size: usize,
        bd: BufferData,
    ) -> anyhow::Result<Arc<ImageView>> {
        let mut cmd_buf = self
            .gfx
            .create_xfer_command_buffer(CommandBufferUsage::OneTimeSubmit)?;

        let fourcc = shm_format_to_fourcc(bd.format)
            .with_context(|| format!("Could not convert {:?} to fourcc", bd.format))?;

        let format = fourcc_to_vk(fourcc)
            .with_context(|| format!("Could not convert {fourcc} to vkFormat"))?;

        let bytes_per_pixel = match format {
            Format::R8G8B8A8_UNORM
            | Format::B8G8R8A8_UNORM
            | Format::R8G8B8A8_SRGB
            | Format::B8G8R8A8_SRGB => 4,
            other => anyhow::bail!("unsupported SHM format for packed upload: {other:?}"),
        };

        let row_bytes = bd.width as usize * bytes_per_pixel;
        let stride = bd.stride as usize;
        let height = bd.height as usize;

        if stride < row_bytes {
            anyhow::bail!("invalid wl_shm stride: {stride} < {row_bytes}");
        }

        let required = stride * height;
        if size < required {
            anyhow::bail!("wl_shm buffer too small: {size} < {required}");
        }

        let src = unsafe { std::slice::from_raw_parts(data, required) };
        let mut packed = vec![0u8; row_bytes * height];

        for y in 0..height {
            let src_row = &src[y * stride..y * stride + row_bytes];
            let dst_row = &mut packed[y * row_bytes..(y + 1) * row_bytes];
            dst_row.copy_from_slice(src_row);
        }

        let image = cmd_buf.upload_image(bd.width as _, bd.height as _, format, &packed)?;
        cmd_buf.build_and_execute_now()?;

        self.gfx.create_image_view(image)
    }

    pub fn get_or_import_dmabuf(&mut self, dmabuf: Dmabuf) -> anyhow::Result<Arc<ImageView>> {
        let key = dmabuf.weak();

        if let Some(view) = self.dmabufs.get(&key) {
            return Ok(view.clone());
        }

        let mut frame = DmabufFrame {
            format: FrameFormat {
                width: dmabuf.width(),
                height: dmabuf.height(),
                drm_format: dmabuf.format(),
                transform: Transform::Undefined,
            },
            num_planes: dmabuf.num_planes(),
            planes: Default::default(),
            mouse: None,
        };

        for (i, handle) in dmabuf.handles().enumerate() {
            // even if the original OwnedFd is dropped, the vkImage will hold reference on the DMA-buf
            frame.planes[i].fd = Some(handle.as_raw_fd());
        }

        for (i, offset) in dmabuf.offsets().enumerate() {
            frame.planes[i].offset = offset;
        }

        for (i, stride) in dmabuf.strides().enumerate() {
            frame.planes[i].stride = stride as _;
        }

        let image = self.gfx.dmabuf_texture(frame)?;
        log::info!("DMA-buf import: creating image view");
        let image_view = self.gfx.create_image_view(image)?;
        log::info!("DMA-buf import: image view created");
        self.dmabufs.insert(key, image_view.clone());
        log::info!("DMA-buf import: cached ImageView");

        Ok(image_view)
    }

    pub fn cleanup(&mut self) {
        self.dmabufs.retain(|k, _| !k.is_gone());
    }
}
