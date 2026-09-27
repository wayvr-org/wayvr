use std::{
	fmt,
	ops::{BitOr, BitOrAssign},
};

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
	R8_UNORM,
	R8G8B8_UNORM,
	R8G8B8A8_UNORM,
	R8G8B8A8_SRGB,
	B8G8R8_UNORM,
	B8G8R8A8_UNORM,
	B8G8R8A8_SRGB,
	A2B10G10R10_UNORM_PACK32,
	R16G16B16A16_SFLOAT,
	R32G32B32A32_SFLOAT,
	BC1_RGBA_UNORM_BLOCK,
	BC1_RGBA_SRGB_BLOCK,
	BC2_UNORM_BLOCK,
	BC2_SRGB_BLOCK,
	BC3_UNORM_BLOCK,
	BC3_SRGB_BLOCK,
	BC4_UNORM_BLOCK,
	BC4_SNORM_BLOCK,
	BC5_UNORM_BLOCK,
	BC5_SNORM_BLOCK,
	BC6H_UFLOAT_BLOCK,
	BC6H_SFLOAT_BLOCK,
	BC7_UNORM_BLOCK,
	BC7_SRGB_BLOCK,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BufferUsage(u32);

impl BufferUsage {
	pub const TRANSFER_SRC: Self = Self(1 << 0);
	pub const TRANSFER_DST: Self = Self(1 << 1);
	pub const UNIFORM_BUFFER: Self = Self(1 << 2);
	pub const STORAGE_BUFFER: Self = Self(1 << 3);
	pub const INDEX_BUFFER: Self = Self(1 << 4);
	pub const VERTEX_BUFFER: Self = Self(1 << 5);

	pub const fn empty() -> Self {
		Self(0)
	}

	pub const fn contains(self, other: Self) -> bool {
		self.0 & other.0 == other.0
	}
}

impl BitOr for BufferUsage {
	type Output = Self;

	fn bitor(self, rhs: Self) -> Self::Output {
		Self(self.0 | rhs.0)
	}
}

