use std::sync::Arc;

use openxr as xr;

use smallvec::SmallVec;
use wgui::gfx::{
    GpuCompletionMarker, ImageCreateInfo, ImageLayout, ImageUsage, ImageView, ImageViewCreateInfo,
    WGfx,
};

use super::XrState;

#[derive(Default)]
pub(super) struct SwapchainOpts {
    pub immutable: bool,
}

impl SwapchainOpts {
    pub fn new() -> Self {
        Self::default()
    }
    pub const fn immutable(mut self) -> Self {
        self.immutable = true;
        self
    }
}

#[allow(clippy::range_plus_one)]
pub(super) fn create_swapchain(
    xr: &XrState,
    gfx: Arc<WGfx>,
    extent: [u32; 2],
    array_size: u32,
    opts: SwapchainOpts,
) -> anyhow::Result<WlxSwapchain> {
    let create_flags = if opts.immutable {
        xr::SwapchainCreateFlags::STATIC_IMAGE
    } else {
        xr::SwapchainCreateFlags::EMPTY
    };

    let swapchain = xr.session.create_swapchain(&xr::SwapchainCreateInfo {
        create_flags,
        usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT | xr::SwapchainUsageFlags::SAMPLED,
        format: gfx.raw_format(gfx.surface_format()) as _,
        sample_count: 1,
        width: extent[0],
        height: extent[1],
        face_count: 1,
        array_size,
        mip_count: 1,
    })?;

    let images = swapchain
        .enumerate_images()?
        .into_iter()
        .map(|handle| {
            // SAFETY: XR runtime owns the swapchain image & keeps it alive until explicitly destroyed
            let image = unsafe {
                gfx.wrap_external_image_with_layout(
                    handle,
                    ImageCreateInfo {
                        format: gfx.surface_format(),
                        extent: [extent[0], extent[1], 1],
                        array_layers: array_size,
                        usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
                        ..ImageCreateInfo::new_2d(
                            extent[0],
                            extent[1],
                            gfx.surface_format(),
                            ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
                        )
                    },
                    ImageLayout::ColorAttachment,
                )?
            };
            let mut wsi = WlxSwapchainImage::default();
            for d in 0..array_size {
                wsi.views.push(gfx.create_image_view_with_info(
                    image.clone(),
                    ImageViewCreateInfo {
                        base_array_layer: d,
                        array_layer_count: 1,
                        ..Default::default()
                    },
                )?);
            }
            Ok(wsi)
        })
        .collect::<anyhow::Result<SmallVec<[WlxSwapchainImage; 4]>>>()?;

    Ok(WlxSwapchain {
        acquired: false,
        ever_acquired: false,
        images,
        swapchain,
        extent,
        array_size,
    })
}

#[derive(Default, Clone)]
pub(super) struct WlxSwapchainImage {
    pub views: SmallVec<[Arc<ImageView>; 2]>,
}

pub(super) struct WlxSwapchain {
    acquired: bool,
    pub(super) ever_acquired: bool,
    // drop image views before destroying parent swapchain
    pub(super) images: SmallVec<[WlxSwapchainImage; 4]>,
    pub(super) swapchain: xr::Swapchain<xr::Vulkan>,
    pub(super) extent: [u32; 2],
    pub(super) array_size: u32,
}

impl WlxSwapchain {
    pub(super) fn acquire_wait_image(&mut self) -> anyhow::Result<WlxSwapchainImage> {
        let idx = self.swapchain.acquire_image()? as usize;
        self.swapchain.wait_image(xr::Duration::INFINITE)?;
        self.ever_acquired = true;
        self.acquired = true;
        Ok(self.images[idx].clone())
    }

    pub(super) fn ensure_image_released(&mut self) -> anyhow::Result<()> {
        if self.acquired {
            self.swapchain.release_image()?;
            self.acquired = false;
        }
        Ok(())
    }

    pub(super) fn get_subimage(&self, array_index: u32) -> xr::SwapchainSubImage<'_, xr::Vulkan> {
        debug_assert!(self.ever_acquired, "swapchain was never acquired!");
        xr::SwapchainSubImage::new()
            .swapchain(&self.swapchain)
            .image_rect(xr::Rect2Di {
                offset: xr::Offset2Di { x: 0, y: 0 },
                extent: xr::Extent2Di {
                    width: self.extent[0] as _,
                    height: self.extent[1] as _,
                },
            })
            .image_array_index(array_index)
    }
}

struct RetiredSwapchain {
    // keep alive until completion is signaled
    _swapchain: WlxSwapchain,
    completion: Option<GpuCompletionMarker>,
}

/// Keeps destroyed xrswapchains alive until work submitted by runtime from has finished
#[derive(Default)]
pub(super) struct SwapchainRetirementQueue {
    retired: Vec<RetiredSwapchain>,
}

impl SwapchainRetirementQueue {
    /// queue the swapchain for non-blocking retirement
    pub(super) fn retire(&mut self, swapchain: WlxSwapchain, gfx: &Arc<WGfx>) {
        self.retired.push(RetiredSwapchain {
            _swapchain: swapchain,
            completion: None,
        });

        let idx = self.retired.len() - 1;
        if let Err(e) = Self::arm(&mut self.retired[idx], gfx) {
            log::warn!("Failed arming retired OpenXR swapchain: {e:#}");
        }
    }

    fn arm(retired: &mut RetiredSwapchain, gfx: &Arc<WGfx>) -> anyhow::Result<()> {
        if retired.completion.is_some() {
            return Ok(());
        }

        retired._swapchain.ensure_image_released()?;
        retired.completion = Some(gfx.submit_graphics_completion_marker()?);
        Ok(())
    }

    pub(super) fn collect(&mut self, gfx: &Arc<WGfx>) {
        let mut idx = 0;
        while idx < self.retired.len() {
            if self.retired[idx].completion.is_none() {
                if let Err(e) = Self::arm(&mut self.retired[idx], gfx) {
                    log::warn!("Failed re-arming retired OpenXR swapchain: {e:#}");
                    idx += 1;
                    continue;
                }
            }

            let complete = match self.retired[idx]
                .completion
                .as_ref()
                .expect("retired swapchain completion marker missing after arm")
                .is_complete()
            {
                Ok(complete) => complete,
                Err(e) => {
                    log::warn!("Failed polling retired OpenXR swapchain: {e:#}");
                    false
                }
            };

            if complete {
                self.retired.swap_remove(idx);
            } else {
                idx += 1;
            }
        }
    }
}
