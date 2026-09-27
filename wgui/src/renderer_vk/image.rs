use std::{
	collections::HashMap,
	sync::{Arc, Weak},
};

use cosmic_text::SubpixelBin;
use glam::Mat4;

use crate::{
	drawing::{Boundary, ImagePrimitive},
	gfx::{
		BLEND_ALPHA, BufferUsage, CommandBufferUsage, DescriptorBinding, DescriptorType, Format, ImageView, Scissor,
		ShaderModule, ShaderStage, Vertex, VertexAttribute, VertexFormat, WGfx,
		cmd::GfxCommandBufferBuilder,
		pipeline::{WGfxPipeline, WPipelineCreateInfo},
	},
	renderer_vk::{
		model_buffer::ModelBuffer,
		text::custom_glyph::{CustomGlyphContent, CustomGlyphData, RasterizeCustomGlyphRequest, RasterizedCustomGlyph},
	},
};

use super::viewport::Viewport;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct ImageVertex {
	pub in_model_idx: u32,
	pub in_rect_dim: [u16; 2],
	pub in_border_color: u32,
	pub round_border: [u8; 4],
}

impl Vertex for ImageVertex {
	fn attributes() -> Vec<VertexAttribute> {
		vec![
			VertexAttribute::new(
				0,
				VertexFormat::R32Uint,
				std::mem::offset_of!(Self, in_model_idx) as u32,
			),
			VertexAttribute::new(1, VertexFormat::R32Uint, std::mem::offset_of!(Self, in_rect_dim) as u32),
			VertexAttribute::new(
				2,
				VertexFormat::R32Uint,
				std::mem::offset_of!(Self, in_border_color) as u32,
			),
			VertexAttribute::new(
				3,
				VertexFormat::R32Uint,
				std::mem::offset_of!(Self, round_border) as u32,
			),
		]
	}
}

/// Cloneable pipeline & shaders to be shared between `ImageRenderer` instances.
#[derive(Clone)]
pub struct ImagePipeline {
	gfx: Arc<WGfx>,
	pub(super) inner: Arc<WGfxPipeline<ImageVertex>>,
}

impl ImagePipeline {
	pub fn new(gfx: Arc<WGfx>, format: Format) -> anyhow::Result<Self> {
		let vert = vert_image::load(&gfx)?;
		let frag = frag_image::load(&gfx)?;

		let pipeline = gfx.create_pipeline::<ImageVertex>(
			&vert,
			&frag,
			WPipelineCreateInfo::new(format)
				.use_blend(BLEND_ALPHA)
				.use_instanced()
				.use_updatable_descriptors([2]),
		)?;

		Ok(Self { gfx, inner: pipeline })
	}
}

pub type ImageViewCache = HashMap<usize, CachedImageView>;

pub struct CachedImageView {
	pub(super) content: Weak<CustomGlyphContent>,
	view: Arc<ImageView>,
	res: [u32; 2],
}

struct ImageVertexWithContent {
	vert: ImageVertex,
	content: CustomGlyphData,
	skip_cache: bool,
}

struct PendingImageUpload {
	content_id: usize,
	content: Weak<CustomGlyphContent>,
	raster: RasterizedCustomGlyph,
}

