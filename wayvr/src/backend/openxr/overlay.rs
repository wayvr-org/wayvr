use glam::Vec3A;
use openxr::{self as xr, CompositionLayerFlags};
use smallvec::{SmallVec, smallvec};
use std::f32::consts::PI;
use xr::EyeVisibility;

use super::{CompositionLayer, XrState, helpers, swapchain::WlxSwapchain};
use crate::{
    backend::openxr::swapchain::{SwapchainOpts, WlxSwapchainImage, create_swapchain},
    state::AppState,
    windowing::window::OverlayWindowData,
};

#[derive(Default)]
pub struct OpenXrOverlayData {
    last_visible: bool,
    pub(super) swapchain: Option<WlxSwapchain>,
    pub(super) init: bool,
    pub(super) cur_visible: bool,
    color_bias_khr: Option<Box<xr::sys::CompositionLayerColorScaleBiasKHR>>,
}

macro_rules! next_chain_insert {
    ($layer:expr, $payload:expr) => {{
        let payload_ptr = $payload.as_mut() as *mut _ as *mut xr::sys::BaseInStructure;
        let new_elem = payload_ptr.as_mut().unwrap();
        let mut raw = $layer.into_raw();
        new_elem.next = raw.next as _;
        raw.next = payload_ptr as *const _;
        raw
    }};
}

impl OverlayWindowData<OpenXrOverlayData> {
    pub(super) fn ensure_swapchain_acquire<'a>(
        &'a mut self,
        app: &AppState,
        xr: &'a XrState,
        extent: [u32; 2],
        stereo: bool,
    ) -> anyhow::Result<WlxSwapchainImage> {
        let array_size = if stereo { 2 } else { 1 };

        if let Some(swapchain) = self.data.swapchain.as_mut()
            && swapchain.extent == extent
            && swapchain.array_size == array_size
        {
            return swapchain.acquire_wait_image();
        }

        log::debug!(
            "{}: recreating swapchain at {}x{}x{}",
            self.config.name,
            extent[0],
            extent[1],
            array_size,
        );
        let mut swapchain = create_swapchain(
            xr,
            app.gfx.clone(),
            extent,
            array_size,
            SwapchainOpts::new(),
        )?;
        let tgt = swapchain.acquire_wait_image()?;
        self.data.swapchain = Some(swapchain);
        Ok(tgt)
    }

    pub(super) fn present<'a>(
        &'a mut self,
        xr: &'a XrState,
    ) -> anyhow::Result<SmallVec<[CompositionLayer<'a>; 2]>> {
        let mut layers = SmallVec::new_const();

        let Some(swapchain) = self.data.swapchain.as_mut() else {
            log::trace!("{}: no swapchain", self.config.name);
            return Ok(layers);
        };
        if !swapchain.ever_acquired {
            log::warn!("{}: swapchain not rendered", self.config.name);
            return Ok(layers);
        }
        swapchain.ensure_image_released()?;

        // overlays without active_state don't get queued for present
        let state = self.config.active_state.as_ref().unwrap();

        let sub_images: SmallVec<[_; 2]> = if swapchain.array_size > 1 {
            smallvec![
                (swapchain.get_subimage(0), EyeVisibility::LEFT),
                (swapchain.get_subimage(1), EyeVisibility::RIGHT),
            ]
        } else {
            smallvec![(swapchain.get_subimage(0), EyeVisibility::BOTH),]
        };

        let transform = state.transform * self.config.backend.frame_meta().unwrap().transform; // contract

        let aspect_ratio = swapchain.extent[1] as f32 / swapchain.extent[0] as f32;
        let (scale_x, scale_y) = if aspect_ratio < 1.0 {
            let major = transform.matrix3.col(0).length();
            (major, major * aspect_ratio)
        } else {
            let major = transform.matrix3.col(1).length();
            (major / aspect_ratio, major)
        };

        let flags = if state.additive {
            CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA
        } else {
            CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA
                | CompositionLayerFlags::UNPREMULTIPLIED_ALPHA
        };

        if let Some(curvature) = state.curvature {
            let radius = scale_x / (2.0 * PI * curvature);
            let quat = helpers::transform_to_norm_quat(&transform);
            let center_point = transform.translation + quat.mul_vec3a(Vec3A::Z * radius);

            let posef = helpers::translation_rotation_to_posef(center_point, quat);
            let angle = 2.0 * (scale_x / (2.0 * radius));

            try_update_color_scale_bias(xr, &mut self.data.color_bias_khr, state.alpha);

            for sub_image in sub_images {
                let mut cylinder = xr::CompositionLayerCylinderKHR::new()
                    .layer_flags(flags)
                    .pose(posef)
                    .sub_image(sub_image.0)
                    .eye_visibility(sub_image.1)
                    .space(&xr.stage)
                    .radius(radius)
                    .central_angle(angle)
                    .aspect_ratio(aspect_ratio);

                if let Some(color_bias_khr) = self.data.color_bias_khr.as_mut() {
                    unsafe {
                        let raw = next_chain_insert!(cylinder, color_bias_khr);
                        cylinder = xr::CompositionLayerCylinderKHR::from_raw(raw);
                    }
                }

                layers.push(CompositionLayer::Cylinder(cylinder));
            }
        } else {
            let posef = helpers::transform_to_posef(&transform);
            try_update_color_scale_bias(xr, &mut self.data.color_bias_khr, state.alpha);

            for sub_image in sub_images {
                let mut quad = xr::CompositionLayerQuad::new()
                    .layer_flags(flags)
                    .pose(posef)
                    .sub_image(sub_image.0)
                    .eye_visibility(sub_image.1)
                    .space(&xr.stage)
                    .size(xr::Extent2Df {
                        width: scale_x,
                        height: scale_y,
                    });

                if let Some(color_bias_khr) = self.data.color_bias_khr.as_mut() {
                    unsafe {
                        let raw = next_chain_insert!(quad, color_bias_khr);
                        quad = xr::CompositionLayerQuad::from_raw(raw);
                    }
                }

                layers.push(CompositionLayer::Quad(quad));
            }
        }
        Ok(layers)
    }

    pub(super) fn after_input(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        let want_visible = self
            .config
            .active_state
            .as_ref()
            .is_some_and(|x| x.alpha > 0.05);

        if self.data.last_visible != want_visible {
            if want_visible {
                self.config.backend.resume(app)?;
            } else {
                self.config.backend.pause(app)?;
            }
        }
        self.data.last_visible = want_visible;
        Ok(())
    }
}

fn try_update_color_scale_bias(
    xr_state: &XrState,
    color_bias_khr: &mut Option<Box<xr::sys::CompositionLayerColorScaleBiasKHR>>,
    alpha: f32,
) {
    if let Some(item) = color_bias_khr.as_mut() {
        item.color_scale.a = alpha;
        return;
    }

    if xr_state
        .instance
        .exts()
        .khr_composition_layer_color_scale_bias
        .is_none()
    {
        return;
    }
    let new_item = Box::new(xr::sys::CompositionLayerColorScaleBiasKHR {
        ty: xr::StructureType::COMPOSITION_LAYER_COLOR_SCALE_BIAS_KHR,
        next: std::ptr::null(),
        color_bias: Default::default(),
        color_scale: xr::Color4f {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: alpha,
        },
    });
    *color_bias_khr = Some(new_item);
}
