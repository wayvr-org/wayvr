use std::{
	collections::HashMap,
	sync::{Arc, Weak},
};

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
		BLEND_ALPHA, WGfx,
		cmd::GfxCommandBuffer,
		pipeline::{WGfxPipeline, WPipelineCreateInfo},
	},
	renderer_vk::{
		model_buffer::ModelBuffer,
		text::custom_glyph::{CustomGlyphContent, CustomGlyphData, RasterizeCustomGlyphRequest, RasterizedCustomGlyph},
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
				round_border: [
					image.round_units,
					(image.border) as u8,
					0, // unused
					0,
				],
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
			scale: 1.0, // unused
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
		vk_scissor: &graphics::viewport::Scissor,
		cmd_buf: &mut GfxCommandBuffer,
		image_view_cache: &mut ImageViewCache,
	) -> anyhow::Result<()> {
		let res = viewport.resolution();
		self.model_buffer.upload(gfx)?;

		let mut pending_upload_by_key = HashMap::<usize, usize>::new();
		let mut pending_uploads = Vec::<PendingImageUpload>::new();
		let mut image_sources = Vec::<ImageViewSource>::with_capacity(self.image_verts.len());

		// decide which images need to be rasterized and uploaded
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

		// upload every missing/stale image using one transfer command buffer
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
				uploaded_image_views[upload_idx] = Some(ImageView::new_default(image)?);
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

		// run the rendering work
		for (img, image_source) in self.image_verts.iter().zip(image_sources.iter()) {
			let image_view = match image_source {
				ImageViewSource::Ready(image_view) => image_view.clone(),
				ImageViewSource::PendingUpload(upload_idx, ..) => {
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
