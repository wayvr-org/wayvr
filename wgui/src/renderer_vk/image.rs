use std::{collections::HashMap, sync::Arc};

use cosmic_text::SubpixelBin;
use glam::Mat4;
use smallvec::smallvec;
use vulkano::{
	buffer::{BufferContents, BufferUsage},
	command_buffer::CommandBufferUsage,
	format::Format,
	image::view::ImageView,
	pipeline::graphics::{self, vertex_input::Vertex},
};

use crate::{
	drawing::{Boundary, ImagePrimitive},
	gfx::{
		cmd::GfxCommandBuffer,
		pipeline::{WGfxPipeline, WPipelineCreateInfo},
		WGfx, BLEND_ALPHA,
	},
	renderer_vk::{
		model_buffer::ModelBuffer,
		text::custom_glyph::{CustomGlyphData, RasterizeCustomGlyphRequest, RasterizedCustomGlyph},
	},
};

use super::viewport::Viewport;

#[repr(C)]
#[derive(BufferContents, Vertex, Copy, Clone, Debug)]
pub struct ImageVertex {
	#[format(R32_UINT)]
	pub in_model_idx: u32,
	#[format(R32_UINT)]
	pub in_rect_dim: [u16; 2],
	#[format(R32_UINT)]
	pub in_border_color: u32,
	#[format(R32_UINT)]
	pub round_border: [u8; 4],
}

/// Cloneable pipeline & shaders to be shared between `RectRenderer` instances.
#[derive(Clone)]
pub struct ImagePipeline {
	gfx: Arc<WGfx>,
	pub(super) inner: Arc<WGfxPipeline<ImageVertex>>,
}

impl ImagePipeline {
	pub fn new(gfx: Arc<WGfx>, format: Format) -> anyhow::Result<Self> {
		let vert = vert_image::load(gfx.device.clone())?;
		let frag = frag_image::load(gfx.device.clone())?;

		let pipeline = gfx.create_pipeline::<ImageVertex>(
			&vert,
			&frag,
			WPipelineCreateInfo::new(format)
				.use_blend(BLEND_ALPHA)
				.use_instanced()
				.use_updatable_descriptors(smallvec![2]),
		)?;

		Ok(Self { gfx, inner: pipeline })
	}
}

pub type ImageViewCache = HashMap<usize, CachedImageView>;

pub struct CachedImageView {
	view: Arc<ImageView>,
	res: [u32; 2],
}

struct ImageVertexWithContent {
	vert: ImageVertex,
	content: CustomGlyphData,
	content_key: usize, // identifies an image tag.
	skip_cache: bool,
}

pub struct ImageRenderer {
	pipeline: ImagePipeline,
	image_verts: Vec<ImageVertexWithContent>,
	model_buffer: ModelBuffer,
}

impl ImageRenderer {
	pub fn new(pipeline: ImagePipeline) -> anyhow::Result<Self> {
		Ok(Self {
			model_buffer: ModelBuffer::new(&pipeline.gfx)?,
			pipeline,
			image_verts: vec![],
		})
	}

	pub fn add_image(&mut self, boundary: Boundary, image: ImagePrimitive, transform: &Mat4) {
		let in_model_idx = self
			.model_buffer
			.register_pos_size(&boundary.pos, &boundary.size, transform);

		self.image_verts.push(ImageVertexWithContent {
			vert: ImageVertex {
				in_model_idx,
				in_rect_dim: [boundary.size.x as u16, boundary.size.y as u16],
				in_border_color: cosmic_text::Color::from(image.border_color).0,
				round_border: [
					image.round_units,
					(image.border) as u8,
					0, // unused
					0,
				],
			},
			content: image.content,
			content_key: image.content_key,
			skip_cache: image.skip_cache,
		});
	}

	fn upload_image(
		gfx: &Arc<WGfx>,
		res: [u32; 2],
		img: &ImageVertexWithContent,
	) -> anyhow::Result<Option<Arc<ImageView>>> {
		let Some(raster) = RasterizedCustomGlyph::try_from(&RasterizeCustomGlyphRequest {
			data: img.content.clone(),
			width: res[0] as _,
			height: res[1] as _,
			x_bin: SubpixelBin::Zero,
			y_bin: SubpixelBin::Zero,
			scale: 1.0, // unused
		}) else {
			log::error!("Unable to rasterize custom image");
			return Ok(None);
		};
		let mut cmd_buf = gfx.create_xfer_command_buffer(CommandBufferUsage::OneTimeSubmit)?;

		let image = cmd_buf.upload_image(
			raster.width.into(),
			raster.height.into(),
			Format::R8G8B8A8_UNORM,
			&raster.data,
		)?;
		let image_view = ImageView::new_default(image)?;

		cmd_buf.build_and_execute_now()?;

		Ok(Some(image_view))
	}

	pub fn render(
		&mut self,
		gfx: &Arc<WGfx>,
		viewport: &mut Viewport,
		vk_scissor: &graphics::viewport::Scissor,
		cmd_buf: &mut GfxCommandBuffer,
		image_view_cache: &mut ImageViewCache,
	) -> anyhow::Result<()> {
		let res = viewport.resolution();
		self.model_buffer.upload(gfx)?;

		for img in &self.image_verts {
			let image_view = if let Some(x) = image_view_cache.get_mut(&img.content_key) {
				if img.skip_cache || x.res != res {
					// image changed
					let Some(image_view) = Self::upload_image(&self.pipeline.gfx, res, img)? else {
						continue;
					};

					x.view = image_view;
					x.res = res;
				}

				x.view.clone()
			} else {
				let Some(image_view) = Self::upload_image(&self.pipeline.gfx, res, img)? else {
					continue;
				};

				image_view_cache.insert(img.content_key, CachedImageView { view: image_view, res });
				image_view_cache.get_mut(&img.content_key).unwrap().view.clone()
			};

			let vert_buffer = self.pipeline.gfx.empty_buffer(
				BufferUsage::VERTEX_BUFFER | BufferUsage::TRANSFER_DST,
				(std::mem::size_of::<ImageVertex>()) as _,
			)?;

			let set0 = viewport.get_image_descriptor(&self.pipeline);
			let set1 = self.model_buffer.get_image_descriptor(&self.pipeline);
			let set2 = self
				.pipeline
				.inner
				.uniform_sampler(2, image_view, self.pipeline.gfx.texture_filter)?;

			let pass = self.pipeline.inner.create_pass(
				[res[0] as _, res[1] as _],
				[0.0, 0.0],
				vert_buffer.clone(),
				0..4,
				0..1,
				vec![set0, set1, set2],
				vk_scissor,
			)?;

			vert_buffer.write()?[0..1].clone_from_slice(&[img.vert]);

			cmd_buf.run_ref(&pass)?;
		}

		Ok(())
	}
}

pub mod vert_image {
	vulkano_shaders::shader! {
			ty: "vertex",
			path: "src/renderer_vk/shaders/image.vert",
	}
}

pub mod frag_image {
	vulkano_shaders::shader! {
			ty: "fragment",
			path: "src/renderer_vk/shaders/image.frag",
	}
}
