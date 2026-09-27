pub mod dds;
pub mod dmabuf;

use std::{collections::HashMap, sync::Arc};

use smallvec::SmallVec;
use wgui::gfx::{
    BufferUsage, BuiltCommandBuffer, Format, ShaderModule, Vert2Buf, Vert2Uv, WGfx, WGfxCreateInfo,
};
use wlx_capture::DrmFormat;

use crate::shaders::{
    frag_color, frag_grid, frag_screen, frag_simple, frag_sky, frag_srgb, vert_quad,
};

use dmabuf::get_drm_formats;

pub struct WGfxExtras {
    pub shaders: HashMap<&'static str, Arc<ShaderModule>>,
    pub drm_formats: Arc<[DrmFormat]>,
    pub quad_verts: Vert2Buf,
    pub drm_device: Option<(i64, i64)>,
}

impl WGfxExtras {
    pub fn new(gfx: &Arc<WGfx>) -> anyhow::Result<Self> {
        let mut shaders = HashMap::new();
        shaders.insert("vert_quad", vert_quad::load(gfx)?);
        shaders.insert("frag_color", frag_color::load(gfx)?);
        shaders.insert("frag_srgb", frag_srgb::load(gfx)?);
        shaders.insert("frag_sky", frag_sky::load(gfx)?);
        shaders.insert("frag_grid", frag_grid::load(gfx)?);
        shaders.insert("frag_screen", frag_screen::load(gfx)?);
        shaders.insert("frag_simple", frag_simple::load(gfx)?);

        let drm_formats = get_drm_formats(gfx).into();
        let vertices = [
            Vert2Uv {
                in_pos: [0.0, 0.0],
                in_uv: [0.0, 0.0],
            },
            Vert2Uv {
                in_pos: [1.0, 0.0],
                in_uv: [1.0, 0.0],
            },
            Vert2Uv {
                in_pos: [0.0, 1.0],
                in_uv: [0.0, 1.0],
            },
            Vert2Uv {
                in_pos: [1.0, 1.0],
                in_uv: [1.0, 1.0],
            },
        ];
        let quad_verts = gfx.new_buffer(BufferUsage::VERTEX_BUFFER, &vertices)?;

        let drm_device = gfx.device_info().drm_render_node.map(|(major, minor)| {
            log::info!("DRM render device: {major} {minor}");
            (i64::from(major), i64::from(minor))
        });
        if drm_device.is_none() {
            log::warn!("No DRM device.");
        }

        Ok(Self {
            shaders,
            drm_formats,
            quad_verts,
            drm_device,
        })
    }
}

fn base_create_info() -> WGfxCreateInfo {
    WGfxCreateInfo {
        application_name: "wayvr".to_owned(),
        engine_name: "wayvr".to_owned(),
        surface_format: Format::R8G8B8A8_SRGB,
        ..WGfxCreateInfo::default()
    }
}

fn log_device(gfx: &WGfx) {
    log::info!("Using vkPhysicalDevice: {}", gfx.device_info().name);
    log::debug!(
        "  DMA-buf supported: {}",
        gfx.capabilities().external_memory_dma_buf
    );
    log::debug!(
        "  DRM format modifiers supported: {}",
        gfx.capabilities().image_drm_format_modifier
    );
    log::debug!("  Separate capture queue: {}", gfx.has_capture_queue());
}

#[cfg(feature = "openxr")]
pub fn init_openxr_graphics(
    xr_instance: openxr::Instance,
    system: openxr::SystemId,
) -> anyhow::Result<(Arc<WGfx>, WGfxExtras)> {
    let mut create_info = base_create_info();
    create_info
        .required_instance_extensions
        .push("VK_KHR_get_physical_device_properties2".to_owned());

    let xr_for_instance = xr_instance.clone();
    let create_instance = move |get_instance_proc_addr: usize,
                                create_info: *const std::ffi::c_void| {
        let get_instance_proc_addr: openxr::sys::platform::VkGetInstanceProcAddr =
            unsafe { std::mem::transmute(get_instance_proc_addr) };
        let result = unsafe {
            xr_for_instance.create_vulkan_instance(
                system,
                get_instance_proc_addr,
                create_info.cast(),
            )
        }
        .map_err(|e| anyhow::anyhow!("XR error creating Vulkan instance: {e:?}"))?;
        let instance =
            result.map_err(|e| anyhow::anyhow!("Vulkan error creating Vulkan instance: {e:?}"))?;
        Ok(instance as u64)
    };

    let xr_for_physical_device = xr_instance.clone();
    let get_physical_device = move |instance: u64| {
        unsafe { xr_for_physical_device.vulkan_graphics_device(system, instance as _) }
            .map(|physical_device| physical_device as u64)
            .map_err(|e| anyhow::anyhow!("XR error getting Vulkan graphics device: {e:?}"))
    };

    let xr_for_device = xr_instance;
    let create_device = move |get_instance_proc_addr: usize,
                              physical_device: u64,
                              create_info: *const std::ffi::c_void| {
        let get_instance_proc_addr: openxr::sys::platform::VkGetInstanceProcAddr =
            unsafe { std::mem::transmute(get_instance_proc_addr) };
        let result = unsafe {
            xr_for_device.create_vulkan_device(
                system,
                get_instance_proc_addr,
                physical_device as _,
                create_info.cast(),
            )
        }
        .map_err(|e| anyhow::anyhow!("XR error creating Vulkan device: {e:?}"))?;
        let device =
            result.map_err(|e| anyhow::anyhow!("Vulkan error creating Vulkan device: {e:?}"))?;
        Ok(device as u64)
    };

    let gfx = unsafe {
        WGfx::new_external_vulkan(
            &create_info,
            create_instance,
            get_physical_device,
            create_device,
        )?
    };
    log_device(&gfx);
    let extras = WGfxExtras::new(&gfx)?;
    Ok((gfx, extras))
}

#[cfg(feature = "openvr")]
pub fn init_openvr_graphics(
    mut instance_extensions: Vec<String>,
    required_device_extensions: impl FnMut(u64) -> Vec<String>,
) -> anyhow::Result<(Arc<WGfx>, WGfxExtras)> {
    const PHYSICAL_DEVICE_PROPERTIES_2: &str = "VK_KHR_get_physical_device_properties2";
    if !instance_extensions
        .iter()
        .any(|extension| extension == PHYSICAL_DEVICE_PROPERTIES_2)
    {
        instance_extensions.push(PHYSICAL_DEVICE_PROPERTIES_2.to_owned());
    }

    log::debug!("Instance exts for runtime: {instance_extensions:?}");

    let mut create_info = base_create_info();
    create_info.required_instance_extensions = instance_extensions;
    let gfx = WGfx::new_with_device_extensions(&create_info, required_device_extensions)?;
    log_device(&gfx);
    let extras = WGfxExtras::new(&gfx)?;
    Ok((gfx, extras))
}

#[derive(Default)]
pub struct GpuFutures {
    command_buffers: Vec<Arc<BuiltCommandBuffer>>,
}

impl GpuFutures {
    pub fn execute(&mut self, command_buffer: Arc<BuiltCommandBuffer>) {
        self.command_buffers.push(command_buffer);
    }

    pub fn execute_results(&mut self, results: SmallVec<[Arc<BuiltCommandBuffer>; 2]>) {
        self.command_buffers.extend(results);
    }

    pub fn wait(self) -> anyhow::Result<()> {
        for command_buffer in self.command_buffers {
            command_buffer.submit_and_wait()?;
        }
        Ok(())
    }
}
