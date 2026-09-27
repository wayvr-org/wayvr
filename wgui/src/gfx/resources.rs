use std::{
	marker::PhantomData,
	mem::{align_of, size_of},
	ops::{Deref, DerefMut},
	slice,
	sync::Arc,
};

use anyhow::{Context as _, bail};
use parking_lot::MutexGuard;

use super::{
	raw::{RawBuffer, RawImage, RawImageView, RawSampler, RawShaderModule},
	types::{DescriptorBinding, Format, ImageTiling, ImageUsage, SamplerCreateInfo, ShaderStage},
};

pub struct Buffer<T> {
	pub(super) raw: Arc<RawBuffer>,
	len: usize,
	_marker: PhantomData<T>,
}

unsafe impl<T: Send> Send for Buffer<T> {}
unsafe impl<T: Sync> Sync for Buffer<T> {}

impl<T> Buffer<T> {
	pub(super) const fn from_raw(raw: Arc<RawBuffer>, len: usize) -> Self {
		Self {
			raw,
			len,
			_marker: PhantomData,
		}
	}

	pub const fn len(&self) -> usize {
		self.len
	}

	pub const fn is_empty(&self) -> bool {
		self.len == 0
	}

	pub const fn byte_len(&self) -> usize {
		size_of::<T>().saturating_mul(self.len)
	}

	pub fn write(&self) -> anyhow::Result<BufferWriteGuard<'_, T>> {
		let lock = self.raw.write_lock.lock();
		let ptr = self.raw.mapped_ptr();
		if ptr.as_ptr().align_offset(align_of::<T>()) != 0 {
			bail!("mapped Vulkan buffer is not aligned for requested element type");
		}
		Ok(BufferWriteGuard {
			buffer: self,
			_lock: lock,
		})
	}
}

pub struct BufferWriteGuard<'a, T> {
	buffer: &'a Buffer<T>,
	_lock: MutexGuard<'a, ()>,
}

impl<T> Deref for BufferWriteGuard<'_, T> {
	type Target = [T];

	fn deref(&self) -> &Self::Target {
		unsafe { slice::from_raw_parts(self.buffer.raw.mapped_ptr().as_ptr().cast::<T>(), self.buffer.len) }
	}
}

impl<T> DerefMut for BufferWriteGuard<'_, T> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		unsafe { slice::from_raw_parts_mut(self.buffer.raw.mapped_ptr().as_ptr().cast::<T>(), self.buffer.len) }
	}
}

impl<T> Drop for BufferWriteGuard<'_, T> {
	fn drop(&mut self) {
		if let Err(e) = self.buffer.raw.flush() {
			log::error!("failed flushing mapped WGfx buffer: {e:#}");
		}
	}
}

pub struct Image {
	pub(super) raw: Arc<RawImage>,
	extent: [u32; 3],
	format: Format,
	usage: ImageUsage,
	tiling: ImageTiling,
	array_layers: u32,
}

impl Image {
	pub(super) const fn from_raw(
		raw: Arc<RawImage>,
		extent: [u32; 3],
		format: Format,
		usage: ImageUsage,
		tiling: ImageTiling,
		array_layers: u32,
	) -> Self {
		Self {
			raw,
			extent,
			format,
			usage,
			tiling,
			array_layers,
		}
	}

	pub const fn extent(&self) -> [u32; 3] {
		self.extent
	}

	pub const fn width(&self) -> u32 {
		self.extent[0]
	}

	pub const fn height(&self) -> u32 {
		self.extent[1]
	}

	pub const fn format(&self) -> Format {
		self.format
	}

	pub const fn usage(&self) -> ImageUsage {
		self.usage
	}

	pub const fn tiling(&self) -> ImageTiling {
		self.tiling
	}

	pub const fn array_layers(&self) -> u32 {
		self.array_layers
	}

	/// returns the image handle while keeping ownership
	pub fn raw_handle(&self) -> u64 {
		self.raw.raw_handle()
	}
}

pub struct ImageView {
	pub(super) raw: Arc<RawImageView>,
	image: Arc<Image>,
}

impl ImageView {
	pub(super) const fn from_raw(raw: Arc<RawImageView>, image: Arc<Image>) -> Self {
		Self { raw, image }
	}

	pub const fn image(&self) -> &Arc<Image> {
		&self.image
	}

	pub fn format(&self) -> Format {
		self.image.format()
	}

	pub fn extent(&self) -> [u32; 3] {
		self.image.extent()
	}

	pub fn extent_2d(&self) -> [u32; 2] {
		let [width, height, _] = self.image.extent();
		[width, height]
	}

	pub fn extent_f32(&self) -> [f32; 2] {
		let [width, height, _] = self.image.extent();
		[width as f32, height as f32]
	}
}

pub struct Sampler {
	pub(super) raw: Arc<RawSampler>,
	info: SamplerCreateInfo,
}

impl Sampler {
	pub(super) const fn from_raw(raw: Arc<RawSampler>, info: SamplerCreateInfo) -> Self {
		Self { raw, info }
	}

	pub const fn info(&self) -> SamplerCreateInfo {
		self.info
	}
}

pub struct ShaderModule {
	pub(super) raw: Arc<RawShaderModule>,
	stage: ShaderStage,
	bindings: Vec<DescriptorBinding>,
}

impl ShaderModule {
	pub(super) const fn from_raw(
		raw: Arc<RawShaderModule>,
		stage: ShaderStage,
		bindings: Vec<DescriptorBinding>,
	) -> Self {
		Self { raw, stage, bindings }
	}

	pub const fn stage(&self) -> ShaderStage {
		self.stage
	}

	pub fn descriptor_bindings(&self) -> &[DescriptorBinding] {
		&self.bindings
	}
}

pub fn spirv_words(bytes: &[u8]) -> anyhow::Result<Vec<u32>> {
	if !bytes.len().is_multiple_of(4) {
		bail!("SPIR-V bytecode length is not a multiple of four");
	}

	let mut words = Vec::with_capacity(bytes.len() / 4);
	for chunk in bytes.chunks_exact(4) {
		let word = u32::from_le_bytes(chunk.try_into().context("invalid SPIR-V word")?);
		words.push(word);
	}

	if words.first().copied() != Some(0x0723_0203) {
		bail!("invalid SPIR-V magic number");
	}

	Ok(words)
}
