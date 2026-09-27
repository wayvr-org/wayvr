use std::sync::Arc;

use wgui::gfx::{DescriptorBinding, DescriptorType, ShaderModule, ShaderStage, WGfx};

fn load(
    gfx: &WGfx,
    spirv: &[u8],
    stage: ShaderStage,
    bindings: &[DescriptorBinding],
) -> anyhow::Result<Arc<ShaderModule>> {
    gfx.create_shader_module_bytes(spirv, stage, bindings)
}

pub mod vert_quad {
    use super::{Arc, ShaderModule, ShaderStage, WGfx};

    pub fn load(gfx: &WGfx) -> anyhow::Result<Arc<ShaderModule>> {
        super::load(
            gfx,
            include_bytes!(concat!(env!("OUT_DIR"), "/quad.vert.spv")),
            ShaderStage::Vertex,
            &[],
        )
    }
}

pub mod frag_color {
    use super::{Arc, DescriptorBinding, DescriptorType, ShaderModule, ShaderStage, WGfx};

    pub fn load(gfx: &WGfx) -> anyhow::Result<Arc<ShaderModule>> {
        super::load(
            gfx,
            include_bytes!(concat!(env!("OUT_DIR"), "/color.frag.spv")),
            ShaderStage::Fragment,
            &[DescriptorBinding::new(
                0,
                0,
                DescriptorType::UniformBuffer,
                ShaderStage::Fragment,
            )],
        )
    }
}

pub mod frag_grid {
    use super::{Arc, ShaderModule, ShaderStage, WGfx};

    pub fn load(gfx: &WGfx) -> anyhow::Result<Arc<ShaderModule>> {
        super::load(
            gfx,
            include_bytes!(concat!(env!("OUT_DIR"), "/grid.frag.spv")),
            ShaderStage::Fragment,
            &[],
        )
    }
}

pub mod frag_screen {
    use super::{Arc, DescriptorBinding, DescriptorType, ShaderModule, ShaderStage, WGfx};

    pub fn load(gfx: &WGfx) -> anyhow::Result<Arc<ShaderModule>> {
        super::load(
            gfx,
            include_bytes!(concat!(env!("OUT_DIR"), "/screen.frag.spv")),
            ShaderStage::Fragment,
            &[DescriptorBinding::new(
                0,
                0,
                DescriptorType::CombinedImageSampler,
                ShaderStage::Fragment,
            )],
        )
    }
}

pub mod frag_simple {
    use super::{Arc, DescriptorBinding, DescriptorType, ShaderModule, ShaderStage, WGfx};

    pub fn load(gfx: &WGfx) -> anyhow::Result<Arc<ShaderModule>> {
        super::load(
            gfx,
            include_bytes!(concat!(env!("OUT_DIR"), "/simple.frag.spv")),
            ShaderStage::Fragment,
            &[DescriptorBinding::new(
                0,
                0,
                DescriptorType::CombinedImageSampler,
                ShaderStage::Fragment,
            )],
        )
    }
}

pub mod frag_srgb {
    use super::{Arc, DescriptorBinding, DescriptorType, ShaderModule, ShaderStage, WGfx};

    pub fn load(gfx: &WGfx) -> anyhow::Result<Arc<ShaderModule>> {
        super::load(
            gfx,
            include_bytes!(concat!(env!("OUT_DIR"), "/srgb.frag.spv")),
            ShaderStage::Fragment,
            &[
                DescriptorBinding::new(
                    0,
                    0,
                    DescriptorType::CombinedImageSampler,
                    ShaderStage::Fragment,
                ),
                DescriptorBinding::new(1, 0, DescriptorType::UniformBuffer, ShaderStage::Fragment),
            ],
        )
    }
}

pub mod frag_sky {
    use super::{Arc, ShaderModule, ShaderStage, WGfx};

    pub fn load(gfx: &WGfx) -> anyhow::Result<Arc<ShaderModule>> {
        super::load(
            gfx,
            include_bytes!(concat!(env!("OUT_DIR"), "/sky.frag.spv")),
            ShaderStage::Fragment,
            &[],
        )
    }
}
