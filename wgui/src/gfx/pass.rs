use std::{marker::PhantomData, ops::Range, sync::Arc};

use anyhow::Context as _;

use super::{
	Buffer, Filter, ImageView, SamplerCreateInfo, Vertex, WGfx,
	pipeline::{DescriptorSet, WGfxPipeline},
	types::{SamplerAddressMode, Scissor},
};

/// Reusable draw description.
///
/// Unlike the old Vulkano implementation this does not expose or own a raw
/// secondary command buffer. `GfxCommandBuffer::run_ref` records the draw into
/// `WGfx`'s private primary command buffer.
pub struct WGfxPass<V> {
	pub(super) pipeline: Arc<WGfxPipeline<V>>,
	pub(super) graphics: Arc<WGfx>,
	pub(super) vertex_buffer: Arc<Buffer<V>>,
	pub(super) vertices: Range<u32>,
	pub(super) instances: Range<u32>,
	pub(super) descriptor_sets: Vec<Arc<DescriptorSet>>,
	pub(super) dimensions: [f32; 2],
	pub(super) offset: [f32; 2],
	pub(super) scissor: Scissor,
	_dummy: PhantomData<V>,
}

impl<V> WGfxPass<V>
where
	V: Vertex,
{
	#[allow(clippy::too_many_arguments)]
	pub(super) fn new(
		pipeline: Arc<WGfxPipeline<V>>,
		dimensions: [f32; 2],
		offset: [f32; 2],
		vertex_buffer: Arc<Buffer<V>>,
		vertices: Range<u32>,
		instances: Range<u32>,
		descriptor_sets: Vec<Arc<DescriptorSet>>,
		scissor: Scissor,
	) -> anyhow::Result<Self> {
		for (index, set) in descriptor_sets.iter().enumerate() {
			if !Arc::ptr_eq(&set.graphics, &pipeline.graphics) {
				anyhow::bail!("descriptor set {index} belongs to a different WGfx instance");
			}
		}

		Ok(Self {
			graphics: pipeline.graphics.clone(),
			pipeline,
			vertex_buffer,
			vertices,
			instances,
			descriptor_sets,
			dimensions,
			offset,
			scissor,
			_dummy: PhantomData,
		})
	}

	pub fn update_sampler(&self, set: usize, texture: Arc<ImageView>, filter: Filter) -> anyhow::Result<()> {
		let descriptor_set = self
			.descriptor_sets
			.get(set)
			.with_context(|| format!("pass has no descriptor set {set}"))?;
		let sampler = self.graphics.create_sampler(SamplerCreateInfo {
			mag_filter: filter,
			min_filter: filter,
			address_mode: [SamplerAddressMode::Repeat; 3],
		})?;
		descriptor_set.write_sampler(0, texture, sampler)
	}
}