impl BitOrAssign for BufferUsage {
	fn bitor_assign(&mut self, rhs: Self) {
		self.0 |= rhs.0;
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ImageUsage(u32);

impl ImageUsage {
	pub const TRANSFER_SRC: Self = Self(1 << 0);
	pub const TRANSFER_DST: Self = Self(1 << 1);
	pub const SAMPLED: Self = Self(1 << 2);
	pub const COLOR_ATTACHMENT: Self = Self(1 << 3);

	pub const fn empty() -> Self {
		Self(0)
	}

	pub const fn contains(self, other: Self) -> bool {
		self.0 & other.0 == other.0
	}
}

impl BitOr for ImageUsage {
	type Output = Self;

	fn bitor(self, rhs: Self) -> Self::Output {
		Self(self.0 | rhs.0)
	}
}

impl BitOrAssign for ImageUsage {
	fn bitor_assign(&mut self, rhs: Self) {
		self.0 |= rhs.0;
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageTiling {
	Optimal,
	Linear,
	DrmFormatModifier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Filter {
	Nearest,
	Linear,
	Cubic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SamplerAddressMode {
	Repeat,
	MirroredRepeat,
	ClampToEdge,
	ClampToBorder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerCreateInfo {
	pub mag_filter: Filter,
	pub min_filter: Filter,
	pub address_mode: [SamplerAddressMode; 3],
}

impl Default for SamplerCreateInfo {
	fn default() -> Self {
		Self {
			mag_filter: Filter::Linear,
			min_filter: Filter::Linear,
			address_mode: [SamplerAddressMode::Repeat; 3],
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandBufferUsage {
	OneTimeSubmit,
	MultipleSubmit,
	SimultaneousUse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueueType {
	Graphics,
	Transfer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageLayout {
	TransferSrc,
	TransferDst,
	ShaderReadOnly,
	ColorAttachment,
	Present,
	General,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveTopology {
	PointList,
	LineList,
	LineStrip,
	TriangleList,
	TriangleStrip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlendFactor {
	Zero,
	One,
	SrcAlpha,
	OneMinusSrcAlpha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlendOp {
	Add,
	Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AttachmentBlend {
	pub src_color_blend_factor: BlendFactor,
	pub dst_color_blend_factor: BlendFactor,
	pub color_blend_op: BlendOp,
	pub src_alpha_blend_factor: BlendFactor,
	pub dst_alpha_blend_factor: BlendFactor,
	pub alpha_blend_op: BlendOp,
}

pub const BLEND_ALPHA: AttachmentBlend = AttachmentBlend {
	src_color_blend_factor: BlendFactor::SrcAlpha,
	dst_color_blend_factor: BlendFactor::OneMinusSrcAlpha,
	color_blend_op: BlendOp::Add,
	src_alpha_blend_factor: BlendFactor::One,
	dst_alpha_blend_factor: BlendFactor::One,
	alpha_blend_op: BlendOp::Max,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShaderStage {
	Vertex,
	Fragment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ShaderStages(u32);

impl ShaderStages {
	pub const VERTEX: Self = Self(1 << 0);
	pub const FRAGMENT: Self = Self(1 << 1);

	pub const fn from_stage(stage: ShaderStage) -> Self {
		match stage {
			ShaderStage::Vertex => Self::VERTEX,
			ShaderStage::Fragment => Self::FRAGMENT,
		}
	}

	pub const fn contains(self, other: Self) -> bool {
		self.0 & other.0 == other.0
	}
}

impl BitOr for ShaderStages {
	type Output = Self;

	fn bitor(self, rhs: Self) -> Self::Output {
		Self(self.0 | rhs.0)
	}
}

impl BitOrAssign for ShaderStages {
	fn bitor_assign(&mut self, rhs: Self) {
		self.0 |= rhs.0;
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DescriptorType {
	CombinedImageSampler,
	UniformBuffer,
	StorageBuffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DescriptorBinding {
	pub set: u32,
	pub binding: u32,
	pub descriptor_type: DescriptorType,
	pub descriptor_count: u32,
	pub stages: ShaderStages,
}

impl DescriptorBinding {
	pub const fn new(set: u32, binding: u32, descriptor_type: DescriptorType, stage: ShaderStage) -> Self {
		Self {
			set,
			binding,
			descriptor_type,
			descriptor_count: 1,
			stages: ShaderStages::from_stage(stage),
		}
	}

	#[must_use]
	pub const fn with_count(mut self, descriptor_count: u32) -> Self {
		self.descriptor_count = descriptor_count;
		self
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VertexFormat {
	R32Uint,
	R32Sfloat,
	R32g32Sfloat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VertexAttribute {
	pub location: u32,
	pub format: VertexFormat,
	pub offset: u32,
}

impl VertexAttribute {
	pub const fn new(location: u32, format: VertexFormat, offset: u32) -> Self {
		Self {
			location,
			format,
			offset,
		}
	}
}

pub trait Vertex: Copy + Send + Sync + 'static {
	fn attributes() -> Vec<VertexAttribute>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scissor {
	pub offset: [i32; 2],
	pub extent: [u32; 2],
}

impl Scissor {
	pub const fn new(offset: [i32; 2], extent: [u32; 2]) -> Self {
		Self { offset, extent }
	}

	pub const fn from_viewport(dimensions: [f32; 2], offset: [f32; 2]) -> Self {
		Self::new(
			[offset[0] as i32, offset[1] as i32],
			[dimensions[0] as u32, dimensions[1] as u32],
		)
	}
}

#[derive(Debug, Clone, Copy)]
pub struct ImageCreateInfo {
	pub extent: [u32; 3],
	pub format: Format,
	pub usage: ImageUsage,
	pub tiling: ImageTiling,
	pub array_layers: u32,
}

impl ImageCreateInfo {
	pub const fn new_2d(width: u32, height: u32, format: Format, usage: ImageUsage) -> Self {
		Self {
			extent: [width, height, 1],
			format,
			usage,
			tiling: ImageTiling::Optimal,
			array_layers: 1,
		}
	}
}

#[derive(Debug, Clone, Copy)]
pub struct ImageViewCreateInfo {
	pub base_mip_level: u32,
	pub mip_level_count: u32,
	pub base_array_layer: u32,
	pub array_layer_count: u32,
}

impl Default for ImageViewCreateInfo {
	fn default() -> Self {
		Self {
			base_mip_level: 0,
			mip_level_count: 1,
			base_array_layer: 0,
			array_layer_count: 1,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceType {
	Other,
	IntegratedGpu,
	DiscreteGpu,
	VirtualGpu,
	Cpu,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DevicePreference {
	#[default]
	Default,
	VendorDevice {
		vendor_id: u32,
		device_id: u32,
	},
	Uuid([u8; 16]),
	DrmRenderNode {
		major: u32,
		minor: u32,
	},
}

#[derive(Debug, Clone)]
pub struct WGfxCreateInfo {
	pub application_name: String,
	pub application_version: u32,
	pub engine_name: String,
	pub engine_version: u32,
	pub surface_format: Format,
	pub required_instance_extensions: Vec<String>,
	pub required_device_extensions: Vec<String>,
	pub optional_device_extensions: Vec<String>,
	pub device_preference: DevicePreference,
}

impl Default for WGfxCreateInfo {
	fn default() -> Self {
		Self {
			application_name: "wgui".to_owned(),
			application_version: 0,
			engine_name: "wgui".to_owned(),
			engine_version: 0,
			surface_format: Format::R8G8B8A8_SRGB,
			required_instance_extensions: Vec::new(),
			required_device_extensions: Vec::new(),
			optional_device_extensions: Vec::new(),
			device_preference: DevicePreference::Default,
		}
	}
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DeviceCaps {
	pub external_memory_dma_buf: bool,
	pub image_drm_format_modifier: bool,
	pub physical_device_drm: bool,
	pub filter_cubic: bool,
	pub descriptor_sampled_image_update_after_bind: bool,
	pub descriptor_uniform_buffer_update_after_bind: bool,
	pub descriptor_storage_buffer_update_after_bind: bool,
	pub dynamic_rendering: bool,
	pub max_image_dimension_2d: u32,
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
	pub name: String,
	pub vendor_id: u32,
	pub device_id: u32,
	pub device_type: DeviceType,
	pub api_version: u32,
	pub driver_version: u32,
	pub uuid: [u8; 16],
	pub drm_render_node: Option<(u32, u32)>,
}

impl fmt::Display for DeviceInfo {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{} ({:04x}:{:04x})", self.name, self.vendor_id, self.device_id)
	}
}
