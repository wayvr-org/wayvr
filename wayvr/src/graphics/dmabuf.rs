use std::sync::Arc;

use wgui::gfx::{DmabufImportInfo, DmabufPlaneLayout, Format, Image, ImageView, WGfx};
use wlx_capture::{DrmFormat, DrmFourcc, DrmModifier, frame::DmabufFrame};

pub trait WGfxDmabuf {
    fn dmabuf_texture(&self, frame: DmabufFrame) -> anyhow::Result<Arc<Image>>;
}

impl WGfxDmabuf for WGfx {
    fn dmabuf_texture(&self, frame: DmabufFrame) -> anyhow::Result<Arc<Image>> {
        let format = fourcc_to_vk(frame.format.drm_format.code)?;
        let modifier = (!matches!(frame.format.drm_format.modifier, DrmModifier::Invalid))
            .then(|| u64::from(frame.format.drm_format.modifier));
        let plane_layouts = if modifier.is_some() {
            (0..frame.num_planes)
                .map(|i| DmabufPlaneLayout {
                    offset: frame.planes[i].offset.into(),
                    row_pitch: frame.planes[i].stride as u64,
                })
                .collect()
        } else {
            Vec::new()
        };
        let Some(fd) = frame.planes[0].fd else {
            anyhow::bail!("DMA-buf plane has no FD");
        };

        log::info!(
            "DMA-buf import start: {}x{}, format={:?}, modifier={:?}, planes={}",
            frame.format.width,
            frame.format.height,
            format,
            modifier,
            frame.num_planes,
        );

        self.import_dmabuf(&DmabufImportInfo {
            extent: [frame.format.width, frame.format.height, 1],
            format,
            fd,
            modifier,
            plane_layouts,
        })
    }
}

pub struct ExportedDmabufImage {
    pub view: Arc<ImageView>,
    pub fd: std::fs::File,
    pub offset: u32,
    pub stride: i32,
    pub modifier: DrmModifier,
}

pub fn export_dmabuf_image(
    gfx: &WGfx,
    extent: [u32; 3],
    format: Format,
    modifier: DrmModifier,
) -> anyhow::Result<ExportedDmabufImage> {
    let exported = gfx.export_dmabuf_image(extent, format, modifier.into())?;

    Ok(ExportedDmabufImage {
        view: exported.view,
        fd: exported.fd,
        modifier: DrmModifier::from(exported.modifier),
        offset: exported.offset as _,
        stride: exported.stride as _,
    })
}

pub(super) fn get_drm_formats(gfx: &WGfx) -> Vec<DrmFormat> {
    let possible_formats = [
        DrmFourcc::Abgr8888,
        DrmFourcc::Xbgr8888,
        DrmFourcc::Argb8888,
        DrmFourcc::Xrgb8888,
        DrmFourcc::Abgr2101010,
        DrmFourcc::Xbgr2101010,
    ];

    let mut out_formats = vec![];

    for &code in &possible_formats {
        let Ok(format) = fourcc_to_vk(code) else {
            continue;
        };

        match gfx.dmabuf_import_modifiers(format) {
            Ok(modifiers) => {
                for modifier in modifiers {
                    log::debug!(
                        "DMA-buf format {code} modifier {modifier:#018x}: sampled + importable"
                    );
                    out_formats.push(DrmFormat {
                        code,
                        modifier: DrmModifier::from(modifier),
                    });
                }
            }
            Err(e) => {
                log::warn!("Failed to query DMA-buf modifiers for {code}: {e:?}");
            }
        }

        // always accept implicit modifier
        out_formats.push(DrmFormat {
            code,
            modifier: DrmModifier::Invalid,
        });
    }
    log::debug!("Supported DRM formats:");
    for f in &out_formats {
        log::debug!("  {} {:?}", f.code, f.modifier);
    }
    out_formats
}

pub fn fourcc_to_vk(fourcc: DrmFourcc) -> anyhow::Result<Format> {
    match fourcc {
        DrmFourcc::Abgr8888 | DrmFourcc::Xbgr8888 => Ok(Format::R8G8B8A8_UNORM),
        DrmFourcc::Argb8888 | DrmFourcc::Xrgb8888 => Ok(Format::B8G8R8A8_UNORM),
        DrmFourcc::Abgr2101010 | DrmFourcc::Xbgr2101010 => Ok(Format::A2B10G10R10_UNORM_PACK32),
        _ => anyhow::bail!("Unsupported format {fourcc}"),
    }
}
