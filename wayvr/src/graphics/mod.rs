pub mod dds;
pub mod dmabuf;

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use glam::{Vec2, vec2};
use smallvec::SmallVec;
use vulkano::{
    buffer::{BufferCreateInfo, BufferUsage},
    command_buffer::{CommandBufferUsage, PrimaryAutoCommandBuffer, PrimaryCommandBufferAbstract},
    image::view::ImageView,
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter},
    sync::GpuFuture,
};
use wgui::gfx::WGfx;

#[cfg(feature = "openvr")]
use vulkano::instance::InstanceCreateFlags;
use wlx_capture::DrmFormat;

use crate::shaders::{frag_color, frag_grid, frag_screen, frag_sky, frag_srgb, vert_quad};

#[cfg(feature = "openxr")]
use {ash::vk, std::os::raw::c_void};

use vulkano::{
    self, VulkanObject,
    buffer::{Buffer, BufferContents, Subbuffer},
    device::{
        DeviceCreateInfo, DeviceExtensions, DeviceFeatures, Queue, QueueCreateInfo, QueueFlags,
        physical::{PhysicalDevice, PhysicalDeviceType},
    },
    format::Format,
    instance::{Instance, InstanceCreateInfo, InstanceExtensions},
    pipeline::graphics::vertex_input::Vertex,
    shader::ShaderModule,
};

use dmabuf::get_drm_formats;

pub type Vert2Buf = Subbuffer<[Vert2Uv]>;

#[repr(C)]
#[derive(BufferContents, Vertex, Copy, Clone, Debug)]
pub struct Vert2Uv {
    #[format(R32G32_SFLOAT)]
    pub in_pos: [f32; 2],
    #[format(R32G32_SFLOAT)]
    pub in_uv: [f32; 2],
}

pub struct WGfxExtras {
    pub shaders: HashMap<&'static str, Arc<ShaderModule>>,
    pub drm_formats: Arc<[DrmFormat]>,
    pub queue_capture: Option<Arc<Queue>>,
    pub quad_verts: Vert2Buf,
    pub fallback_image: Arc<ImageView>,
    pub drm_device: Option<(i64, i64)>,
}

impl WGfxExtras {
    pub fn new(gfx: Arc<WGfx>, queue_capture: Option<Arc<Queue>>) -> anyhow::Result<Self> {
        let mut shaders = HashMap::new();

        let shader = vert_quad::load(gfx.device.clone())?;
        shaders.insert("vert_quad", shader);

        let shader = frag_color::load(gfx.device.clone())?;
        shaders.insert("frag_color", shader);

        let shader = frag_srgb::load(gfx.device.clone())?;
        shaders.insert("frag_srgb", shader);

        let shader = frag_sky::load(gfx.device.clone())?;
        shaders.insert("frag_sky", shader);

        let shader = frag_grid::load(gfx.device.clone())?;
        shaders.insert("frag_grid", shader);

        let shader = frag_screen::load(gfx.device.clone())?;
        shaders.insert("frag_screen", shader);

        let drm_formats = get_drm_formats(gfx.device.clone()).into();

        let vertices = [
            Vert2Uv {
                in_pos: [0., 0.],
                in_uv: [0., 0.],
            },
            Vert2Uv {
                in_pos: [1., 0.],
                in_uv: [1., 0.],
            },
            Vert2Uv {
                in_pos: [0., 1.],
                in_uv: [0., 1.],
            },
            Vert2Uv {
                in_pos: [1., 1.],
                in_uv: [1., 1.],
            },
        ];
        let quad_verts = Buffer::from_iter(
            gfx.memory_allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::VERTEX_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            vertices.into_iter(),
        )?;

        let mut cmd_xfer = gfx.create_xfer_command_buffer(CommandBufferUsage::OneTimeSubmit)?;
        let fallback_image =
            cmd_xfer.upload_image(1, 1, Format::R8G8B8A8_SRGB, &[255, 0, 255, 255])?;
        cmd_xfer.build_and_execute_now()?;

        let fallback_image = ImageView::new_default(fallback_image)?;

        let p = gfx.device.physical_device().properties();

        let drm_device = if let (Some(maj), Some(min)) = (p.render_major, p.render_minor) {
            log::info!("DRM render device: {maj} {min}");
            Some((maj, min))
        } else {
            log::warn!("No DRM device.");
            None
        };

        Ok(Self {
            shaders,
            drm_formats,
            queue_capture,
            quad_verts,
            fallback_image,
            drm_device,
        })
    }
}

