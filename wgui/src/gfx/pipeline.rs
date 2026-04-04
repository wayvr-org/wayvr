use std::{marker::PhantomData, ops::Range, sync::Arc};

use smallvec::{SmallVec, smallvec};
use vulkano::{
	buffer::{
		BufferContents, BufferUsage, Subbuffer,
		allocator::{SubbufferAllocator, SubbufferAllocatorCreateInfo},
	},
	descriptor_set::{
		DescriptorSet, WriteDescriptorSet,
		layout::{DescriptorBindingFlags, DescriptorSetLayoutCreateFlags},
	},
	format::Format,
	image::{
		sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo},
		view::ImageView,
	},
	memory::allocator::MemoryTypeFilter,
	pipeline::{
		DynamicState, GraphicsPipeline, Pipeline, PipelineLayout,
		graphics::{
			self, GraphicsPipelineCreateInfo,
			color_blend::{AttachmentBlend, ColorBlendAttachmentState, ColorBlendState},
			input_assembly::{InputAssemblyState, PrimitiveTopology},
			multisample::MultisampleState,
			rasterization::RasterizationState,
			subpass::PipelineRenderingCreateInfo,
			vertex_input::{Vertex, VertexDefinition, VertexInputState},
			viewport::ViewportState,
		},
		layout::PipelineDescriptorSetLayoutCreateInfo,
	},
	shader::{EntryPoint, ShaderModule},
};

use super::{WGfx, pass::WGfxPass};

pub struct WGfxPipeline<V> {
	pub graphics: Arc<WGfx>,
	pub pipeline: Arc<GraphicsPipeline>,
	pub format: Format,
	_dummy: PhantomData<V>,
}

impl<V> WGfxPipeline<V>
where
	V: Sized,
{
	#[allow(clippy::too_many_arguments)]
	fn new_from_stages(
		graphics: Arc<WGfx>,
		format: Format,
		blend: Option<AttachmentBlend>,
		topology: PrimitiveTopology,
		vert_entry_point: EntryPoint,
		frag_entry_point: EntryPoint,
		vertex_input_state: Option<VertexInputState>,
		updatable_sets: &[usize],
	) -> anyhow::Result<Self> {
		let stages = smallvec![
			vulkano::pipeline::PipelineShaderStageCreateInfo::new(vert_entry_point),
			vulkano::pipeline::PipelineShaderStageCreateInfo::new(frag_entry_point),
		];

		let mut layout_info = PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages);
		for (idx_l, l) in layout_info.set_layouts.iter_mut().enumerate() {
			if updatable_sets.contains(&idx_l) {
				// mark all bindings in the set as UAB
				l.flags |= DescriptorSetLayoutCreateFlags::UPDATE_AFTER_BIND_POOL;
				for b in l.bindings.values_mut() {
					b.binding_flags |= DescriptorBindingFlags::UPDATE_AFTER_BIND;
				}
			}
		}

		let layout = PipelineLayout::new(
			graphics.device.clone(),
			layout_info.into_pipeline_layout_create_info(graphics.device.clone())?,
		)?;

		let subpass = PipelineRenderingCreateInfo {
			color_attachment_formats: vec![Some(format)],
			..Default::default()
		};

		let pipeline = GraphicsPipeline::new(
			graphics.device.clone(),
			None,
			GraphicsPipelineCreateInfo {
				stages,
				vertex_input_state,
				input_assembly_state: Some(InputAssemblyState {
					topology,
					..InputAssemblyState::default()
				}),
				viewport_state: Some(ViewportState::default()),
				rasterization_state: Some(RasterizationState {
					cull_mode: vulkano::pipeline::graphics::rasterization::CullMode::None,
					..RasterizationState::default()
				}),
				multisample_state: Some(MultisampleState::default()),
				color_blend_state: Some(ColorBlendState {
					attachments: vec![ColorBlendAttachmentState {
						blend,
						..Default::default()
					}],
					..Default::default()
				}),
				dynamic_state: [DynamicState::Viewport, DynamicState::Scissor].into_iter().collect(),
				subpass: Some(subpass.into()),
				..GraphicsPipelineCreateInfo::layout(layout)
			},
		)?;

		Ok(Self {
			graphics,
			pipeline,
			format,
			_dummy: PhantomData,
		})
	}

	pub fn inner(&self) -> Arc<GraphicsPipeline> {
		self.pipeline.clone()
	}

	pub fn uniform_sampler(
		&self,
		set: usize,
		texture: Arc<ImageView>,
		filter: Filter,
	) -> anyhow::Result<Arc<DescriptorSet>> {
		let sampler = Sampler::new(
			self.graphics.device.clone(),
			SamplerCreateInfo {
				mag_filter: filter,
				min_filter: filter,
				address_mode: [SamplerAddressMode::Repeat; 3],
				..Default::default()
			},
		)?;

		let layout = self.pipeline.layout().set_layouts().get(set).unwrap(); // want panic

		Ok(DescriptorSet::new(
			self.graphics.descriptor_set_allocator.clone(),
			layout.clone(),
			[WriteDescriptorSet::image_view_sampler(0, texture, sampler)],
			[],
		)?)
	}

	// uniform or storage buffer
	pub fn buffer<T>(&self, set: usize, buffer: Subbuffer<[T]>) -> anyhow::Result<Arc<DescriptorSet>>
	where
		T: BufferContents + Copy,
	{
		let layout = self.pipeline.layout().set_layouts().get(set).unwrap(); // want panic
		Ok(DescriptorSet::new(
			self.graphics.descriptor_set_allocator.clone(),
			layout.clone(),
			[WriteDescriptorSet::buffer(0, buffer)],
			[],
		)?)
	}

	#[allow(clippy::needless_pass_by_value)]
	pub fn uniform_buffer_upload<T>(&self, set: usize, data: Vec<T>) -> anyhow::Result<Arc<DescriptorSet>>
	where
		T: BufferContents + Copy,
	{
		let buf = SubbufferAllocator::new(
			self.graphics.memory_allocator.clone(),
			SubbufferAllocatorCreateInfo {
				buffer_usage: BufferUsage::UNIFORM_BUFFER,
				memory_type_filter: MemoryTypeFilter::PREFER_DEVICE | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
				..Default::default()
			},
		);

		let uniform_buffer_subbuffer = {
			let subbuffer = buf.allocate_slice(data.len() as _)?;
			subbuffer.write()?.copy_from_slice(data.as_slice());
			subbuffer
		};

		self.buffer(set, uniform_buffer_subbuffer)
	}
}

