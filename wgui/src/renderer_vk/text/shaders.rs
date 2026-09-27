use std::sync::Arc;

use crate::gfx::{DescriptorBinding, DescriptorType, ShaderModule, ShaderStage, WGfx};

pub mod vert_atlas {
	use super::{Arc, DescriptorBinding, DescriptorType, ShaderModule, ShaderStage, WGfx};

	pub fn load(gfx: &WGfx) -> anyhow::Result<Arc<ShaderModule>> {
		gfx.create_shader_module_bytes(
			include_bytes!(concat!(env!("OUT_DIR"), "/text.vert.spv")),
			ShaderStage::Vertex,
			&[
				DescriptorBinding::new(0, 0, DescriptorType::CombinedImageSampler, ShaderStage::Vertex),
				DescriptorBinding::new(1, 0, DescriptorType::CombinedImageSampler, ShaderStage::Vertex),
				DescriptorBinding::new(2, 0, DescriptorType::UniformBuffer, ShaderStage::Vertex),
				DescriptorBinding::new(3, 0, DescriptorType::StorageBuffer, ShaderStage::Vertex),
			],
		)
	}
}

pub mod frag_atlas {
	use super::{Arc, DescriptorBinding, DescriptorType, ShaderModule, ShaderStage, WGfx};

	pub fn load(gfx: &WGfx) -> anyhow::Result<Arc<ShaderModule>> {
		gfx.create_shader_module_bytes(
			include_bytes!(concat!(env!("OUT_DIR"), "/text.frag.spv")),
			ShaderStage::Fragment,
			&[
				DescriptorBinding::new(0, 0, DescriptorType::CombinedImageSampler, ShaderStage::Fragment),
				DescriptorBinding::new(1, 0, DescriptorType::CombinedImageSampler, ShaderStage::Fragment),
			],
		)
	}
}
