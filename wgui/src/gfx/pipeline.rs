use std::{any::Any, collections::BTreeMap, marker::PhantomData, ops::Range, sync::Arc};

use anyhow::bail;
use parking_lot::Mutex;

use super::{
	Buffer, Filter, Format, ImageView, PrimitiveTopology, Sampler, SamplerCreateInfo, ShaderModule, Vertex, WGfx,
	pass::WGfxPass,
	raw::{PipelineSpec, RawDescriptorSet, RawPipeline},
	types::{AttachmentBlend, DescriptorBinding, DescriptorType, SamplerAddressMode, Scissor},
};

pub struct DescriptorSet {
	pub(super) raw: Arc<RawDescriptorSet>,
	pub(super) graphics: Arc<WGfx>,
	keepalive: Mutex<BTreeMap<u32, Box<dyn Any + Send + Sync>>>,
}

impl DescriptorSet {
	fn new(graphics: Arc<WGfx>, raw: Arc<RawDescriptorSet>) -> Arc<Self> {
		Arc::new(Self {
			raw,
			graphics,
			keepalive: Mutex::new(BTreeMap::new()),
		})
	}

	fn write_buffer<T>(&self, binding: u32, buffer: Arc<Buffer<T>>) -> anyhow::Result<()>
	where
		T: Send + Sync + 'static,
	{
		self
			.graphics
			.raw
			.update_descriptor_buffer(&self.raw, binding, &buffer.raw)?;
		self.keepalive.lock().insert(binding, Box::new(buffer));
		Ok(())
	}

	pub(super) fn write_sampler(
		&self,
		binding: u32,
		texture: Arc<ImageView>,
		sampler: Arc<Sampler>,
	) -> anyhow::Result<()> {
		self
			.graphics
			.raw
			.update_descriptor_image_sampler(&self.raw, binding, &texture.raw, &sampler.raw)?;
		self.keepalive.lock().insert(binding, Box::new((texture, sampler)));
		Ok(())
	}
}

pub struct WGfxPipeline<V> {
	pub(super) graphics: Arc<WGfx>,
	pub(super) raw: Arc<RawPipeline>,
	format: Format,
	_dummy: PhantomData<V>,
}

impl<V> WGfxPipeline<V>
where
	V: Vertex,
{
	pub(super) fn new(
		graphics: Arc<WGfx>,
		vert: &Arc<ShaderModule>,
		frag: &Arc<ShaderModule>,
		info: WPipelineCreateInfo,
	) -> anyhow::Result<Self> {
		let descriptor_bindings = merge_descriptor_bindings(vert, frag)?;
		let spec = PipelineSpec {
			format: info.format,
			blend: info.blend,
			topology: info.topology,
			instanced: info.instanced,
			vertex_stride: std::mem::size_of::<V>() as u32,
			vertex_attributes: V::attributes(),
			descriptor_bindings,
			updatable_sets: info.updatable_sets,
		};
		let raw = graphics.raw.create_pipeline(&vert.raw, &frag.raw, &spec)?;
		Ok(Self {
			graphics,
			raw,
			format: info.format,
			_dummy: PhantomData,
		})
	}

	pub const fn format(&self) -> Format {
		self.format
	}

	pub fn uniform_sampler(
		self: &Arc<Self>,
		set: usize,
		texture: Arc<ImageView>,
		filter: Filter,
	) -> anyhow::Result<Arc<DescriptorSet>> {
		let layout = self.raw.set_layout(set)?;
		let descriptor_set = DescriptorSet::new(self.graphics.clone(), self.graphics.raw.create_descriptor_set(layout)?);
		let sampler = self.graphics.create_sampler(SamplerCreateInfo {
			mag_filter: filter,
			min_filter: filter,
			address_mode: [SamplerAddressMode::Repeat; 3],
		})?;
		descriptor_set.write_sampler(0, texture, sampler)?;
		Ok(descriptor_set)
	}

	pub fn buffer<T>(self: &Arc<Self>, set: usize, buffer: Arc<Buffer<T>>) -> anyhow::Result<Arc<DescriptorSet>>
	where
		T: Send + Sync + 'static,
	{
		let layout = self.raw.set_layout(set)?;
		let descriptor_set = DescriptorSet::new(self.graphics.clone(), self.graphics.raw.create_descriptor_set(layout)?);
		descriptor_set.write_buffer(0, buffer)?;
		Ok(descriptor_set)
	}

	pub fn uniform_buffer_upload<T>(self: &Arc<Self>, set: usize, contents: &[T]) -> anyhow::Result<Arc<DescriptorSet>>
	where
		T: Copy + Send + Sync + 'static,
	{
		let buffer = self.graphics.new_buffer(super::BufferUsage::UNIFORM_BUFFER, contents)?;
		self.buffer(set, buffer)
	}

	#[allow(clippy::too_many_arguments)]
	pub fn create_pass(
		self: &Arc<Self>,
		dimensions: [f32; 2],
		offset: [f32; 2],
		vertex_buffer: Arc<Buffer<V>>,
		vertices: Range<u32>,
		instances: Range<u32>,
		descriptor_sets: Vec<Arc<DescriptorSet>>,
		scissor: Scissor,
	) -> anyhow::Result<WGfxPass<V>> {
		WGfxPass::new(
			self.clone(),
			dimensions,
			offset,
			vertex_buffer,
			vertices,
			instances,
			descriptor_sets,
			scissor,
		)
	}
}

