use core::f32;
use std::sync::Arc;

use glam::Vec4;
use ovr_overlay::{
    overlay::{OverlayHandle, OverlayManager},
    pose::Matrix3x4,
    sys::{ETrackingUniverseOrigin, VRVulkanTextureData_t},
};
use vulkano::{
    Handle, VulkanObject,
    image::{ImageUsage, view::ImageView},
};
use wgui::gfx::WGfx;

use crate::{graphics::ExtentExt, state::AppState, windowing::window::OverlayWindowData};

use super::helpers::Affine3AConvert;

#[derive(Default)]
pub(super) struct OpenVrOverlayData {
    pub(super) handle: Option<OverlayHandle>,
    pub(super) visible: bool,
    pub(super) color: Vec4,
    pub(crate) width: f32,
    pub(super) override_width: bool,
    pub(super) image_view: Option<Arc<ImageView>>,
    pub(super) image_dirty: bool,
}

impl OverlayWindowData<OpenVrOverlayData> {
    pub(super) fn initialize(
        &mut self,
        overlay: &mut OverlayManager,
        app: &mut AppState,
    ) -> anyhow::Result<OverlayHandle> {
        let key = format!("wlx-{}", self.config.name);
        log::debug!("Create overlay with key: {key}");
        let handle = match overlay.create_overlay(&key, &key) {
            Ok(handle) => handle,
            Err(e) => {
                panic!("Failed to create overlay: {e}");
            }
        };
        log::debug!("{}: initialize", self.config.name);

        self.data.handle = Some(handle);
        self.data.color = Vec4::ONE;

        self.init(app)?;

        if self.data.width < f32::EPSILON {
            self.data.width = 1.0;
        }

        self.upload_width(overlay);
        self.upload_color(overlay);
        self.upload_alpha(overlay);
        self.upload_curvature(overlay);
        self.upload_sort_order(overlay);

        Ok(handle)
    }

    pub(super) fn ensure_staging_image(
        &mut self,
        app: &mut AppState,
        extent: [u32; 2],
    ) -> anyhow::Result<Arc<ImageView>> {
        if let Some(image_view) = self.data.image_view.as_ref()
            && image_view.extent_u32arr() == extent
        {
            return Ok(image_view.clone());
        }

        log::debug!(
            "{}: recreating staging image at {}x{}",
            self.config.name,
            extent[0],
            extent[1],
        );

        let image = app.gfx.new_image(
            extent[0],
            extent[1],
            app.gfx.surface_format,
            ImageUsage::TRANSFER_SRC | ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
        )?;
        let image_view = ImageView::new_default(image)?;
        self.data.image_view = Some(image_view.clone());
        Ok(image_view)
    }

    pub(super) fn after_input(
        &mut self,
        overlay: &mut OverlayManager,
        app: &mut AppState,
    ) -> anyhow::Result<()> {
        let want_visible = self
            .config
            .active_state
            .as_ref()
            .is_some_and(|x| x.alpha > 0.05);

        if want_visible && !self.data.visible {
            self.show_internal(overlay, app)?;
        } else if !want_visible && self.data.visible {
            self.hide_internal(overlay, app)?;
        }
        Ok(())
    }

    pub(super) fn after_render(
        &mut self,
        universe: ETrackingUniverseOrigin,
        overlay: &mut OverlayManager,
        graphics: &WGfx,
    ) {
        if self.data.visible {
            if self.config.dirty {
                self.upload_curvature(overlay);

                self.upload_transform(universe, overlay);
                self.upload_alpha(overlay);
                self.config.dirty = false;
            }
            self.upload_texture(overlay, graphics);
        }
    }

    fn show_internal(
        &mut self,
        overlay: &mut OverlayManager,
        app: &mut AppState,
    ) -> anyhow::Result<()> {
        let handle = match self.data.handle {
            Some(handle) => handle,
            None => self.initialize(overlay, app)?,
        };
        log::debug!("{}: show", self.config.name);
        if let Err(e) = overlay.set_visibility(handle, true) {
            log::error!("{}: Failed to show overlay: {}", self.config.name, e);
        }
        self.data.visible = true;
        self.config.backend.resume(app)
    }

    fn hide_internal(
        &mut self,
        overlay: &mut OverlayManager,
        app: &mut AppState,
    ) -> anyhow::Result<()> {
        let Some(handle) = self.data.handle else {
            return Ok(());
        };
        log::debug!("{}: hide", self.config.name);
        if let Err(e) = overlay.set_visibility(handle, false) {
            log::error!("{}: Failed to hide overlay: {}", self.config.name, e);
        }
        self.data.visible = false;
        self.config.backend.pause(app)
    }

    pub(super) fn upload_alpha(&self, overlay: &mut OverlayManager) {
        let Some(handle) = self.data.handle else {
            log::debug!("{}: No overlay handle", self.config.name);
            return;
        };
        let Some(state) = self.config.active_state.as_ref() else {
            return;
        };
        if let Err(e) = overlay.set_opacity(handle, state.alpha) {
            log::error!("{}: Failed to set overlay alpha: {}", self.config.name, e);
        }
    }

