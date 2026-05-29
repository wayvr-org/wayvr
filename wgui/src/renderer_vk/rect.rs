use std::sync::Arc;

use glam::Mat4;
use vulkano::{
	buffer::{BufferContents, BufferUsage, Subbuffer},
	format::Format,
	pipeline::graphics::{self, vertex_input::Vertex},
};

use crate::{
	drawing::{Boundary, Rectangle},
	gfx::{
		BLEND_ALPHA, WGfx,
		cmd::GfxCommandBuffer,
		pipeline::{WGfxPipeline, WPipelineCreateInfo},
	},
	renderer_vk::model_buffer::ModelBuffer,
};

use super::viewport::Viewport;

#[repr(C)]
#[derive(BufferContents, Vertex, Copy, Clone, Debug)]
pub struct RectVertex {
	#[format(R32_UINT)]
	pub in_model_idx: u32,
	#[format(R32_UINT)]
	pub in_rect_dim: [u16; 2],
	#[format(R32_UINT)]
	pub in_color: u32,
	#[format(R32_UINT)]
	pub in_color2: u32,
	#[format(R32_UINT)]
	pub in_border_color: u32,
	#[format(R32_UINT)]
	pub round_border_gradient: [u8; 4],
}

/// Cloneable pipeline & shaders to be shared between `RectRenderer` instances.
#[derive(Clone)]
pub struct RectPipeline {
	gfx: Arc<WGfx>,
	pub(super) color_rect: Arc<WGfxPipeline<RectVertex>>,
}

impl RectPipeline {
	pub fn new(gfx: Arc<WGfx>, format: Format) -> anyhow::Result<Self> {
		let vert = vert_rect::load(gfx.device.clone())?;
		let frag = frag_rect::load(gfx.device.clone())?;

		let color_rect = gfx.create_pipeline::<RectVertex>(
			&vert,
			&frag,
			WPipelineCreateInfo::new(format).use_blend(BLEND_ALPHA).use_instanced(),
		)?;

		Ok(Self { gfx, color_rect })
	}
}

pub struct RectRenderer {
	pipeline: RectPipeline,
	rect_vertices: Vec<RectVertex>,
	vert_buffer: Subbuffer<[RectVertex]>,
	vert_buffer_len: usize,
	model_buffer: ModelBuffer,
}

impl RectRenderer {
	pub fn new(pipeline: RectPipeline) -> anyhow::Result<Self> {
		const BUFFER_SIZE: usize = 32;

		let vert_buffer = pipeline.gfx.empty_buffer(
			BufferUsage::VERTEX_BUFFER | BufferUsage::TRANSFER_DST,
			(std::mem::size_of::<RectVertex>() * BUFFER_SIZE) as _,
		)?;

		Ok(Self {
			model_buffer: ModelBuffer::new(&pipeline.gfx)?,
			pipeline,
			rect_vertices: vec![],
			vert_buffer,
			vert_buffer_len: BUFFER_SIZE,
		})
	}

	pub fn add_rect(&mut self, boundary: Boundary, rectangle: Rectangle, transform: &Mat4) {
		let in_model_idx = self
			.model_buffer
			.register_pos_size(&boundary.pos, &boundary.size, transform);

		self.rect_vertices.push(RectVertex {
			in_model_idx,
			in_rect_dim: [boundary.size.x as u16, boundary.size.y as u16],
			in_color: cosmic_text::Color::from(rectangle.color).0,
			in_color2: cosmic_text::Color::from(rectangle.color2).0,
			in_border_color: cosmic_text::Color::from(rectangle.border_color).0,
			round_border_gradient: [
				rectangle.round_units,
				(rectangle.border) as u8,
				rectangle.gradient as u8,
				0, // unused
			],
		});
	}

	fn upload_verts(&mut self) -> anyhow::Result<()> {
		if self.vert_buffer_len < self.rect_vertices.len() {
			let new_size = self.vert_buffer_len * 2;
			self.vert_buffer = self.pipeline.gfx.empty_buffer(
				BufferUsage::VERTEX_BUFFER | BufferUsage::TRANSFER_DST,
				(std::mem::size_of::<RectVertex>() * new_size) as _,
			)?;
			self.vert_buffer_len = new_size;
		}

		self.vert_buffer.write()?[0..self.rect_vertices.len()].clone_from_slice(&self.rect_vertices);

		Ok(())
	}

	pub fn render(
		&mut self,
		gfx: &Arc<WGfx>,
		viewport: &mut Viewport,
		vk_scissor: &graphics::viewport::Scissor,
		cmd_buf: &mut GfxCommandBuffer,
	) -> anyhow::Result<()> {
		let res = viewport.resolution();

		self.model_buffer.upload(gfx)?;
		self.upload_verts()?;

		let set0 = viewport.get_rect_descriptor(&self.pipeline);
		let set1 = self.model_buffer.get_rect_descriptor(&self.pipeline);
		let pass = self.pipeline.color_rect.create_pass(
			[res[0] as _, res[1] as _],
			[0.0, 0.0],
			self.vert_buffer.clone(),
			0..4,
			0..self.rect_vertices.len() as _,
			vec![set0, set1],
			vk_scissor,
		)?;

		self.rect_vertices.clear();
		cmd_buf.run_ref(&pass)?;
		Ok(())
	}
}

pub mod vert_rect {
	vulkano_shaders::shader! {
			ty: "vertex",
			path: "src/renderer_vk/shaders/rect.vert",
	}
}

pub mod frag_rect {
	vulkano_shaders::shader! {
			ty: "fragment",
			path: "src/renderer_vk/shaders/rect.frag",
	}
}