pub struct WPipelineCreateInfo {
	format: Format,
	blend: Option<AttachmentBlend>,
	topology: PrimitiveTopology,
	instanced: bool,
	updatable_sets: SmallVec<[usize; 8]>,
}

impl WPipelineCreateInfo {
	pub fn new(format: Format) -> Self {
		Self {
			format,
			blend: None,
			topology: PrimitiveTopology::TriangleStrip,
			instanced: false,
			updatable_sets: smallvec![],
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
	pub fn use_updatable_descriptors(mut self, updatable_sets: SmallVec<[usize; 8]>) -> Self {
		self.updatable_sets = updatable_sets;
		self
	}
}

impl<V> WGfxPipeline<V>
where
	V: BufferContents + Vertex,
{
	pub(super) fn new_with_vert_input(
		graphics: Arc<WGfx>,
		vert: &Arc<ShaderModule>,
		frag: &Arc<ShaderModule>,
		info: WPipelineCreateInfo,
	) -> anyhow::Result<Self> {
		let vert_entry_point = vert.entry_point("main").unwrap(); // want panic
		let frag_entry_point = frag.entry_point("main").unwrap(); // want panic

		let vertex_input_state = Some(if info.instanced {
			V::per_instance().definition(&vert_entry_point)?
		} else {
			V::per_vertex().definition(&vert_entry_point)?
		});

		Self::new_from_stages(
			graphics,
			info.format,
			info.blend,
			info.topology,
			vert_entry_point,
			frag_entry_point,
			vertex_input_state,
			&info.updatable_sets,
		)
	}

	#[allow(clippy::too_many_arguments)]
	pub fn create_pass(
		self: &Arc<Self>,
		dimensions: [f32; 2],
		offset: [f32; 2],
		vertex_buffer: Subbuffer<[V]>,
		vertices: Range<u32>,
		instances: Range<u32>,
		descriptor_sets: Vec<Arc<DescriptorSet>>,
		vk_scissor: &graphics::viewport::Scissor,
	) -> anyhow::Result<WGfxPass<V>> {
		WGfxPass::new(
			&self.clone(),
			dimensions,
			offset,
			vertex_buffer,
			vertices,
			instances,
			descriptor_sets,
			vk_scissor,
		)
	}
}
