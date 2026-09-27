use std::{any::Any, marker::PhantomData, sync::Arc};

use parking_lot::Mutex;

use super::{
	BufferUsage, Format, Image, ImageCreateInfo, ImageLayout, ImageUsage, ImageView, QueueType, Vertex, WGfx,
	pass::WGfxPass,
	raw::{RawClearMode, RawCommandBuffer},
};

pub type GfxCommandBufferBuilder = CommandBufferBuilder<CmdBufGfx>;
pub type XferCommandBufferBuilder = CommandBufferBuilder<CmdBufXfer>;

pub struct CmdBufGfx;
pub struct CmdBufXfer;

// A built primary command buffer
pub struct BuiltCommandBuffer {
	pub(super) raw: Mutex<RawCommandBuffer>,
	_keepalive: Vec<Box<dyn Any + Send + Sync>>,
}

impl BuiltCommandBuffer {
	pub fn submit_and_wait(&self) -> anyhow::Result<()> {
		self.raw.lock().submit_and_wait()
	}
}

pub struct CommandBufferBuilder<T> {
	pub(super) graphics: Arc<WGfx>,
	raw: RawCommandBuffer,
	keepalive: Vec<Box<dyn Any + Send + Sync>>,
	_dummy: PhantomData<T>,
}

impl<T> CommandBufferBuilder<T> {
	pub(super) fn new(
		graphics: Arc<WGfx>,
		queue_type: QueueType,
		usage: super::CommandBufferUsage,
		dummy: PhantomData<T>,
	) -> anyhow::Result<Self> {
		let raw = graphics.raw.create_command_buffer(queue_type, usage)?;
		Ok(Self {
			graphics,
			raw,
			keepalive: Vec::new(),
			_dummy: dummy,
		})
	}

	pub fn build(mut self) -> anyhow::Result<Arc<BuiltCommandBuffer>> {
		self.raw.finish()?;
		Ok(Arc::new(BuiltCommandBuffer {
			raw: Mutex::new(self.raw),
			_keepalive: std::mem::take(&mut self.keepalive),
		}))
	}

	pub fn build_and_execute(mut self) -> anyhow::Result<()> {
		self.raw.submit_and_wait()
	}

	pub fn build_and_execute_now(self) -> anyhow::Result<()> {
		self.build_and_execute()
	}

	pub fn transition_image(&mut self, image: &Arc<Image>, new_layout: ImageLayout) -> anyhow::Result<()> {
		self.raw.transition_image(&image.raw, new_layout);
		self.keepalive.push(Box::new(image.clone()));
		Ok(())
	}
}

#[derive(Default, Debug, Clone, Copy)]
pub enum WGfxClearMode {
	#[default]
	DontCare,
	Keep,
	Clear([f32; 4]),
}

impl WGfxClearMode {
	#[must_use]
	pub const fn or_default(self, default: Self) -> Self {
		match self {
			Self::DontCare => default,
			other => other,
		}
	}

	const fn raw(self) -> RawClearMode {
		match self {
			Self::DontCare => RawClearMode::DontCare,
			Self::Keep => RawClearMode::Keep,
			Self::Clear(color) => RawClearMode::Clear(color),
		}
	}
}

impl CommandBufferBuilder<CmdBufGfx> {
	pub fn begin_rendering(&mut self, render_target: Arc<ImageView>, clear_mode: WGfxClearMode) -> anyhow::Result<()> {
		self
			.raw
			.begin_rendering(&render_target.raw, render_target.extent(), clear_mode.raw());
		self.keepalive.push(Box::new(render_target));
		Ok(())
	}

	pub fn run_ref<V: Vertex>(&mut self, pass: &WGfxPass<V>) -> anyhow::Result<()> {
		self.raw.draw(
			&pass.pipeline.raw,
			&pass.vertex_buffer.raw,
			&pass
				.descriptor_sets
				.iter()
				.map(|set| set.raw.clone())
				.collect::<Vec<_>>(),
			pass.dimensions,
			pass.offset,
			pass.scissor,
			pass.vertices.clone(),
			pass.instances.clone(),
		)?;

		self.keepalive.push(Box::new(pass.pipeline.clone()));
		self.keepalive.push(Box::new(pass.vertex_buffer.clone()));
		for descriptor_set in &pass.descriptor_sets {
			self.keepalive.push(Box::new(descriptor_set.clone()));
		}
		Ok(())
	}

	pub fn end_rendering(&mut self) -> anyhow::Result<()> {
		self.raw.end_rendering();
		Ok(())
	}
}

impl CommandBufferBuilder<CmdBufXfer> {
	pub fn upload_image(&mut self, width: u32, height: u32, format: Format, data: &[u8]) -> anyhow::Result<Arc<Image>> {
		let image = self.graphics.create_image(ImageCreateInfo::new_2d(
			width,
			height,
			format,
			ImageUsage::TRANSFER_DST | ImageUsage::TRANSFER_SRC | ImageUsage::SAMPLED,
		))?;
		let staging = self
			.graphics
			.empty_buffer::<u8>(BufferUsage::TRANSFER_SRC, data.len() as u64)?;
		staging.write()?.copy_from_slice(data);
		self
			.raw
			.copy_buffer_to_image(&staging.raw, &image.raw, [0, 0, 0], [width, height, 1]);
		self.keepalive.push(Box::new(staging));
		self.keepalive.push(Box::new(image.clone()));
		Ok(image)
	}

	pub fn clear_image(&mut self, image: &Arc<Image>) -> anyhow::Result<()> {
		self.raw.clear_image(&image.raw);
		self.keepalive.push(Box::new(image.clone()));
		Ok(())
	}

	pub fn update_image(
		&mut self,
		image: &Arc<Image>,
		data: &[u8],
		offset: [u32; 3],
		extent: Option<[u32; 3]>,
	) -> anyhow::Result<()> {
		#[allow(clippy::or_fun_call)] //const fn returning array
		let extent = extent.unwrap_or(image.extent());
		let staging = self
			.graphics
			.empty_buffer::<u8>(BufferUsage::TRANSFER_SRC, data.len() as u64)?;
		staging.write()?.copy_from_slice(data);
		self.raw.copy_buffer_to_image(&staging.raw, &image.raw, offset, extent);
		self.keepalive.push(Box::new(staging));
		self.keepalive.push(Box::new(image.clone()));
		Ok(())
	}

	pub fn copy_image(
		&mut self,
		src: &Arc<Image>,
		src_offset: [u32; 3],
		dst: &Arc<Image>,
		dst_offset: [u32; 3],
		extent: Option<[u32; 3]>,
	) -> anyhow::Result<()> {
		self.raw.copy_image(
			&src.raw,
			src_offset,
			&dst.raw,
			dst_offset,
			#[allow(clippy::or_fun_call)] //const fn returning array
			extent.unwrap_or(src.extent()),
		);
		self.keepalive.push(Box::new(src.clone()));
		self.keepalive.push(Box::new(dst.clone()));
		Ok(())
	}
}
