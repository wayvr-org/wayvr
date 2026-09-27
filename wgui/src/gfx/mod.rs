//! `WGfx`: an abstraction for vulkan graphics over ash

pub mod cmd;
mod dmabuf;
pub mod pass;
pub mod pipeline;
pub mod resources;
pub mod swapchain;
pub mod types;

mod raw;

use std::{
	collections::HashMap,
	ffi::c_void,
	marker::PhantomData,
	mem::size_of,
	sync::{Arc, Weak},
};

use anyhow::Context as _;
use parking_lot::Mutex;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

pub use cmd::{BuiltCommandBuffer, GfxCommandBufferBuilder, WGfxClearMode, XferCommandBufferBuilder};
pub use dmabuf::{DmabufImportInfo, DmabufPlaneLayout, ExportedDmabufImage};
pub use pipeline::{DescriptorSet, WGfxPipeline, WPipelineCreateInfo};
pub use resources::{Buffer, BufferWriteGuard, Image, ImageView, Sampler, ShaderModule, spirv_words};
pub use swapchain::{PresentStatus, Swapchain, SwapchainError, SwapchainFrame};
pub use types::{
	AttachmentBlend, BLEND_ALPHA, BlendFactor, BlendOp, BufferUsage, CommandBufferUsage, DescriptorBinding,
	DescriptorType, DeviceCaps, DeviceInfo, DevicePreference, DeviceType, Filter, Format, ImageCreateInfo, ImageLayout,
	ImageTiling, ImageUsage, ImageViewCreateInfo, PrimitiveTopology, QueueType, SamplerAddressMode, SamplerCreateInfo,
	Scissor, ShaderStage, ShaderStages, Vertex, VertexAttribute, VertexFormat, WGfxCreateInfo,
};

pub type Vert2Buf = Arc<Buffer<Vert2Uv>>;
pub type IndexBuf = Arc<Buffer<u32>>;

