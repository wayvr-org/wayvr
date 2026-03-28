use cosmic_text::{FontSystem, SwashCache};
use etagere::{Allocation, BucketedAtlasAllocator, size2};
use lru::LruCache;
use rustc_hash::FxHasher;
use std::{collections::HashSet, hash::BuildHasherDefault, sync::Arc};
use vulkano::{
	buffer::BufferContents,
	command_buffer::CommandBufferUsage,
	descriptor_set::DescriptorSet,
	format::Format,
	image::{Image, ImageCreateInfo, ImageType, ImageUsage, view::ImageView},
	memory::allocator::AllocationCreateInfo,
	pipeline::graphics::vertex_input::Vertex,
};

use super::{
	GlyphDetails, GpuCacheStatus,
	custom_glyph::ContentType,
	shaders::{frag_atlas, vert_atlas},
	text_renderer::GlyphonCacheKey,
};
use crate::gfx::{
	BLEND_ALPHA, WGfx,
	pipeline::{WGfxPipeline, WPipelineCreateInfo},
};

/// Pipeline & shaders to be reused between `TextRenderer` instances
#[derive(Clone)]
pub struct TextPipeline {
	pub(super) gfx: Arc<WGfx>,
	pub(in super::super) inner: Arc<WGfxPipeline<GlyphVertex>>,
}

impl TextPipeline {
	pub fn new(gfx: Arc<WGfx>, format: Format) -> anyhow::Result<Self> {
		let vert = vert_atlas::load(gfx.device.clone())?;
		let frag = frag_atlas::load(gfx.device.clone())?;

		let pipeline = gfx.create_pipeline::<GlyphVertex>(
			&vert,
			&frag,
			WPipelineCreateInfo::new(format).use_blend(BLEND_ALPHA).use_instanced(),
		)?;

		Ok(Self { gfx, inner: pipeline })
	}
}

#[repr(C)]
#[derive(BufferContents, Vertex, Copy, Clone, Debug, Default)]
pub struct GlyphVertex {
	#[format(R32_UINT)]
	pub in_model_idx: u32,
	#[format(R32_UINT)]
	pub in_rect_dim: [u16; 2],
	#[format(R32_UINT)]
	pub in_uv: [u16; 2],
	#[format(R32_UINT)]
	pub in_color: u32,
	#[format(R32_UINT)]
	pub in_content_type: [u16; 2], // 2 bytes unused! TODO
	#[format(R32_SFLOAT)]
	pub scale: f32,
}

type Hasher = BuildHasherDefault<FxHasher>;

pub(super) struct InnerAtlas {
	pub kind: Kind,
	pub image_view: Arc<ImageView>,
	pub image_descriptor: Arc<DescriptorSet>,
	pub packer: BucketedAtlasAllocator,
	pub size: u32,
	pub glyph_cache: LruCache<GlyphonCacheKey, GlyphDetails, Hasher>,
	pub glyphs_in_use: HashSet<GlyphonCacheKey, Hasher>,
	pub max_texture_dimension_2d: u32,
	common: TextPipeline,
}

pub const TEXT_ATLAS_ISLAND_PADDING_PX: u32 = 1;

impl InnerAtlas {
	const INITIAL_SIZE: u32 = 256;

	fn new(common: TextPipeline, kind: Kind) -> anyhow::Result<Self> {
		let max_texture_dimension_2d = common.gfx.device.physical_device().properties().max_image_dimension2_d;
		let size = Self::INITIAL_SIZE.min(max_texture_dimension_2d);

		let packer = BucketedAtlasAllocator::new(size2(size as i32, size as i32));

		// Create a texture to use for our atlas
		let image = Image::new(
			common.gfx.memory_allocator.clone(),
			ImageCreateInfo {
				image_type: ImageType::Dim2d,
				format: kind.texture_format(),
				extent: [size, size, 1],
				usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_SRC | ImageUsage::TRANSFER_DST,
				..Default::default()
			},
			AllocationCreateInfo::default(),
		)?;

		let image_view = ImageView::new_default(image).unwrap();

		let image_descriptor = common.inner.uniform_sampler(
			Self::descriptor_set(kind),
			image_view.clone(),
			common.gfx.texture_filter,
		)?;

		let glyph_cache = LruCache::unbounded_with_hasher(Hasher::default());
		let glyphs_in_use = HashSet::with_hasher(Hasher::default());

		Ok(Self {
			kind,
			image_view,
			image_descriptor,
			packer,
			size,
			glyph_cache,
			glyphs_in_use,
			max_texture_dimension_2d,
			common,
		})
	}

	const fn descriptor_set(kind: Kind) -> usize {
		match kind {
			Kind::Color => 0,
			Kind::Mask => 1,
		}
	}