    pub(super) fn upload_color(&self, overlay: &mut OverlayManager) {
        let Some(handle) = self.data.handle else {
            log::debug!("{}: No overlay handle", self.config.name);
            return;
        };
        if let Err(e) = overlay.set_tint(
            handle,
            ovr_overlay::ColorTint {
                r: self.data.color.x,
                g: self.data.color.y,
                b: self.data.color.z,
                a: self.data.color.w,
            },
        ) {
            log::error!("{}: Failed to set overlay tint: {}", self.config.name, e);
        }
    }

    fn upload_width(&self, overlay: &mut OverlayManager) {
        let Some(handle) = self.data.handle else {
            log::debug!("{}: No overlay handle", self.config.name);
            return;
        };
        if let Err(e) = overlay.set_width(handle, self.data.width) {
            log::error!("{}: Failed to set overlay width: {}", self.config.name, e);
        }
    }

    fn upload_curvature(&self, overlay: &mut OverlayManager) {
        let Some(handle) = self.data.handle else {
            log::debug!("{}: No overlay handle", self.config.name);
            return;
        };
        if let Err(e) = overlay.set_curvature(
            handle,
            self.config
                .active_state
                .as_ref()
                .unwrap()
                .curvature
                .unwrap_or(0.0),
        ) {
            log::error!(
                "{}: Failed to set overlay curvature: {}",
                self.config.name,
                e
            );
        }
    }

    fn upload_sort_order(&self, overlay: &mut OverlayManager) {
        let Some(handle) = self.data.handle else {
            log::debug!("{}: No overlay handle", self.config.name);
            return;
        };
        if let Err(e) = overlay.set_sort_order(handle, self.config.z_order) {
            log::error!("{}: Failed to set overlay z order: {}", self.config.name, e);
        }
    }

    pub(super) fn upload_transform(
        &mut self,
        universe: ETrackingUniverseOrigin,
        overlay: &mut OverlayManager,
    ) {
        let Some(handle) = self.data.handle else {
            log::debug!("{}: No overlay handle", self.config.name);
            return;
        };
        let Some(state) = self.config.active_state.as_ref() else {
            return;
        };

        let effective = state.transform
            * self
                .config
                .backend
                .frame_meta()
                .map(|f| f.transform)
                .unwrap_or_default();

        let transform = Matrix3x4::from_affine(&effective);

        if let Err(e) = overlay.set_transform_absolute(handle, universe, &transform) {
            log::error!(
                "{}: Failed to set overlay transform: {}",
                self.config.name,
                e
            );
        }
    }

    pub(super) fn upload_texture(&mut self, overlay: &mut OverlayManager, graphics: &WGfx) {
        let Some(handle) = self.data.handle else {
            log::debug!("{}: No overlay handle", self.config.name);
            return;
        };

        let Some(view) = self.data.image_view.as_ref() else {
            log::debug!("{}: Not rendered", self.config.name);
            return;
        };

        if !self.data.image_dirty {
            return;
        }
        self.data.image_dirty = false;

        let image = view.image().clone();
        let dimensions = image.extent();
        if !self.data.override_width {
            let new_width = ((dimensions[0] as f32) / (dimensions[1] as f32)).min(1.0);
            if (new_width - self.data.width).abs() > f32::EPSILON {
                log::info!("{}: New width {}", self.config.name, new_width);
                self.data.width = new_width;
                self.upload_width(overlay);
            }
        }

        let raw_image = image.handle().as_raw();
        let format = image.format();

        let mut texture = VRVulkanTextureData_t {
            m_nImage: raw_image,
            m_nFormat: format as _,
            m_nWidth: dimensions[0],
            m_nHeight: dimensions[1],
            m_nSampleCount: image.samples() as u32,
            m_pDevice: graphics.device.handle().as_raw() as *mut _,
            m_pPhysicalDevice: graphics.device.physical_device().handle().as_raw() as *mut _,
            m_pInstance: graphics.instance.handle().as_raw() as *mut _,
            m_pQueue: graphics.queue_gfx.handle().as_raw() as *mut _,
            m_nQueueFamilyIndex: graphics.queue_gfx.queue_family_index(),
        };
        log::trace!(
            "{}: UploadTex {:?}, {}x{}, {:?}",
            self.config.name,
            format,
            texture.m_nWidth,
            texture.m_nHeight,
            image.usage()
        );
        if let Err(e) = overlay.set_image_vulkan(handle, &mut texture) {
            log::error!("{}: Failed to set overlay texture: {}", self.config.name, e);
        }
    }

    pub(super) fn destroy(&mut self, overlay: &mut OverlayManager) {
        if let Some(handle) = self.data.handle {
            log::debug!("{}: destroy", self.config.name);
            if let Err(e) = overlay.destroy_overlay(handle) {
                log::error!("{}: Failed to destroy overlay: {}", self.config.name, e);
            }
        }
    }
}