#[derive(Debug, Clone)]
pub struct WPipelineCreateInfo {
	pub(super) format: Format,
	pub(super) blend: Option<AttachmentBlend>,
	pub(super) topology: PrimitiveTopology,
	pub(super) instanced: bool,
	pub(super) updatable_sets: Vec<usize>,
}

impl WPipelineCreateInfo {
	pub const fn new(format: Format) -> Self {
		Self {
			format,
			blend: None,
			topology: PrimitiveTopology::TriangleStrip,
			instanced: false,
			updatable_sets: Vec::new(),
		}
	}

	#[must_use]
	pub const fn use_blend(mut self, blend: AttachmentBlend) -> Self {
		self.blend = Some(blend);
		self
	}

	#[must_use]
	pub const fn use_topology(mut self, topology: PrimitiveTopology) -> Self {
		self.topology = topology;
		self
	}

	#[must_use]
	pub const fn use_instanced(mut self) -> Self {
		self.instanced = true;
		self
	}

	#[must_use]
	pub fn use_updatable_descriptors<I>(mut self, updatable_sets: I) -> Self
	where
		I: IntoIterator<Item = usize>,
	{
		self.updatable_sets = updatable_sets.into_iter().collect();
		self
	}
}

fn merge_descriptor_bindings(vert: &ShaderModule, frag: &ShaderModule) -> anyhow::Result<Vec<DescriptorBinding>> {
	let mut merged = BTreeMap::<(u32, u32), DescriptorBinding>::new();

	for binding in vert
		.descriptor_bindings()
		.iter()
		.chain(frag.descriptor_bindings())
		.copied()
	{
		match merged.get_mut(&(binding.set, binding.binding)) {
			Some(existing) => {
				if existing.descriptor_type != binding.descriptor_type || existing.descriptor_count != binding.descriptor_count
				{
					bail!(
						"descriptor set {} binding {} has incompatible declarations between shaders",
						binding.set,
						binding.binding
					);
				}
				existing.stages |= binding.stages;
			}
			None => {
				merged.insert((binding.set, binding.binding), binding);
			}
		}
	}

	let result = merged.into_values().collect::<Vec<_>>();
	for binding in &result {
		if binding.descriptor_count == 0 {
			bail!(
				"descriptor set {} binding {} has descriptor_count=0",
				binding.set,
				binding.binding
			);
		}
		match binding.descriptor_type {
			DescriptorType::CombinedImageSampler | DescriptorType::UniformBuffer | DescriptorType::StorageBuffer => {}
		}
	}
	Ok(result)
}