enum ImageViewSource {
	Ready(Arc<ImageView>),
	PendingUpload(usize),
	Missing,
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
				round_border: [image.round_units, (image.border) as u8, 0, 0],
			},
			content: image.content,
			skip_cache: image.skip_cache,
		});
	}

	fn rasterize_image(res: [u32; 2], img: &ImageVertexWithContent) -> Option<RasterizedCustomGlyph> {
		let Some(raster) = RasterizedCustomGlyph::try_from(&RasterizeCustomGlyphRequest {
			data: img.content.clone(),
			width: res[0] as _,
			height: res[1] as _,
			x_bin: SubpixelBin::Zero,
			y_bin: SubpixelBin::Zero,
			scale: 1.0,
		}) else {
			log::error!("Unable to rasterize custom image");
			return None;
		};

		Some(raster)
	}

	pub fn render(
		&mut self,
		gfx: &Arc<WGfx>,
		viewport: &mut Viewport,
		gfx_scissor: &Scissor,
		cmd_buf: &mut GfxCommandBufferBuilder,
		image_view_cache: &mut ImageViewCache,
	) -> anyhow::Result<()> {
		let res = viewport.resolution();
		self.model_buffer.upload(gfx)?;

		let mut pending_upload_by_key = HashMap::<usize, usize>::new();
		let mut pending_uploads = Vec::<PendingImageUpload>::new();
		let mut image_sources = Vec::<ImageViewSource>::with_capacity(self.image_verts.len());

		for img in &self.image_verts {
			if let Some(upload_idx) = pending_upload_by_key.get(&img.content.id) {
				image_sources.push(ImageViewSource::PendingUpload(*upload_idx));
				continue;
			}

			if let Some(cached) = image_view_cache.get(&img.content.id)
				&& !img.skip_cache
				&& cached.res == res
			{
				image_sources.push(ImageViewSource::Ready(cached.view.clone()));
				continue;
			}

			let Some(raster) = Self::rasterize_image(res, img) else {
				image_sources.push(ImageViewSource::Missing);
				continue;
			};

			let upload_idx = pending_uploads.len();
			pending_uploads.push(PendingImageUpload {
				content: Arc::downgrade(&img.content.content),
				content_id: img.content.id,
				raster,
			});
			pending_upload_by_key.insert(img.content.id, upload_idx);
			image_sources.push(ImageViewSource::PendingUpload(upload_idx));
		}

		let mut uploaded_image_views = vec![None; pending_uploads.len()];

		if !pending_uploads.is_empty() {
			let mut xfer_cmd_buf = gfx.create_xfer_command_buffer(CommandBufferUsage::OneTimeSubmit)?;

			for (upload_idx, upload) in pending_uploads.iter().enumerate() {
				log::trace!("Uploading image {}", upload.content_id);
				let image = xfer_cmd_buf.upload_image(
					upload.raster.width.into(),
					upload.raster.height.into(),
					Format::R8G8B8A8_UNORM,
					&upload.raster.data,
				)?;
				uploaded_image_views[upload_idx] = Some(gfx.create_image_view(image)?);
			}

			xfer_cmd_buf.build_and_execute_now()?;

			for (upload_idx, upload) in pending_uploads.iter().enumerate() {
				let Some(image_view) = uploaded_image_views[upload_idx].as_ref() else {
					continue;
				};

				image_view_cache.insert(
					upload.content_id,
					CachedImageView {
						content: upload.content.clone(),
						view: image_view.clone(),
						res,
					},
				);
			}
		}

		for (img, image_source) in self.image_verts.iter().zip(image_sources.iter()) {
			let image_view = match image_source {
				ImageViewSource::Ready(image_view) => image_view.clone(),
				ImageViewSource::PendingUpload(upload_idx) => {
					let Some(image_view) = uploaded_image_views
						.get(*upload_idx)
						.and_then(|image_view| image_view.as_ref())
					else {
						continue;
					};
					image_view.clone()
				}
				ImageViewSource::Missing => continue,
			};

			let vert_buffer = self
				.pipeline
				.gfx
				.empty_buffer(BufferUsage::VERTEX_BUFFER | BufferUsage::TRANSFER_DST, 1)?;

			let set0 = viewport.get_image_descriptor(&self.pipeline);
			let set1 = self.model_buffer.get_image_descriptor(&self.pipeline);
			let set2 = self
				.pipeline
				.inner
				.uniform_sampler(2, image_view, self.pipeline.gfx.texture_filter())?;

			let pass = self.pipeline.inner.create_pass(
				[res[0] as _, res[1] as _],
				[0.0, 0.0],
				vert_buffer.clone(),
				0..4,
				0..1,
				vec![set0, set1, set2],
				*gfx_scissor,
			)?;

			vert_buffer.write()?[0..1].clone_from_slice(&[img.vert]);
			cmd_buf.run_ref(&pass)?;
		}

		Ok(())
	}
}

pub mod vert_image {
	use super::{Arc, DescriptorBinding, DescriptorType, ShaderModule, ShaderStage, WGfx};

	pub fn load(gfx: &WGfx) -> anyhow::Result<Arc<ShaderModule>> {
		gfx.create_shader_module_bytes(
			include_bytes!(concat!(env!("OUT_DIR"), "/image.vert.spv")),
			ShaderStage::Vertex,
			&[
				DescriptorBinding::new(0, 0, DescriptorType::UniformBuffer, ShaderStage::Vertex),
				DescriptorBinding::new(1, 0, DescriptorType::StorageBuffer, ShaderStage::Vertex),
			],
		)
	}
}

pub mod frag_image {
	use super::{Arc, DescriptorBinding, DescriptorType, ShaderModule, ShaderStage, WGfx};

	pub fn load(gfx: &WGfx) -> anyhow::Result<Arc<ShaderModule>> {
		gfx.create_shader_module_bytes(
			include_bytes!(concat!(env!("OUT_DIR"), "/image.frag.spv")),
			ShaderStage::Fragment,
			&[
				DescriptorBinding::new(0, 0, DescriptorType::UniformBuffer, ShaderStage::Fragment),
				DescriptorBinding::new(2, 0, DescriptorType::CombinedImageSampler, ShaderStage::Fragment),
			],
		)
	}
}
