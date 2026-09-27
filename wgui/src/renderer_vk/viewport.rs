use std::sync::Arc;

use crate::{
	gfx::{Buffer, BufferUsage, DescriptorSet, WGfx},
	renderer_vk::{image::ImagePipeline, util::WMat4},
};

use super::{rect::RectPipeline, text::text_atlas::TextPipeline};

/// Controls the visible area of all text for a given renderer. Any text outside of the visible
/// area will be clipped.
pub struct Viewport {
	params: Params,
	params_buffer: Arc<Buffer<Params>>,
	text_descriptor: Option<Arc<DescriptorSet>>,
	rect_descriptor: Option<Arc<DescriptorSet>>,
	image_descriptor: Option<Arc<DescriptorSet>>,
}

impl Viewport {
	/// Creates a new `Viewport` with the given graphics context.
	pub fn new(gfx: &Arc<WGfx>) -> anyhow::Result<Self> {
		let params = Params {
			screen_resolution: [0, 0],
			pixel_scale: 1.0,
			padding1: [0.0],
			projection: WMat4::default(),
		};

		let params_buffer = gfx.new_buffer(BufferUsage::UNIFORM_BUFFER | BufferUsage::TRANSFER_DST, &[params])?;

		Ok(Self {
			params,
			params_buffer,
			text_descriptor: None,
			rect_descriptor: None,
			image_descriptor: None,
		})
	}

	pub fn get_text_descriptor(&mut self, pipeline: &TextPipeline) -> Arc<DescriptorSet> {
		self
			.text_descriptor
			.get_or_insert_with(|| pipeline.inner.buffer(2, self.params_buffer.clone()).unwrap())
			.clone()
	}

	pub fn get_rect_descriptor(&mut self, pipeline: &RectPipeline) -> Arc<DescriptorSet> {
		self
			.rect_descriptor
			.get_or_insert_with(|| pipeline.color_rect.buffer(0, self.params_buffer.clone()).unwrap())
			.clone()
	}

	pub fn get_image_descriptor(&mut self, pipeline: &ImagePipeline) -> Arc<DescriptorSet> {
		self
			.image_descriptor
			.get_or_insert_with(|| pipeline.inner.buffer(0, self.params_buffer.clone()).unwrap())
			.clone()
	}

	/// Updates the `Viewport` with the given `resolution` and `projection`.
	pub fn update(&mut self, resolution: [u32; 2], projection: &glam::Mat4, pixel_scale: f32) -> anyhow::Result<()> {
		if self.params.screen_resolution == resolution
			&& self.params.projection.0 == *projection.as_ref()
			&& self.params.pixel_scale == pixel_scale
		{
			return Ok(());
		}

		self.params.screen_resolution = resolution;
		self.params.projection = WMat4::from_glam(projection);
		self.params.pixel_scale = pixel_scale;
		self.params_buffer.write()?.copy_from_slice(&[self.params]);
		Ok(())
	}

	/// Returns the current resolution of the `Viewport`.
	pub const fn resolution(&self) -> [u32; 2] {
		self.params.screen_resolution
	}
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Params {
	pub screen_resolution: [u32; 2],
	pub pixel_scale: f32,
	pub padding1: [f32; 1], // always zero

	pub projection: WMat4,
}
