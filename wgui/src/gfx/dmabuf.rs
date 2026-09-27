use std::{fs::File, os::fd::RawFd, sync::Arc};

use super::{Format, Image, ImageTiling, ImageUsage, ImageView, WGfx};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmabufPlaneLayout {
	pub offset: u64,
	pub row_pitch: u64,
}

#[derive(Debug, Clone)]
pub struct DmabufImportInfo {
	pub extent: [u32; 3],
	pub format: Format,
	pub fd: RawFd,
	pub modifier: Option<u64>,
	pub plane_layouts: Vec<DmabufPlaneLayout>,
}

pub struct ExportedDmabufImage {
	pub view: Arc<ImageView>,
	pub fd: File,
	pub offset: u64,
	pub stride: u64,
	pub modifier: u64,
}

impl WGfx {
	pub fn import_dmabuf(&self, info: &DmabufImportInfo) -> anyhow::Result<Arc<Image>> {
		if info.extent.contains(&0) {
			anyhow::bail!("DMA-buf image extent must be non-zero");
		}
		if info.modifier.is_some() && info.plane_layouts.is_empty() {
			anyhow::bail!("explicit DMA-buf modifiers require at least one plane layout");
		}

		let raw = self.raw.import_dmabuf(info)?;
		Ok(Arc::new(Image::from_raw(
			raw,
			info.extent,
			info.format,
			ImageUsage::SAMPLED,
			if info.modifier.is_some() {
				ImageTiling::DrmFormatModifier
			} else {
				ImageTiling::Optimal
			},
			1,
		)))
	}

	pub fn export_dmabuf_image(
		&self,
		extent: [u32; 3],
		format: Format,
		modifier: u64,
	) -> anyhow::Result<ExportedDmabufImage> {
		if extent.contains(&0) {
			anyhow::bail!("DMA-buf image extent must be non-zero");
		}

		let (raw, fd, offset, stride) = self.raw.export_dmabuf_image(extent, format, modifier)?;
		let image = Arc::new(Image::from_raw(
			raw,
			extent,
			format,
			ImageUsage::TRANSFER_DST | ImageUsage::TRANSFER_SRC | ImageUsage::SAMPLED,
			ImageTiling::DrmFormatModifier,
			1,
		));
		let view = self.create_image_view(image)?;

		Ok(ExportedDmabufImage {
			view,
			fd,
			offset,
			stride,
			modifier,
		})
	}

	pub fn dmabuf_import_modifiers(&self, format: Format) -> anyhow::Result<Vec<u64>> {
		self.raw.dmabuf_import_modifiers(format)
	}
}
