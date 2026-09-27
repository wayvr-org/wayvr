use image_dds::{ImageFormat, Surface};
use std::{io::Read, sync::Arc};
use wgui::gfx::{Format, Image, cmd::XferCommandBufferBuilder};

pub trait WlxCommandBufferDds {
    fn upload_image_dds<R>(&mut self, r: R) -> anyhow::Result<Arc<Image>>
    where
        R: Read;
}

impl WlxCommandBufferDds for XferCommandBufferBuilder {
    fn upload_image_dds<R>(&mut self, r: R) -> anyhow::Result<Arc<Image>>
    where
        R: Read,
    {
        let Ok(dds) = image_dds::ddsfile::Dds::read(r) else {
            anyhow::bail!(
                "Not a valid DDS file.\nSee: https://github.com/galister/wlx-overlay-s/wiki/Custom-Textures"
            );
        };

        let surface = Surface::from_dds(&dds)?;

        if surface.depth != 1 {
            anyhow::bail!("Not a 2D texture.")
        }

        self.upload_image(
            surface.width,
            surface.height,
            dds_to_wgfx(surface.image_format)?,
            surface.data,
        )
    }
}

pub fn dds_to_wgfx(dds_fmt: ImageFormat) -> anyhow::Result<Format> {
    match dds_fmt {
        ImageFormat::R8Unorm => Ok(Format::R8_UNORM),
        ImageFormat::Rgba8Unorm => Ok(Format::R8G8B8A8_UNORM),
        ImageFormat::Rgba8UnormSrgb => Ok(Format::R8G8B8A8_SRGB),
        ImageFormat::Rgba16Float => Ok(Format::R16G16B16A16_SFLOAT),
        ImageFormat::Rgba32Float => Ok(Format::R32G32B32A32_SFLOAT),
        ImageFormat::Bgr8Unorm => Ok(Format::B8G8R8_UNORM),
        ImageFormat::Bgra8Unorm => Ok(Format::B8G8R8A8_UNORM),
        ImageFormat::Bgra8UnormSrgb => Ok(Format::B8G8R8A8_SRGB),
        // DXT1
        ImageFormat::BC1RgbaUnorm => Ok(Format::BC1_RGBA_UNORM_BLOCK),
        ImageFormat::BC1RgbaUnormSrgb => Ok(Format::BC1_RGBA_SRGB_BLOCK),
        // DXT3
        ImageFormat::BC2RgbaUnorm => Ok(Format::BC2_UNORM_BLOCK),
        ImageFormat::BC2RgbaUnormSrgb => Ok(Format::BC2_SRGB_BLOCK),
        // DXT5
        ImageFormat::BC3RgbaUnorm => Ok(Format::BC3_UNORM_BLOCK),
        ImageFormat::BC3RgbaUnormSrgb => Ok(Format::BC3_SRGB_BLOCK),
        // RGTC1
        ImageFormat::BC4RUnorm => Ok(Format::BC4_UNORM_BLOCK),
        ImageFormat::BC4RSnorm => Ok(Format::BC4_SNORM_BLOCK),
        // RGTC2
        ImageFormat::BC5RgUnorm => Ok(Format::BC5_UNORM_BLOCK),
        ImageFormat::BC5RgSnorm => Ok(Format::BC5_SNORM_BLOCK),
        // BPTC
        ImageFormat::BC6hRgbUfloat => Ok(Format::BC6H_UFLOAT_BLOCK),
        ImageFormat::BC6hRgbSfloat => Ok(Format::BC6H_SFLOAT_BLOCK),
        // BPTC
        ImageFormat::BC7RgbaUnorm => Ok(Format::BC7_UNORM_BLOCK),
        ImageFormat::BC7RgbaUnormSrgb => Ok(Format::BC7_SRGB_BLOCK),
        _ => anyhow::bail!("Unsupported format {dds_fmt:?}"),
    }
}