pub fn upload_quad_vertices(
	buf: &Buffer<Vert2Uv>,
	width: f32,
	height: f32,
	x: f32,
	y: f32,
	w: f32,
	h: f32,
) -> anyhow::Result<()> {
	let x0 = x / width;
	let y0 = y / height;
	let x1 = w / width + x0;
	let y1 = h / height + y0;

	let data = [
		Vert2Uv {
			in_pos: [x0, y0],
			in_uv: [0.0, 0.0],
		},
		Vert2Uv {
			in_pos: [x0, y1],
			in_uv: [0.0, 1.0],
		},
		Vert2Uv {
			in_pos: [x1, y0],
			in_uv: [1.0, 0.0],
		},
		Vert2Uv {
			in_pos: [x1, y1],
			in_uv: [1.0, 1.0],
		},
	];

	buf.write()?[..4].copy_from_slice(&data);
	Ok(())
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct Vert2Uv {
	pub in_pos: [f32; 2],
	pub in_uv: [f32; 2],
}

impl Vertex for Vert2Uv {
	fn attributes() -> Vec<VertexAttribute> {
		vec![
			VertexAttribute::new(0, VertexFormat::R32g32Sfloat, std::mem::offset_of!(Self, in_pos) as u32),
			VertexAttribute::new(1, VertexFormat::R32g32Sfloat, std::mem::offset_of!(Self, in_uv) as u32),
		]
	}
}

/// owns vulkan instance, device, queues and is used for manading GPU resources
pub struct WGfx {
	raw: Arc<raw::Context>,
	surface_format: Format,
	texture_filter: Filter,
	samplers: Mutex<HashMap<SamplerCreateInfo, Weak<Sampler>>>,
}

impl WGfx {
	pub fn new(create_info: &WGfxCreateInfo) -> anyhow::Result<Arc<Self>> {
		let raw = raw::Context::new(create_info)?;
		Ok(Self::from_raw(raw, create_info.surface_format))
	}

	pub fn new_with_device_extensions<F>(
		create_info: &WGfxCreateInfo,
		mut required_device_extensions: F,
	) -> anyhow::Result<Arc<Self>>
	where
		F: FnMut(u64) -> Vec<String>,
	{
		let raw = raw::Context::new_with_device_extensions(create_info, &mut required_device_extensions)?;
		Ok(Self::from_raw(raw, create_info.surface_format))
	}

	/// wrap around an external VK instance and device.
	/// returned instance and device will be owned by `WGfx`.
	///
	/// # Safety
	/// callbacks must create objects from supplied create infos
	/// and return valid handles with ownership transferred to `WGfx`
	pub unsafe fn new_external_vulkan<FI, FP, FD>(
		create_info: &WGfxCreateInfo,
		create_instance: FI,
		get_physical_device: FP,
		create_device: FD,
	) -> anyhow::Result<Arc<Self>>
	where
		FI: FnOnce(usize, *const c_void) -> anyhow::Result<u64>,
		FP: FnOnce(u64) -> anyhow::Result<u64>,
		FD: FnOnce(usize, u64, *const c_void) -> anyhow::Result<u64>,
	{
		let raw =
			unsafe { raw::Context::new_external_vulkan(create_info, create_instance, get_physical_device, create_device)? };
		Ok(Self::from_raw(raw, create_info.surface_format))
	}

	/// create using a native window surface
	pub fn new_for_window<W>(window: &W, create_info: &WGfxCreateInfo) -> anyhow::Result<Arc<Self>>
	where
		W: HasDisplayHandle + HasWindowHandle,
	{
		let display_handle = window
			.display_handle()
			.map_err(|err| anyhow::anyhow!("failed to get display handle: {err:?}"))?
			.as_raw();
		let window_handle = window
			.window_handle()
			.map_err(|err| anyhow::anyhow!("failed to get window handle: {err:?}"))?
			.as_raw();
		let raw = raw::Context::new_windowed(create_info, display_handle, window_handle)?;
		let surface_format = raw.surface_format().unwrap_or(create_info.surface_format);
		Ok(Self::from_raw(raw, surface_format))
	}

	fn from_raw(raw: Arc<raw::Context>, surface_format: Format) -> Arc<Self> {
		let texture_filter = if raw.caps().filter_cubic {
			Filter::Cubic
		} else {
			Filter::Linear
		};

		Arc::new(Self {
			raw,
			surface_format,
			texture_filter,
			samplers: Mutex::new(HashMap::new()),
		})
	}

	pub fn device_info(&self) -> &DeviceInfo {
		self.raw.info()
	}

	pub fn capabilities(&self) -> DeviceCaps {
		self.raw.caps()
	}

	pub const fn surface_format(&self) -> Format {
		self.surface_format
	}

	pub const fn texture_filter(&self) -> Filter {
		self.texture_filter
	}

	/// Wait until all submissions on this vk device are finished
	pub fn wait_idle(&self) -> anyhow::Result<()> {
		self.raw.wait_idle()
	}

	pub fn raw_instance_handle(&self) -> u64 {
		self.raw.raw_instance_handle()
	}

	pub fn raw_physical_device_handle(&self) -> u64 {
		self.raw.raw_physical_device_handle()
	}

	pub fn raw_device_handle(&self) -> u64 {
		self.raw.raw_device_handle()
	}

	pub fn raw_graphics_queue_handle(&self) -> u64 {
		self.raw.raw_graphics_queue_handle()
	}

	pub fn graphics_queue_family_index(&self) -> u32 {
		self.raw.graphics_queue_family_index()
	}

	pub fn graphics_queue_index(&self) -> u32 {
		self.raw.graphics_queue_index()
	}

	pub const fn raw_format(&self, format: Format) -> i32 {
		raw::Context::raw_format(format)
	}

	pub fn create_swapchain(self: &Arc<Self>, extent: [u32; 2]) -> anyhow::Result<Arc<Swapchain>> {
		self.create_swapchain_inner(extent, None)
	}

	pub fn transition_image_now(&self, image: &Arc<Image>, new_layout: ImageLayout) -> anyhow::Result<()> {
		let mut cmd = self
			.raw
			.create_command_buffer(QueueType::Graphics, CommandBufferUsage::OneTimeSubmit)?;
		cmd.transition_image(&image.raw, new_layout);
		cmd.submit_and_wait()
	}

	fn create_swapchain_inner(
		self: &Arc<Self>,
		extent: [u32; 2],
		old_swapchain: Option<&Arc<raw::RawSwapchain>>,
	) -> anyhow::Result<Arc<Swapchain>> {
		let (raw, images) = self.raw.create_swapchain(extent, old_swapchain.map(Arc::as_ref))?;
		Swapchain::from_raw(self.clone(), raw, images)
	}

	pub fn empty_buffer<T>(&self, usage: BufferUsage, capacity: u64) -> anyhow::Result<Arc<Buffer<T>>>
	where
		T: Send + Sync + 'static,
	{
		let len = usize::try_from(capacity).context("buffer element count does not fit usize")?;
		let byte_len = size_of::<T>().checked_mul(len).context("buffer byte size overflow")?;
		let raw = self.raw.create_buffer(byte_len, usage)?;
		Ok(Arc::new(Buffer::from_raw(raw, len)))
	}

	pub fn new_buffer<T>(&self, usage: BufferUsage, contents: &[T]) -> anyhow::Result<Arc<Buffer<T>>>
	where
		T: Copy + Send + Sync + 'static,
	{
		let buffer = self.empty_buffer::<T>(usage, contents.len() as u64)?;
		buffer.write()?.copy_from_slice(contents);
		Ok(buffer)
	}

	pub fn create_image(&self, info: ImageCreateInfo) -> anyhow::Result<Arc<Image>> {
		if info.extent.contains(&0) {
			anyhow::bail!("image extent must be non-zero");
		}
		if info.array_layers == 0 {
			anyhow::bail!("image array_layers must be non-zero");
		}
		let raw = self.raw.create_image(info)?;
		Ok(Arc::new(Image::from_raw(
			raw,
			info.extent,
			info.format,
			info.usage,
			info.tiling,
			info.array_layers,
		)))
	}

	/// wraps an external vkImage without taking ownership
	///
	/// # Safety
	/// caller ensures that `raw_handle` is valid & compatible with `info`.
	/// caller ensures that the extenal image outlives the returned Image.
	pub unsafe fn wrap_external_image(&self, raw_handle: u64, info: ImageCreateInfo) -> anyhow::Result<Arc<Image>> {
		self.wrap_external_image_inner(raw_handle, info, None)
	}

	/// Wraps an external vkImage without taking ownership and initializes our
	/// layout tracker to a layout guaranteed by external API
	///
	/// # Safety
	/// In addition to the requirements of `Self::wrap_external_image`,
	/// caller must guarantee that image is actually in `initial_layout`
	pub unsafe fn wrap_external_image_with_layout(
		&self,
		raw_handle: u64,
		info: ImageCreateInfo,
		initial_layout: ImageLayout,
	) -> anyhow::Result<Arc<Image>> {
		self.wrap_external_image_inner(raw_handle, info, Some(initial_layout))
	}

	fn wrap_external_image_inner(
		&self,
		raw_handle: u64,
		info: ImageCreateInfo,
		initial_layout: Option<ImageLayout>,
	) -> anyhow::Result<Arc<Image>> {
		if info.extent.contains(&0) {
			anyhow::bail!("image extent must be non-zero");
		}
		if info.array_layers == 0 {
			anyhow::bail!("image array_layers must be non-zero");
		}
		let raw = self.raw.wrap_external_image(raw_handle, initial_layout);
		Ok(Arc::new(Image::from_raw(
			raw,
			info.extent,
			info.format,
			info.usage,
			info.tiling,
			info.array_layers,
		)))
	}

	pub fn new_image(&self, width: u32, height: u32, format: Format, usage: ImageUsage) -> anyhow::Result<Arc<Image>> {
		self.create_image(ImageCreateInfo::new_2d(width, height, format, usage))
	}

	pub fn create_image_view(&self, image: Arc<Image>) -> anyhow::Result<Arc<ImageView>> {
		let info = ImageViewCreateInfo {
			array_layer_count: image.array_layers(),
			..ImageViewCreateInfo::default()
		};
		self.create_image_view_with_info(image, info)
	}

	pub fn create_image_view_with_info(
		&self,
		image: Arc<Image>,
		info: ImageViewCreateInfo,
	) -> anyhow::Result<Arc<ImageView>> {
		if info.mip_level_count != 1 || info.base_mip_level != 0 {
			anyhow::bail!("WGfx images currently expose exactly one mip level");
		}
		if info.array_layer_count == 0
			|| info.base_array_layer >= image.array_layers()
			|| info.base_array_layer + info.array_layer_count > image.array_layers()
		{
			anyhow::bail!("image-view array layer range is out of bounds");
		}
		let raw = self
			.raw
			.create_image_view(image.raw.clone(), image.format(), image.array_layers(), info)?;
		Ok(Arc::new(ImageView::from_raw(raw, image)))
	}

	pub fn create_sampler(&self, info: SamplerCreateInfo) -> anyhow::Result<Arc<Sampler>> {
		let mut cache = self.samplers.lock();
		if let Some(existing) = cache.get(&info).and_then(Weak::upgrade) {
			return Ok(existing);
		}
		let raw = self.raw.create_sampler(info)?;
		let sampler = Arc::new(Sampler::from_raw(raw, info));
		cache.insert(info, Arc::downgrade(&sampler));
		Ok(sampler)
	}

	pub fn create_shader_module(
		&self,
		spirv: &[u32],
		stage: ShaderStage,
		bindings: &[DescriptorBinding],
	) -> anyhow::Result<Arc<ShaderModule>> {
		let raw = self.raw.create_shader_module(spirv, stage)?;
		Ok(Arc::new(ShaderModule::from_raw(raw, stage, bindings.to_vec())))
	}

	pub fn create_shader_module_bytes(
		&self,
		spirv: &[u8],
		stage: ShaderStage,
		bindings: &[DescriptorBinding],
	) -> anyhow::Result<Arc<ShaderModule>> {
		let words = spirv_words(spirv)?;
		self.create_shader_module(&words, stage, bindings)
	}

	pub fn create_pipeline<V>(
		self: &Arc<Self>,
		vert: &Arc<ShaderModule>,
		frag: &Arc<ShaderModule>,
		info: WPipelineCreateInfo,
	) -> anyhow::Result<Arc<WGfxPipeline<V>>>
	where
		V: Vertex,
	{
		Ok(Arc::new(WGfxPipeline::new(self.clone(), vert, frag, info)?))
	}

	pub fn create_gfx_command_buffer(
		self: &Arc<Self>,
		usage: CommandBufferUsage,
	) -> anyhow::Result<GfxCommandBufferBuilder> {
		cmd::CommandBufferBuilder::new(self.clone(), QueueType::Graphics, usage, PhantomData)
	}

	pub fn create_xfer_command_buffer(
		self: &Arc<Self>,
		usage: CommandBufferUsage,
	) -> anyhow::Result<XferCommandBufferBuilder> {
		cmd::CommandBufferBuilder::new(self.clone(), QueueType::Graphics, usage, PhantomData)
	}

	/// Whether a second queue was reserved for capture work
	pub fn has_capture_queue(&self) -> bool {
		self.raw.has_capture_queue()
	}

	pub fn create_capture_command_buffer(
		self: &Arc<Self>,
		usage: CommandBufferUsage,
	) -> anyhow::Result<XferCommandBufferBuilder> {
		if !self.raw.has_capture_queue() {
			anyhow::bail!("no dedicated capture queue available");
		}
		cmd::CommandBufferBuilder::new(self.clone(), QueueType::Transfer, usage, PhantomData)
	}
}