	pub(super) fn try_allocate(&mut self, width: usize, height: usize) -> Option<Allocation> {
		let glyph_size = size2(width as i32, height as i32);
		let padded_size = glyph_size
			+ size2(
				(TEXT_ATLAS_ISLAND_PADDING_PX * 2) as i32,
				(TEXT_ATLAS_ISLAND_PADDING_PX * 2) as i32,
			);

		loop {
			let allocation = self.packer.allocate(padded_size);

			if allocation.is_some() {
				return allocation;
			}

			// Try to free least recently used allocation
			let (mut key, mut value) = self.glyph_cache.peek_lru()?;

			// Find a glyph with an actual size
			while value.atlas_id.is_none() {
				// All sized glyphs are in use, cache is full
				if self.glyphs_in_use.contains(key) {
					return None;
				}

				let _ = self.glyph_cache.pop_lru();

				(key, value) = self.glyph_cache.peek_lru()?;
			}

			// All sized glyphs are in use, cache is full
			if self.glyphs_in_use.contains(key) {
				return None;
			}

			let (_, value) = self.glyph_cache.pop_lru().unwrap();
			self.packer.deallocate(value.atlas_id.unwrap());
		}
	}

	#[allow(dead_code)]
	pub const fn num_channels(&self) -> usize {
		self.kind.num_channels()
	}

	pub(super) fn grow(&mut self, font_system: &mut FontSystem, cache: &mut SwashCache) -> anyhow::Result<bool> {
		const GROWTH_FACTOR: u32 = 2;

		if self.size >= self.max_texture_dimension_2d {
			return Ok(false);
		}

		// Grow each dimension by a factor of 2. The growth factor was chosen to match the growth
		// factor of `Vec`.`
		let new_size = (self.size * GROWTH_FACTOR).min(self.max_texture_dimension_2d);
		log::debug!("Grow {:?} atlas {} → {new_size}", self.kind, self.size);

		self.packer.grow(size2(new_size as i32, new_size as i32));

		let old_image = self.image_view.image().clone();

		let image = self.common.gfx.new_image(
			new_size,
			new_size,
			old_image.format(),
			ImageUsage::SAMPLED | ImageUsage::TRANSFER_SRC | ImageUsage::TRANSFER_DST,
		)?;

		self.image_view = ImageView::new_default(image.clone()).unwrap();

		let mut cmd_buf = self
			.common
			.gfx
			.create_xfer_command_buffer(CommandBufferUsage::OneTimeSubmit)?;

		// clear newly allocated image with zeros
		cmd_buf.clear_image(&image)?;

		// Re-upload glyphs
		for (&cache_key, glyph) in &self.glyph_cache {
			let (x, y) = match glyph.gpu_cache {
				GpuCacheStatus::InAtlas { x, y, .. } => (x, y),
				GpuCacheStatus::SkipRasterization => continue,
			};

			let (width, height) = match cache_key {
				GlyphonCacheKey::Text(cache_key) => {
					let image = cache.get_image_uncached(font_system, cache_key).unwrap();
					let width = image.placement.width as usize;
					let height = image.placement.height as usize;
					(width, height)
				}
				GlyphonCacheKey::Custom(cache_key) => (cache_key.width as usize, cache_key.height as usize),
			};

			let offset = [x.into(), y.into(), 0];
			cmd_buf.copy_image(
				&old_image.clone(),
				offset,
				&image.clone(),
				offset,
				Some([width as _, height as _, 1]),
			)?;
		}
		cmd_buf.build_and_execute_now()?;

		self.size = new_size;

		Ok(true)
	}

	fn trim(&mut self) {
		self.glyphs_in_use.clear();
	}

	fn rebind_descriptor(&mut self) -> anyhow::Result<()> {
		self.image_descriptor = self.common.inner.uniform_sampler(
			Self::descriptor_set(self.kind),
			self.image_view.clone(),
			self.common.gfx.texture_filter,
		)?;
		Ok(())
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
	Mask,
	Color,
}

impl Kind {
	const fn num_channels(self) -> usize {
		match self {
			Self::Mask => 1,
			Self::Color => 4,
		}
	}

	const fn texture_format(self) -> Format {
		match self {
			Self::Mask => Format::R8_UNORM,
			Self::Color => Format::R8G8B8A8_UNORM,
		}
	}
}

/// An atlas containing a cache of rasterized glyphs that can be rendered.
pub struct TextAtlas {
	pub(super) common: TextPipeline,
	pub(super) color_atlas: InnerAtlas,
	pub(super) mask_atlas: InnerAtlas,
}

impl TextAtlas {
	/// Creates a new [`TextAtlas`].
	pub fn new(common: TextPipeline) -> anyhow::Result<Self> {
		let color_atlas = InnerAtlas::new(common.clone(), Kind::Color)?;
		let mask_atlas = InnerAtlas::new(common.clone(), Kind::Mask)?;

		Ok(Self {
			common,
			color_atlas,
			mask_atlas,
		})
	}

	pub fn trim(&mut self) {
		self.mask_atlas.trim();
		self.color_atlas.trim();
	}

	pub(super) fn grow(
		&mut self,
		font_system: &mut FontSystem,
		cache: &mut SwashCache,
		content_type: ContentType,
	) -> anyhow::Result<bool> {
		let did_grow = match content_type {
			ContentType::Mask => self.mask_atlas.grow(font_system, cache)?,
			ContentType::Color => self.color_atlas.grow(font_system, cache)?,
		};

		if did_grow {
			self.color_atlas.rebind_descriptor()?;
			self.mask_atlas.rebind_descriptor()?;
		}

		Ok(did_grow)
	}

	pub(super) const fn inner_for_content_mut(&mut self, content_type: ContentType) -> &mut InnerAtlas {
		match content_type {
			ContentType::Color => &mut self.color_atlas,
			ContentType::Mask => &mut self.mask_atlas,
		}
	}
}
