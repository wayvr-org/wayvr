use std::sync::Arc;

use glam::{Mat4, Vec3};

use crate::{
	gfx::{self, Buffer, BufferUsage, DescriptorSet},
	renderer_vk::{image::ImagePipeline, rect::RectPipeline, text::text_atlas::TextPipeline},
};

pub struct ModelBuffer {
	idx: u32,
	models: Vec<glam::Mat4>,

	buffer: Arc<Buffer<f32>>, // 4x4 floats = 1 mat4
	buffer_capacity_f32: u32,

	rect_descriptor: Option<Arc<DescriptorSet>>,
	text_descriptor: Option<Arc<DescriptorSet>>,
	image_descriptor: Option<Arc<DescriptorSet>>,
}

impl ModelBuffer {
	pub fn new(gfx: &Arc<gfx::WGfx>) -> anyhow::Result<Self> {
		const INITIAL_CAPACITY_MAT4: u32 = 16;
		const INITIAL_CAPACITY_F32: u32 = INITIAL_CAPACITY_MAT4 * (4 * 4);

		let buffer = gfx.empty_buffer::<f32>(
			BufferUsage::STORAGE_BUFFER | BufferUsage::TRANSFER_DST,
			INITIAL_CAPACITY_F32.into(),
		)?;

		let mut models = Vec::<glam::Mat4>::new();
		models.resize(INITIAL_CAPACITY_MAT4 as _, Default::default());

		Ok(Self {
			models,
			idx: 0,
			buffer,
			buffer_capacity_f32: INITIAL_CAPACITY_F32,
			rect_descriptor: None,
			text_descriptor: None,
			image_descriptor: None,
		})
	}

	pub const fn begin(&mut self) {
		self.idx = 0;
	}

	pub fn upload(&mut self, gfx: &Arc<gfx::WGfx>) -> anyhow::Result<()> {
		let required_capacity_f32 = (self.models.len() * (4 * 4)) as u32;

		if self.buffer_capacity_f32 < required_capacity_f32 {
			self.buffer_capacity_f32 = required_capacity_f32;
			self.buffer = gfx.empty_buffer::<f32>(
				BufferUsage::STORAGE_BUFFER | BufferUsage::TRANSFER_DST,
				required_capacity_f32.into(),
			)?;
			self.rect_descriptor = None;
			self.text_descriptor = None;
			self.image_descriptor = None;
		}

		let floats = unsafe {
			std::slice::from_raw_parts(
				self.models.as_slice().as_ptr().cast::<f32>(),
				required_capacity_f32 as usize,
			)
		};

		self.buffer.write()?.copy_from_slice(floats);
		Ok(())
	}

	pub fn register(&mut self, model: &glam::Mat4) -> u32 {
		if self.idx == self.models.len() as u32 {
			self.models.resize((self.models.len() * 2).max(1), Default::default());
		}

		self.models[self.idx as usize] = *model;
		let ret = self.idx;
		self.idx += 1;
		ret
	}

	pub fn register_pos_size(&mut self, pos: &glam::Vec2, size: &glam::Vec2, transform: &Mat4) -> u32 {
		let mut model = glam::Mat4::from_translation(Vec3::new(pos.x, pos.y, 0.0));
		model *= *transform;
		model *= glam::Mat4::from_scale(Vec3::new(size.x, size.y, 1.0));
		self.register(&model)
	}

	pub fn get_rect_descriptor(&mut self, pipeline: &RectPipeline) -> Arc<DescriptorSet> {
		self
			.rect_descriptor
			.get_or_insert_with(|| pipeline.color_rect.buffer(1, self.buffer.clone()).unwrap())
			.clone()
	}

	pub fn get_text_descriptor(&mut self, pipeline: &TextPipeline) -> Arc<DescriptorSet> {
		self
			.text_descriptor
			.get_or_insert_with(|| pipeline.inner.buffer(3, self.buffer.clone()).unwrap())
			.clone()
	}

	pub fn get_image_descriptor(&mut self, pipeline: &ImagePipeline) -> Arc<DescriptorSet> {
		self
			.image_descriptor
			.get_or_insert_with(|| pipeline.inner.buffer(1, self.buffer.clone()).unwrap())
			.clone()
	}
}
