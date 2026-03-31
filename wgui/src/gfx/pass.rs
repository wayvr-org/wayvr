use std::{marker::PhantomData, ops::Range, sync::Arc};

use smallvec::smallvec;
use vulkano::{
	buffer::{BufferContents, Subbuffer},
	command_buffer::{
		AutoCommandBufferBuilder, CommandBufferInheritanceInfo, CommandBufferInheritanceRenderPassType,
		CommandBufferInheritanceRenderingInfo, CommandBufferUsage, SecondaryAutoCommandBuffer,
	},
	descriptor_set::{DescriptorSet, WriteDescriptorSet},
	image::{
		sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo},
		view::ImageView,
	},
	pipeline::{
		Pipeline, PipelineBindPoint,
		graphics::{self, vertex_input::Vertex, viewport::Viewport},
	},
};

use super::{WGfx, pipeline::WGfxPipeline};

pub struct WGfxPass<V> {
	pub command_buffer: Arc<SecondaryAutoCommandBuffer>,
	graphics: Arc<WGfx>,
	descriptor_sets: Vec<Arc<DescriptorSet>>,
	_dummy: PhantomData<V>,
}

impl<V> WGfxPass<V>
where
	V: BufferContents + Vertex,
{
	#[allow(clippy::too_many_arguments)]
	pub(super) fn new(
		pipeline: &Arc<WGfxPipeline<V>>,
		dimensions: [f32; 2],
		offset: [f32; 2],
		vertex_buffer: Subbuffer<[V]>,
		vertices: Range<u32>,
		instances: Range<u32>,
		descriptor_sets: Vec<Arc<DescriptorSet>>,
		vk_scissor: &graphics::viewport::Scissor,
	) -> anyhow::Result<Self> {
		let viewport = Viewport {
			offset,
			extent: dimensions,
			depth_range: 0.0..=1.0,
		};
		let pipeline_inner = pipeline.inner();
		let mut command_buffer = AutoCommandBufferBuilder::secondary(
			pipeline.graphics.command_buffer_allocator.clone(),
			pipeline.graphics.queue_gfx.queue_family_index(),
			CommandBufferUsage::SimultaneousUse,
			CommandBufferInheritanceInfo {
				render_pass: Some(CommandBufferInheritanceRenderPassType::BeginRendering(
					CommandBufferInheritanceRenderingInfo {
						color_attachment_formats: vec![Some(pipeline.format)],

						..Default::default()
					},
				)),
				..Default::default()
			},
		)?;

		unsafe {
			command_buffer
				.set_viewport(0, smallvec![viewport])?
				.set_scissor(0, smallvec![*vk_scissor])?
				.bind_pipeline_graphics(pipeline_inner)?
				.bind_descriptor_sets(
					PipelineBindPoint::Graphics,
					pipeline.inner().layout().clone(),
					0,
					descriptor_sets.clone(),
				)?
				.bind_vertex_buffers(0, vertex_buffer)?
				.draw(
					vertices.end - vertices.start,
					instances.end - instances.start,
					vertices.start,
					instances.start,
				)?
		};

		Ok(Self {
			command_buffer: command_buffer.build()?,
			graphics: pipeline.graphics.clone(),
			descriptor_sets,
			_dummy: PhantomData,
		})
	}

	pub fn update_sampler(&self, set: usize, texture: Arc<ImageView>, filter: Filter) -> anyhow::Result<()> {
		let sampler = Sampler::new(
			self.graphics.device.clone(),
			SamplerCreateInfo {
				mag_filter: filter,
				min_filter: filter,
				address_mode: [SamplerAddressMode::Repeat; 3],
				..Default::default()
			},
		)?;

		unsafe {
			self.descriptor_sets[set].update_by_ref([WriteDescriptorSet::image_view_sampler(0, texture, sampler)], [])?;
		}

		Ok(())
	}
}
