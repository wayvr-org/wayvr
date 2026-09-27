use std::{error::Error, fmt, sync::Arc};

use super::{
	BuiltCommandBuffer, Image, ImageTiling, ImageUsage, ImageView, WGfx,
	raw::{RawAcquiredFrame, RawPresentStatus, RawSwapchain, RawSwapchainError},
};

pub struct Swapchain {
	gfx: Arc<WGfx>,
	raw: Arc<RawSwapchain>,
	image_views: Vec<Arc<ImageView>>,
}

impl Swapchain {
	pub(super) fn from_raw(
		gfx: Arc<WGfx>,
		raw: Arc<RawSwapchain>,
		raw_images: Vec<Arc<super::raw::RawImage>>,
	) -> anyhow::Result<Arc<Self>> {
		let extent = raw.extent();
		let format = raw.format();
		let mut image_views = Vec::with_capacity(raw_images.len());

		for raw_image in raw_images {
			let image = Arc::new(Image::from_raw(
				raw_image,
				[extent[0], extent[1], 1],
				format,
				ImageUsage::COLOR_ATTACHMENT,
				ImageTiling::Optimal,
				1,
			));
			image_views.push(gfx.create_image_view(image)?);
		}

		Ok(Arc::new(Self { gfx, raw, image_views }))
	}

	pub fn extent(&self) -> [u32; 2] {
		self.raw.extent()
	}

	pub const fn image_count(&self) -> usize {
		self.image_views.len()
	}

	pub fn recreate(self: &Arc<Self>, extent: [u32; 2]) -> anyhow::Result<Arc<Self>> {
		self.gfx.create_swapchain_inner(extent, Some(&self.raw))
	}

	pub fn acquire(self: &Arc<Self>) -> Result<SwapchainFrame, SwapchainError> {
		let raw = self.raw.acquire().map_err(SwapchainError::from_raw)?;
		let image_index = raw.image_index() as usize;
		let Some(image_view) = self.image_views.get(image_index).cloned() else {
			return Err(SwapchainError::Other(format!(
				"swapchain returned invalid image index {image_index}"
			)));
		};

		Ok(SwapchainFrame { raw, image_view })
	}
}

/// an acquired swapchain image. consume in `SwapchainFrame::present` to perform submission.
/// transitions from color-attachment to present layout automatically.
pub struct SwapchainFrame {
	raw: RawAcquiredFrame,
	image_view: Arc<ImageView>,
}

impl SwapchainFrame {
	pub const fn image_view(&self) -> &Arc<ImageView> {
		&self.image_view
	}

	pub fn present(self, command_buffer: &BuiltCommandBuffer) -> Result<PresentStatus, SwapchainError> {
		let Self { raw, image_view } = self;
		let mut command_buffer = command_buffer.raw.lock();
		raw
			.submit_and_present(&mut command_buffer, &image_view.image().raw)
			.map(PresentStatus::from_raw)
			.map_err(SwapchainError::from_raw)
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentStatus {
	Optimal,
	Suboptimal,
}

impl PresentStatus {
	const fn from_raw(status: RawPresentStatus) -> Self {
		match status {
			RawPresentStatus::Optimal => Self::Optimal,
			RawPresentStatus::Suboptimal => Self::Suboptimal,
		}
	}
}

#[derive(Debug)]
pub enum SwapchainError {
	OutOfDate,
	SurfaceLost,
	Other(String),
}

impl SwapchainError {
	fn from_raw(error: RawSwapchainError) -> Self {
		match error {
			RawSwapchainError::OutOfDate => Self::OutOfDate,
			RawSwapchainError::SurfaceLost => Self::SurfaceLost,
			RawSwapchainError::Other(error) => Self::Other(error),
		}
	}
}

impl fmt::Display for SwapchainError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::OutOfDate => f.write_str("swapchain is out of date"),
			Self::SurfaceLost => f.write_str("window surface was lost"),
			Self::Other(error) => f.write_str(error),
		}
	}
}

impl Error for SwapchainError {}