const fn get_dmabuf_extensions() -> DeviceExtensions {
    DeviceExtensions {
        khr_external_memory: true,
        khr_external_memory_fd: true,
        ext_external_memory_dma_buf: true,
        ..DeviceExtensions::empty()
    }
}

static VULKAN_LIBRARY: OnceLock<Arc<vulkano::VulkanLibrary>> = OnceLock::new();
fn get_vulkan_library() -> &'static Arc<vulkano::VulkanLibrary> {
    VULKAN_LIBRARY.get_or_init(|| vulkano::VulkanLibrary::new().unwrap()) // want panic
}

#[cfg(feature = "openxr")]
unsafe extern "system" fn get_instance_proc_addr(
    instance: openxr::sys::platform::VkInstance,
    name: *const std::ffi::c_char,
) -> Option<unsafe extern "system" fn()> {
    use vulkano::Handle;
    let instance = ash::vk::Instance::from_raw(instance as _);
    let library = get_vulkan_library();
    unsafe { library.get_instance_proc_addr(instance, name) }
}

#[cfg(feature = "openxr")]
#[allow(clippy::too_many_lines)]
pub fn init_openxr_graphics(
    xr_instance: openxr::Instance,
    system: openxr::SystemId,
) -> anyhow::Result<(Arc<WGfx>, WGfxExtras)> {
    use std::ffi::{self, CString};

    use vulkano::{Handle, Version};

    let instance_extensions = InstanceExtensions {
        khr_get_physical_device_properties2: true,
        ..InstanceExtensions::empty()
    };

    let instance_extensions_raw = instance_extensions
        .into_iter()
        .filter_map(|(name, enabled)| {
            if enabled {
                Some(ffi::CString::new(name).unwrap().into_raw().cast_const())
            // want panic
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let vk_target_version = vk::make_api_version(0, 1, 3, 0);
    let target_version = vulkano::Version::V1_3;
    let library = get_vulkan_library();

    let vk_app_info_raw = vk::ApplicationInfo::default()
        .application_version(0)
        .engine_version(0)
        .api_version(vk_target_version);

    let instance = unsafe {
        let vk_instance = xr_instance
            .create_vulkan_instance(
                system,
                get_instance_proc_addr,
                std::ptr::from_ref(
                    &vk::InstanceCreateInfo::default()
                        .application_info(&vk_app_info_raw)
                        .enabled_extension_names(&instance_extensions_raw),
                )
                .cast(),
            )
            .expect("XR error creating Vulkan instance")
            .map_err(vk::Result::from_raw)
            .expect("Vulkan error creating Vulkan instance");

        Instance::from_handle(
            library.clone(),
            ash::vk::Instance::from_raw(vk_instance as _),
            InstanceCreateInfo {
                application_version: Version::major_minor(0, 0),
                engine_version: Version::major_minor(0, 0),
                max_api_version: Some(Version::V1_3),
                enabled_extensions: instance_extensions,
                ..Default::default()
            },
        )
    };

    let physical_device = unsafe {
        PhysicalDevice::from_handle(
            instance.clone(),
            vk::PhysicalDevice::from_raw(
                xr_instance.vulkan_graphics_device(system, instance.handle().as_raw() as _)? as _,
            ),
        )
    }?;

    let vk_device_properties = physical_device.properties();
    assert!(
        (vk_device_properties.api_version >= target_version),
        "Vulkan physical device doesn't support Vulkan {target_version}"
    );

    log::info!(
        "Using vkPhysicalDevice: {}",
        physical_device.properties().device_name,
    );

    let queue_families = try_all_queue_families(physical_device.as_ref())
        .expect("vkPhysicalDevice does not have a GRAPHICS / TRANSFER queue.");

    let mut device_extensions = DeviceExtensions::empty();
    let dmabuf_extensions = get_dmabuf_extensions();

    if physical_device
        .supported_extensions()
        .contains(&dmabuf_extensions)
    {
        device_extensions = device_extensions.union(&dmabuf_extensions);
        device_extensions.ext_image_drm_format_modifier = physical_device
            .supported_extensions()
            .ext_image_drm_format_modifier;
    }

    if physical_device
        .supported_extensions()
        .ext_physical_device_drm
    {
        device_extensions.ext_physical_device_drm = true;
    }

    let device_extensions_raw = device_extensions
        .into_iter()
        .filter_map(|(name, enabled)| {
            if enabled {
                Some(ffi::CString::new(name).unwrap().into_raw().cast_const())
            // want panic
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    // If adding anything here, also add to the native device_create_info below
    let features = DeviceFeatures {
        dynamic_rendering: true,
        descriptor_binding_sampled_image_update_after_bind: true,
        ..DeviceFeatures::empty()
    };

    let queue_create_infos = queue_families
        .iter()
        .map(|fam| {
            vk::DeviceQueueCreateInfo::default()
                .queue_family_index(fam.queue_family_index)
                .queue_priorities(&fam.priorities)
        })
        .collect::<Vec<_>>();

    let mut device_create_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queue_create_infos)
        .enabled_extension_names(&device_extensions_raw);

    let mut dynamic_rendering =
        vk::PhysicalDeviceDynamicRenderingFeatures::default().dynamic_rendering(true);

    let mut indexing_features = vk::PhysicalDeviceDescriptorIndexingFeatures::default()
        .descriptor_binding_sampled_image_update_after_bind(true);

    dynamic_rendering.p_next = device_create_info.p_next.cast_mut();
    indexing_features.p_next = (&raw mut dynamic_rendering).cast::<c_void>();
    device_create_info.p_next = &raw mut indexing_features as *const c_void;

    let (device, queues) = unsafe {
        let vk_device = xr_instance
            .create_vulkan_device(
                system,
                get_instance_proc_addr,
                physical_device.handle().as_raw() as _,
                (&raw const device_create_info).cast(),
            )
            .expect("XR error creating Vulkan device")
            .map_err(vk::Result::from_raw)
            .expect("Vulkan error creating Vulkan device");

        vulkano::device::Device::from_handle(
            physical_device,
            vk::Device::from_raw(vk_device as _),
            DeviceCreateInfo {
                queue_create_infos: queue_families
                    .iter()
                    .map(|fam| QueueCreateInfo {
                        queue_family_index: fam.queue_family_index,
                        queues: fam.priorities.clone(),
                        ..Default::default()
                    })
                    .collect::<Vec<_>>(),
                enabled_extensions: device_extensions,
                enabled_features: features,
                ..Default::default()
            },
        )
    };

    log::debug!(
        "  DMA-buf supported: {}",
        device.enabled_extensions().ext_external_memory_dma_buf
    );
    log::debug!(
        "  DRM format modifiers supported: {}",
        device.enabled_extensions().ext_image_drm_format_modifier
    );

    // Drop the CStrings
    device_extensions_raw
        .into_iter()
        .for_each(|c_string| unsafe {
            let _ = CString::from_raw(c_string.cast_mut());
        });

    let (queue_gfx, queue_xfer, queue_capture) = unwrap_queues(queues.collect());

    let gfx = WGfx::new_from_raw(
        instance,
        device,
        queue_gfx,
        queue_xfer,
        Format::R8G8B8A8_SRGB,
    );
    let extras = WGfxExtras::new(gfx.clone(), queue_capture)?;

    Ok((gfx, extras))
}

#[allow(clippy::too_many_lines)]
#[cfg(feature = "openvr")]
pub fn init_openvr_graphics(
    mut vk_instance_extensions: InstanceExtensions,
    mut vk_device_extensions_fn: impl FnMut(&PhysicalDevice) -> DeviceExtensions,
) -> anyhow::Result<(Arc<WGfx>, WGfxExtras)> {
    use vulkano::device::Device;

    //#[cfg(debug_assertions)]
    //let layers = vec!["VK_LAYER_KHRONOS_validation".to_owned()];
    //#[cfg(not(debug_assertions))]

    let layers = vec![];

    log::debug!("Instance exts for runtime: {:?}", &vk_instance_extensions);

    vk_instance_extensions.khr_get_physical_device_properties2 = true;

    let instance = Instance::new(
        get_vulkan_library().clone(),
        InstanceCreateInfo {
            flags: InstanceCreateFlags::ENUMERATE_PORTABILITY,
            enabled_extensions: vk_instance_extensions,
            enabled_layers: layers,
            ..Default::default()
        },
    )?;

    let dmabuf_extensions = get_dmabuf_extensions();

    let (physical_device, my_extensions, queue_families) = instance
        .enumerate_physical_devices()?
        .filter_map(|p| {
            let mut my_extensions = vk_device_extensions_fn(&p);

            if !p.supported_extensions().contains(&my_extensions) {
                log::debug!(
                    "Not using {} due to missing extensions:",
                    p.properties().device_name,
                );
                for (ext, missing) in p.supported_extensions().difference(&my_extensions) {
                    if missing {
                        log::debug!("  {ext}");
                    }
                }
                return None;
            }

            if p.supported_extensions().contains(&dmabuf_extensions) {
                my_extensions = my_extensions.union(&dmabuf_extensions);
                my_extensions.ext_image_drm_format_modifier =
                    p.supported_extensions().ext_image_drm_format_modifier;
            }

            if p.supported_extensions().ext_filter_cubic {
                my_extensions.ext_filter_cubic = true;
            }

            if p.supported_extensions().ext_physical_device_drm {
                // needed for wayland_server
                my_extensions.ext_physical_device_drm = true;
            }

            log::debug!(
                "Device exts for {}: {:?}",
                p.properties().device_name,
                &my_extensions
            );
            Some((p, my_extensions))
        })
        .filter_map(|(p, my_extensions)| {
            try_all_queue_families(p.as_ref()).map(|families| (p, my_extensions, families))
        })
        .min_by_key(|(p, _, families)| prio_from_device_type(p) * 10 + prio_from_families(families))
        .expect("no suitable physical device found");

    log::info!(
        "Using vkPhysicalDevice: {}",
        physical_device.properties().device_name,
    );

    let (device, queues) = Device::new(
        physical_device,
        DeviceCreateInfo {
            enabled_extensions: my_extensions,
            enabled_features: DeviceFeatures {
                dynamic_rendering: true,
                descriptor_binding_sampled_image_update_after_bind: true,
                ..DeviceFeatures::empty()
            },
            queue_create_infos: queue_families
                .iter()
                .map(|fam| QueueCreateInfo {
                    queue_family_index: fam.queue_family_index,
                    queues: fam.priorities.clone(),
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
            ..Default::default()
        },
    )?;

    log::debug!(
        "  DMA-buf supported: {}",
        device.enabled_extensions().ext_external_memory_dma_buf
    );
    log::debug!(
        "  DRM format modifiers supported: {}",
        device.enabled_extensions().ext_image_drm_format_modifier
    );

    let (queue_gfx, queue_xfer, queue_capture) = unwrap_queues(queues.collect());

    let gfx = WGfx::new_from_raw(
        instance,
        device,
        queue_gfx,
        queue_xfer,
        Format::R8G8B8A8_SRGB,
    );
    let extras = WGfxExtras::new(gfx.clone(), queue_capture)?;

    Ok((gfx, extras))
}

pub fn upload_quad_vertices(
    buf: &mut Subbuffer<[Vert2Uv]>,
    width: f32,
    height: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> anyhow::Result<()> {
    let rw = width;
    let rh = height;

    let x0 = x / rw;
    let y0 = y / rh;

    let x1 = w / rw + x0;
    let y1 = h / rh + y0;

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

    buf.write()?[0..4].copy_from_slice(&data);
    Ok(())
}

#[derive(Debug)]
struct QueueFamilyLayout {
    queue_family_index: u32,
    priorities: Vec<f32>,
}

fn prio_from_device_type(physical_device: &PhysicalDevice) -> u32 {
    match physical_device.properties().device_type {
        PhysicalDeviceType::DiscreteGpu => 0,
        PhysicalDeviceType::IntegratedGpu => 1,
        PhysicalDeviceType::VirtualGpu => 2,
        PhysicalDeviceType::Cpu => 3,
        _ => 4,
    }
}

const fn prio_from_families(families: &[QueueFamilyLayout]) -> u32 {
    match families.len() {
        2 | 3 => 0,
        _ => 1,
    }
}

fn unwrap_queues(queues: Vec<Arc<Queue>>) -> (Arc<Queue>, Arc<Queue>, Option<Arc<Queue>>) {
    match queues[..] {
        [ref g, ref t, ref c] => (g.clone(), t.clone(), Some(c.clone())),
        [ref gt, ref c] => (gt.clone(), gt.clone(), Some(c.clone())),
        [ref gt] => (gt.clone(), gt.clone(), None),
        _ => unreachable!(),
    }
}

fn try_all_queue_families(physical_device: &PhysicalDevice) -> Option<Vec<QueueFamilyLayout>> {
    queue_families_priorities(
        physical_device,
        vec![
            // main-thread graphics + uploads
            QueueFlags::GRAPHICS | QueueFlags::TRANSFER,
            // capture-thread uploads
            QueueFlags::TRANSFER,
        ],
    )
    .or_else(|| {
        queue_families_priorities(
            physical_device,
            vec![
                // main thread graphics
                QueueFlags::GRAPHICS,
                // main thread uploads
                QueueFlags::TRANSFER,
                // capture thread uploads
                QueueFlags::TRANSFER,
            ],
        )
    })
    .or_else(|| {
        queue_families_priorities(
            physical_device,
            // main thread-only. software capture not supported.
            vec![QueueFlags::GRAPHICS | QueueFlags::TRANSFER],
        )
    })
}

fn queue_families_priorities(
    physical_device: &PhysicalDevice,
    mut requested_queues: Vec<QueueFlags>,
) -> Option<Vec<QueueFamilyLayout>> {
    let mut result = Vec::with_capacity(3);

    for (idx, props) in physical_device.queue_family_properties().iter().enumerate() {
        let mut remaining = props.queue_count;
        let mut want = 0usize;

        requested_queues.retain(|requested| {
            if props.queue_flags.intersects(*requested) && remaining > 0 {
                remaining -= 1;
                want += 1;
                false
            } else {
                true
            }
        });

        if want > 0 {
            result.push(QueueFamilyLayout {
                queue_family_index: idx as u32,
                priorities: std::iter::repeat_n(1.0, want).collect(),
            });
        }
    }

    if requested_queues.is_empty() {
        log::debug!("Selected GPU queue families: {result:?}");
        Some(result)
    } else {
        None
    }
}

pub struct RenderResult {
    pub queue: Arc<Queue>,
    pub cmd_buf: Arc<PrimaryAutoCommandBuffer>,
}

#[derive(Default)]
pub struct GpuFutures {
    futures: Vec<Box<dyn GpuFuture>>,
}

impl GpuFutures {
    pub fn execute(
        &mut self,
        queue: Arc<Queue>,
        cmd_buf: Arc<PrimaryAutoCommandBuffer>,
    ) -> anyhow::Result<()> {
        self.futures.push(cmd_buf.execute(queue)?.boxed());
        Ok(())
    }

    pub fn execute_results(&mut self, results: SmallVec<[RenderResult; 2]>) -> anyhow::Result<()> {
        for (i, res) in results.into_iter().enumerate() {
            if i == 0 {
                let future = res.cmd_buf.execute(res.queue)?;
                self.futures.push(Box::new(future));
            } else {
                let future = self.futures.pop().unwrap();
                let future = future.then_execute(res.queue, res.cmd_buf)?;
                self.futures.push(Box::new(future));
            }
        }
        Ok(())
    }

    pub fn wait(self) -> anyhow::Result<()> {
        let mut it = self.futures.into_iter();
        let Some(mut all) = it.next() else {
            return Ok(());
        };
        for f in it {
            all = all.join(f).boxed();
        }

        let finished = all.then_signal_fence_and_flush()?;
        finished.wait(None)?;

        Ok(())
    }
}

pub trait ExtentExt {
    fn extent_f32(&self) -> [f32; 2];
    #[allow(dead_code)]
    fn extent_vec2(&self) -> Vec2;
    fn extent_u32arr(&self) -> [u32; 2];
}

impl ExtentExt for Arc<ImageView> {
    fn extent_f32(&self) -> [f32; 2] {
        let [w, h, _] = self.image().extent();
        [w as _, h as _]
    }
    fn extent_vec2(&self) -> Vec2 {
        let [w, h, _] = self.image().extent();
        vec2(w as _, h as _)
    }
    fn extent_u32arr(&self) -> [u32; 2] {
        let [w, h, _] = self.image().extent();
        [w, h]
    }
}

impl ExtentExt for [u32; 3] {
    fn extent_f32(&self) -> [f32; 2] {
        let [w, h, _] = *self;
        [w as _, h as _]
    }
    fn extent_vec2(&self) -> Vec2 {
        let [w, h, _] = *self;
        Vec2 {
            x: w as _,
            y: h as _,
        }
    }
    fn extent_u32arr(&self) -> [u32; 2] {
        let [w, h, _] = *self;
        [w, h]
    }
}
