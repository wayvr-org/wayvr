//! This is the module that speaks with Vulkan directly (via ash).

use std::{
	collections::{BTreeMap, BTreeSet},
	ffi::{CStr, CString, c_void},
	fs::File,
	mem::ManuallyDrop,
	os::fd::{AsRawFd, FromRawFd},
	ptr::NonNull,
	sync::Arc,
};

use anyhow::{Context as _, anyhow, bail};
use ash::{
	Entry,
	vk::{self, Handle as _},
};
use parking_lot::Mutex;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use super::dmabuf::DmabufImportInfo;
use super::types::{
	AttachmentBlend, BlendFactor, BlendOp, BufferUsage, CommandBufferUsage, DescriptorBinding, DescriptorType,
	DeviceCaps, DeviceInfo, DevicePreference, DeviceType, Filter, Format, ImageCreateInfo, ImageLayout, ImageTiling,
	ImageUsage, ImageViewCreateInfo, PrimitiveTopology, QueueType, SamplerAddressMode, SamplerCreateInfo, Scissor,
	ShaderStage, ShaderStages, VertexAttribute, VertexFormat, WGfxCreateInfo,
};

const OPTIONAL_DEVICE_EXTENSIONS: &[&str] = &[
	"VK_EXT_filter_cubic",
	"VK_IMG_filter_cubic",
	"VK_KHR_external_memory",
	"VK_KHR_external_memory_fd",
	"VK_EXT_external_memory_dma_buf",
	"VK_EXT_image_drm_format_modifier",
	"VK_EXT_queue_family_foreign",
	"VK_EXT_physical_device_drm",
];

#[derive(Clone, Copy)]
pub(super) struct RawQueue {
	handle: vk::Queue,
	family_index: u32,
	queue_index: u32,
}

pub(super) struct Context {
	_entry: Entry,
	instance: ash::Instance,
	physical_device: vk::PhysicalDevice,
	device: ash::Device,
	memory_properties: vk::PhysicalDeviceMemoryProperties,
	queue_gfx: RawQueue,
	queue_xfer: RawQueue,
	has_capture_queue: bool,
	surface_loader: Option<ash::khr::surface::Instance>,
	surface: Option<vk::SurfaceKHR>,
	surface_format: Option<vk::SurfaceFormatKHR>,
	info: DeviceInfo,
	caps: DeviceCaps,
}

unsafe impl Send for Context {}
unsafe impl Sync for Context {}

impl Context {
	pub(super) fn new(info: &WGfxCreateInfo) -> anyhow::Result<Arc<Self>> {
		Self::new_impl(info, None, None)
	}

	pub(super) fn new_with_device_extensions(
		info: &WGfxCreateInfo,
		required_device_extensions: &mut dyn FnMut(u64) -> Vec<String>,
	) -> anyhow::Result<Arc<Self>> {
		Self::new_impl(info, None, Some(required_device_extensions))
	}

	pub(super) fn new_windowed(
		info: &WGfxCreateInfo,
		display_handle: RawDisplayHandle,
		window_handle: RawWindowHandle,
	) -> anyhow::Result<Arc<Self>> {
		Self::new_impl(info, Some((display_handle, window_handle)), None)
	}

	fn new_impl(
		info: &WGfxCreateInfo,
		window: Option<(RawDisplayHandle, RawWindowHandle)>,
		mut required_device_extensions_provider: Option<&mut dyn FnMut(u64) -> Vec<String>>,
	) -> anyhow::Result<Arc<Self>> {
		let entry = unsafe { Entry::load() }.context("loading Vulkan loader")?;

		let available_instance_extensions = unsafe { entry.enumerate_instance_extension_properties(None) }
			.map_err(|e| anyhow!("vkEnumerateInstanceExtensionProperties failed: {e:?}"))?
			.into_iter()
			.map(|p| {
				unsafe { CStr::from_ptr(p.extension_name.as_ptr()) }
					.to_string_lossy()
					.into_owned()
			})
			.collect::<BTreeSet<_>>();

		let mut required_instance_extensions = info.required_instance_extensions.clone();
		if let Some((display_handle, _)) = window.as_ref() {
			for extension in window_surface_extensions(*display_handle)? {
				if !required_instance_extensions.contains(&extension) {
					required_instance_extensions.push(extension);
				}
			}
		}

		for required in &required_instance_extensions {
			if !available_instance_extensions.contains(required) {
				bail!("required Vulkan instance extension is unavailable: {required}");
			}
		}

		let app_name = CString::new(info.application_name.as_str()).context("application_name contains NUL")?;
		let engine_name = CString::new(info.engine_name.as_str()).context("engine_name contains NUL")?;
		let extension_names = strings_to_cstrings(&required_instance_extensions)?;
		let extension_ptrs = extension_names.iter().map(|s| s.as_ptr()).collect::<Vec<_>>();

		let app_info = vk::ApplicationInfo::default()
			.application_name(&app_name)
			.application_version(info.application_version)
			.engine_name(&engine_name)
			.engine_version(info.engine_version)
			.api_version(vk::API_VERSION_1_3);

		let instance_info = vk::InstanceCreateInfo::default()
			.application_info(&app_info)
			.enabled_extension_names(&extension_ptrs);

		let instance =
			unsafe { entry.create_instance(&instance_info, None) }.map_err(|e| anyhow!("vkCreateInstance failed: {e:?}"))?;

		let (surface_loader, surface) = if let Some((display_handle, window_handle)) = window {
			match create_window_surface(&entry, &instance, display_handle, window_handle) {
				Ok(surface) => (Some(ash::khr::surface::Instance::new(&entry, &instance)), Some(surface)),
				Err(e) => {
					unsafe { instance.destroy_instance(None) };
					return Err(e);
				}
			}
		} else {
			(None, None)
		};

		let surface_ref = surface_loader.as_ref().zip(surface);
		let init = (|| -> anyhow::Result<_> {
			let candidates = unsafe { instance.enumerate_physical_devices() }
				.map_err(|e| anyhow!("vkEnumeratePhysicalDevices failed: {e:?}"))?;

			let mut best: Option<DeviceCandidate> = None;
			for physical_device in candidates {
				let runtime_extensions = required_device_extensions_provider
					.as_deref_mut()
					.map_or_else(Vec::new, |provider| provider(physical_device.as_raw()));
				let Some(candidate) = Self::probe_device(&instance, physical_device, info, surface_ref, &runtime_extensions)?
				else {
					continue;
				};

				if best.as_ref().is_none_or(|current| candidate.score > current.score) {
					best = Some(candidate);
				}
			}

			let candidate = best.context("no suitable Vulkan 1.3 physical device found")?;

			let queue_priorities = if candidate.has_capture_queue {
				vec![1.0f32, 1.0f32]
			} else {
				vec![1.0f32]
			};
			let queue_infos = [vk::DeviceQueueCreateInfo::default()
				.queue_family_index(candidate.gfx_family)
				.queue_priorities(&queue_priorities)];

			let surface_format = if let Some((surface_loader, surface)) = surface_ref {
				Some(choose_surface_format(
					surface_loader,
					candidate.physical_device,
					surface,
					info.surface_format,
				)?)
			} else {
				None
			};

			let extension_names = strings_to_cstrings(&candidate.enabled_extensions)?;
			let extension_ptrs = extension_names.iter().map(|s| s.as_ptr()).collect::<Vec<_>>();

			let mut features12 = vk::PhysicalDeviceVulkan12Features::default()
				.descriptor_indexing(true)
				.descriptor_binding_sampled_image_update_after_bind(candidate.caps.descriptor_sampled_image_update_after_bind)
				.descriptor_binding_uniform_buffer_update_after_bind(candidate.caps.descriptor_uniform_buffer_update_after_bind)
				.descriptor_binding_storage_buffer_update_after_bind(
					candidate.caps.descriptor_storage_buffer_update_after_bind,
				);
			let mut features13 = vk::PhysicalDeviceVulkan13Features::default().dynamic_rendering(true);
			let device_info = vk::DeviceCreateInfo::default()
				.queue_create_infos(&queue_infos)
				.enabled_extension_names(&extension_ptrs)
				.push_next(&mut features12)
				.push_next(&mut features13);

			let device = unsafe { instance.create_device(candidate.physical_device, &device_info, None) }
				.map_err(|e| anyhow!("vkCreateDevice failed for {}: {e:?}", candidate.info.name))?;

			let memory_properties = unsafe { instance.get_physical_device_memory_properties(candidate.physical_device) };
			let queue_gfx = RawQueue {
				handle: unsafe { device.get_device_queue(candidate.gfx_family, 0) },
				family_index: candidate.gfx_family,
				queue_index: 0,
			};
			let queue_xfer = RawQueue {
				handle: unsafe { device.get_device_queue(candidate.gfx_family, u32::from(candidate.has_capture_queue)) },
				family_index: candidate.gfx_family,
				queue_index: u32::from(candidate.has_capture_queue),
			};

			Ok((
				candidate,
				device,
				memory_properties,
				queue_gfx,
				queue_xfer,
				surface_format,
			))
		})();

		let (candidate, device, memory_properties, queue_gfx, queue_xfer, surface_format) = match init {
			Ok(v) => v,
			Err(e) => {
				if let (Some(surface_loader), Some(surface)) = (&surface_loader, surface) {
					unsafe { surface_loader.destroy_surface(surface, None) };
				}
				unsafe { instance.destroy_instance(None) };
				return Err(e);
			}
		};

		Ok(Arc::new(Self {
			_entry: entry,
			instance,
			physical_device: candidate.physical_device,
			device,
			memory_properties,
			queue_gfx,
			queue_xfer,
			has_capture_queue: candidate.has_capture_queue,
			surface_loader,
			surface,
			surface_format,
			info: candidate.info,
			caps: candidate.caps,
		}))
	}

	pub(super) unsafe fn new_external_vulkan<FI, FP, FD>(
		info: &WGfxCreateInfo,
		create_instance: FI,
		get_physical_device: FP,
		create_device: FD,
	) -> anyhow::Result<Arc<Self>>
	where
		FI: FnOnce(usize, *const c_void) -> anyhow::Result<u64>,
		FP: FnOnce(u64) -> anyhow::Result<u64>,
		FD: FnOnce(usize, u64, *const c_void) -> anyhow::Result<u64>,
	{
		let entry = unsafe { Entry::load() }.context("loading Vulkan loader")?;

		let available_instance_extensions = unsafe { entry.enumerate_instance_extension_properties(None) }
			.map_err(|e| anyhow!("vkEnumerateInstanceExtensionProperties failed: {e:?}"))?
			.into_iter()
			.map(|p| {
				unsafe { CStr::from_ptr(p.extension_name.as_ptr()) }
					.to_string_lossy()
					.into_owned()
			})
			.collect::<BTreeSet<_>>();
		for required in &info.required_instance_extensions {
			if !available_instance_extensions.contains(required) {
				bail!("required Vulkan instance extension is unavailable: {required}");
			}
		}

		let app_name = CString::new(info.application_name.as_str()).context("application_name contains NUL")?;
		let engine_name = CString::new(info.engine_name.as_str()).context("engine_name contains NUL")?;
		let instance_extension_names = strings_to_cstrings(&info.required_instance_extensions)?;
		let instance_extension_ptrs = instance_extension_names.iter().map(|s| s.as_ptr()).collect::<Vec<_>>();
		let app_info = vk::ApplicationInfo::default()
			.application_name(&app_name)
			.application_version(info.application_version)
			.engine_name(&engine_name)
			.engine_version(info.engine_version)
			.api_version(vk::API_VERSION_1_3);
		let instance_info = vk::InstanceCreateInfo::default()
			.application_info(&app_info)
			.enabled_extension_names(&instance_extension_ptrs);

		let get_instance_proc_addr = entry.static_fn().get_instance_proc_addr as usize;
		let instance_handle = create_instance(
			get_instance_proc_addr,
			std::ptr::from_ref(&instance_info).cast::<c_void>(),
		)?;
		let instance = unsafe { ash::Instance::load(entry.static_fn(), vk::Instance::from_raw(instance_handle)) };

		let init = (|| -> anyhow::Result<_> {
			let physical_device_handle = get_physical_device(instance.handle().as_raw())?;
			let physical_device = vk::PhysicalDevice::from_raw(physical_device_handle);
			let candidate = Self::probe_device(&instance, physical_device, info, None, &[])?
				.context("OpenXR Vulkan physical device is not compatible with WGfx")?;

			let queue_priorities = if candidate.has_capture_queue {
				vec![1.0f32, 1.0f32]
			} else {
				vec![1.0f32]
			};
			let queue_infos = [vk::DeviceQueueCreateInfo::default()
				.queue_family_index(candidate.gfx_family)
				.queue_priorities(&queue_priorities)];

			let extension_names = strings_to_cstrings(&candidate.enabled_extensions)?;
			let extension_ptrs = extension_names.iter().map(|s| s.as_ptr()).collect::<Vec<_>>();
			let mut features12 = vk::PhysicalDeviceVulkan12Features::default()
				.descriptor_indexing(true)
				.descriptor_binding_sampled_image_update_after_bind(candidate.caps.descriptor_sampled_image_update_after_bind)
				.descriptor_binding_uniform_buffer_update_after_bind(candidate.caps.descriptor_uniform_buffer_update_after_bind)
				.descriptor_binding_storage_buffer_update_after_bind(
					candidate.caps.descriptor_storage_buffer_update_after_bind,
				);
			let mut features13 = vk::PhysicalDeviceVulkan13Features::default().dynamic_rendering(true);
			let device_info = vk::DeviceCreateInfo::default()
				.queue_create_infos(&queue_infos)
				.enabled_extension_names(&extension_ptrs)
				.push_next(&mut features12)
				.push_next(&mut features13);

			let device_handle = create_device(
				get_instance_proc_addr,
				physical_device.as_raw(),
				std::ptr::from_ref(&device_info).cast::<c_void>(),
			)?;
			let device = unsafe { ash::Device::load(instance.fp_v1_0(), vk::Device::from_raw(device_handle)) };
			let memory_properties = unsafe { instance.get_physical_device_memory_properties(physical_device) };
			let queue_gfx = RawQueue {
				handle: unsafe { device.get_device_queue(candidate.gfx_family, 0) },
				family_index: candidate.gfx_family,
				queue_index: 0,
			};
			let queue_xfer = RawQueue {
				handle: unsafe { device.get_device_queue(candidate.gfx_family, u32::from(candidate.has_capture_queue)) },
				family_index: candidate.gfx_family,
				queue_index: u32::from(candidate.has_capture_queue),
			};

			Ok((candidate, device, memory_properties, queue_gfx, queue_xfer))
		})();

		let (candidate, device, memory_properties, queue_gfx, queue_xfer) = match init {
			Ok(values) => values,
			Err(error) => {
				unsafe { instance.destroy_instance(None) };
				return Err(error);
			}
		};

		Ok(Arc::new(Self {
			_entry: entry,
			instance,
			physical_device: candidate.physical_device,
			device,
			memory_properties,
			queue_gfx,
			queue_xfer,
			has_capture_queue: candidate.has_capture_queue,
			surface_loader: None,
			surface: None,
			surface_format: None,
			info: candidate.info,
			caps: candidate.caps,
		}))
	}

	#[allow(clippy::similar_names)]
	fn probe_device(
		instance: &ash::Instance,
		physical_device: vk::PhysicalDevice,
		create_info: &WGfxCreateInfo,
		surface: Option<(&ash::khr::surface::Instance, vk::SurfaceKHR)>,
		extra_required_device_extensions: &[String],
	) -> anyhow::Result<Option<DeviceCandidate>> {
		let properties = unsafe { instance.get_physical_device_properties(physical_device) };
		if properties.api_version < vk::API_VERSION_1_3 {
			return Ok(None);
		}

		let mut id = vk::PhysicalDeviceIDProperties::default();
		let mut properties2 = vk::PhysicalDeviceProperties2::default().push_next(&mut id);
		unsafe { instance.get_physical_device_properties2(physical_device, &mut properties2) };

		let mut features12 = vk::PhysicalDeviceVulkan12Features::default();
		let mut features13 = vk::PhysicalDeviceVulkan13Features::default();
		let mut features2 = vk::PhysicalDeviceFeatures2::default()
			.push_next(&mut features12)
			.push_next(&mut features13);
		unsafe { instance.get_physical_device_features2(physical_device, &mut features2) };

		if features13.dynamic_rendering == vk::FALSE
			|| features12.descriptor_indexing == vk::FALSE
			|| features12.descriptor_binding_sampled_image_update_after_bind == vk::FALSE
		{
			return Ok(None);
		}

		let extension_props = unsafe { instance.enumerate_device_extension_properties(physical_device) }
			.map_err(|e| anyhow!("vkEnumerateDeviceExtensionProperties failed: {e:?}"))?;
		let supported_extensions = extension_props
			.iter()
			.map(|p| {
				unsafe { CStr::from_ptr(p.extension_name.as_ptr()) }
					.to_string_lossy()
					.into_owned()
			})
			.collect::<BTreeSet<_>>();

		if create_info
			.required_device_extensions
			.iter()
			.chain(extra_required_device_extensions)
			.any(|required| !supported_extensions.contains(required))
			|| (surface.is_some() && !supported_extensions.contains("VK_KHR_swapchain"))
		{
			return Ok(None);
		}

		let queues = unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
		let gfx_family = queues
			.iter()
			.enumerate()
			.find(|(idx, p)| {
				if p.queue_count == 0 || !p.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
					return false;
				}
				surface.is_none_or(|(loader, surface)| unsafe {
					loader
						.get_physical_device_surface_support(physical_device, *idx as u32, surface)
						.unwrap_or(false)
				})
			})
			.map(|(idx, _)| idx as u32);
		let Some(gfx_family) = gfx_family else {
			return Ok(None);
		};
		let has_capture_queue = queues[gfx_family as usize].queue_count >= 2;

		let mut enabled_extensions = create_info
			.required_device_extensions
			.iter()
			.chain(extra_required_device_extensions)
			.cloned()
			.collect::<BTreeSet<_>>();
		if surface.is_some() {
			enabled_extensions.insert("VK_KHR_swapchain".to_owned());
		}
		for optional in create_info
			.optional_device_extensions
			.iter()
			.map(String::as_str)
			.chain(OPTIONAL_DEVICE_EXTENSIONS.iter().copied())
		{
			if supported_extensions.contains(optional) {
				enabled_extensions.insert(optional.to_owned());
			}
		}

		let drm_render_node = if supported_extensions.contains("VK_EXT_physical_device_drm") {
			let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
			let mut drm_props = vk::PhysicalDeviceProperties2::default().push_next(&mut drm);
			unsafe { instance.get_physical_device_properties2(physical_device, &mut drm_props) };
			(drm.has_render == vk::TRUE).then_some((drm.render_major as u32, drm.render_minor as u32))
		} else {
			None
		};

		let info = DeviceInfo {
			name: unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
				.to_string_lossy()
				.into_owned(),
			vendor_id: properties.vendor_id,
			device_id: properties.device_id,
			device_type: device_type_from_vk(properties.device_type),
			api_version: properties.api_version,
			driver_version: properties.driver_version,
			uuid: id.device_uuid,
			drm_render_node,
		};

		if !matches_preference(&info, &create_info.device_preference) {
			return Ok(None);
		}

		let caps = DeviceCaps {
			external_memory_dma_buf: supported_extensions.contains("VK_EXT_external_memory_dma_buf")
				&& supported_extensions.contains("VK_KHR_external_memory_fd")
				&& supported_extensions.contains("VK_EXT_queue_family_foreign"),
			image_drm_format_modifier: supported_extensions.contains("VK_EXT_image_drm_format_modifier"),
			physical_device_drm: supported_extensions.contains("VK_EXT_physical_device_drm"),
			filter_cubic: supported_extensions.contains("VK_EXT_filter_cubic")
				|| supported_extensions.contains("VK_IMG_filter_cubic"),
			descriptor_sampled_image_update_after_bind: features12.descriptor_binding_sampled_image_update_after_bind
				== vk::TRUE,
			descriptor_uniform_buffer_update_after_bind: features12.descriptor_binding_uniform_buffer_update_after_bind
				== vk::TRUE,
			descriptor_storage_buffer_update_after_bind: features12.descriptor_binding_storage_buffer_update_after_bind
				== vk::TRUE,
			dynamic_rendering: true,
			max_image_dimension_2d: properties.limits.max_image_dimension2_d,
		};

		let score = device_score(info.device_type, has_capture_queue);
		Ok(Some(DeviceCandidate {
			physical_device,
			info,
			caps,
			gfx_family,
			has_capture_queue,
			enabled_extensions: enabled_extensions.into_iter().collect(),
			score,
		}))
	}

	pub(super) const fn info(&self) -> &DeviceInfo {
		&self.info
	}

	pub(super) const fn caps(&self) -> DeviceCaps {
		self.caps
	}

	pub(super) fn surface_format(&self) -> Option<Format> {
		self.surface_format.and_then(|surface| format_from_vk(surface.format))
	}

	pub(super) const fn queue(&self, ty: QueueType) -> RawQueue {
		match ty {
			QueueType::Graphics => self.queue_gfx,
			QueueType::Transfer => self.queue_xfer,
		}
	}

	pub(super) const fn has_capture_queue(&self) -> bool {
		self.has_capture_queue
	}

	pub(super) fn wait_idle(&self) -> anyhow::Result<()> {
		unsafe { self.device.device_wait_idle() }.map_err(|e| anyhow!("vkDeviceWaitIdle failed: {e:?}"))
	}

	pub(super) fn raw_instance_handle(&self) -> u64 {
		self.instance.handle().as_raw()
	}

	pub(super) fn raw_physical_device_handle(&self) -> u64 {
		self.physical_device.as_raw()
	}

	pub(super) fn raw_device_handle(&self) -> u64 {
		self.device.handle().as_raw()
	}

	pub(super) fn raw_graphics_queue_handle(&self) -> u64 {
		self.queue_gfx.handle.as_raw()
	}

	pub(super) const fn graphics_queue_family_index(&self) -> u32 {
		self.queue_gfx.family_index
	}

	pub(super) const fn graphics_queue_index(&self) -> u32 {
		self.queue_gfx.queue_index
	}

	pub(super) const fn raw_format(format: Format) -> i32 {
		format_to_vk(format).as_raw()
	}

	pub(super) fn find_memory_type(
		&self,
		memory_type_bits: u32,
		required: vk::MemoryPropertyFlags,
		preferred: vk::MemoryPropertyFlags,
	) -> Option<u32> {
		let count = self.memory_properties.memory_type_count as usize;

		for idx in 0..count {
			if memory_type_bits & (1 << idx) == 0 {
				continue;
			}
			let flags = self.memory_properties.memory_types[idx].property_flags;
			if flags.contains(required) && flags.contains(preferred) {
				return Some(idx as u32);
			}
		}

		for idx in 0..count {
			if memory_type_bits & (1 << idx) == 0 {
				continue;
			}
			let flags = self.memory_properties.memory_types[idx].property_flags;
			if flags.contains(required) {
				return Some(idx as u32);
			}
		}

		None
	}

	pub(super) fn create_swapchain(
		self: &Arc<Self>,
		requested_extent: [u32; 2],
		old_swapchain: Option<&RawSwapchain>,
	) -> anyhow::Result<(Arc<RawSwapchain>, Vec<Arc<RawImage>>)> {
		let surface_loader = self
			.surface_loader
			.as_ref()
			.context("WGfx was not created for a window")?;
		let surface = self.surface.context("WGfx has no window surface")?;
		let surface_format = self.surface_format.context("window surface has no selected format")?;

		let capabilities =
			unsafe { surface_loader.get_physical_device_surface_capabilities(self.physical_device, surface) }
				.map_err(|e| anyhow!("vkGetPhysicalDeviceSurfaceCapabilitiesKHR failed: {e:?}"))?;

		if !capabilities
			.supported_usage_flags
			.contains(vk::ImageUsageFlags::COLOR_ATTACHMENT)
		{
			bail!("window surface does not support COLOR_ATTACHMENT swapchain images");
		}

		let present_modes =
			unsafe { surface_loader.get_physical_device_surface_present_modes(self.physical_device, surface) }
				.map_err(|e| anyhow!("vkGetPhysicalDeviceSurfacePresentModesKHR failed: {e:?}"))?;
		let present_mode = if present_modes.contains(&vk::PresentModeKHR::MAILBOX) {
			vk::PresentModeKHR::MAILBOX
		} else if present_modes.contains(&vk::PresentModeKHR::FIFO) {
			vk::PresentModeKHR::FIFO
		} else {
			bail!("surface does not support the Vulkan-mandated FIFO present mode");
		};

		let composite_alpha = [
			vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
			vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
			vk::CompositeAlphaFlagsKHR::OPAQUE,
		]
		.into_iter()
		.find(|mode| capabilities.supported_composite_alpha.contains(*mode))
		.context("surface reports no usable composite alpha mode")?;

		let extent = if capabilities.current_extent.width == u32::MAX {
			vk::Extent2D {
				width: requested_extent[0].clamp(capabilities.min_image_extent.width, capabilities.max_image_extent.width),
				height: requested_extent[1].clamp(
					capabilities.min_image_extent.height,
					capabilities.max_image_extent.height,
				),
			}
		} else {
			capabilities.current_extent
		};

		let mut image_count = capabilities.min_image_count.max(2);
		if capabilities.max_image_count != 0 {
			image_count = image_count.min(capabilities.max_image_count);
		}

		let loader = ash::khr::swapchain::Device::new(&self.instance, &self.device);
		let create_info = vk::SwapchainCreateInfoKHR::default()
			.surface(surface)
			.min_image_count(image_count)
			.image_format(surface_format.format)
			.image_color_space(surface_format.color_space)
			.image_extent(extent)
			.image_array_layers(1)
			.image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
			.image_sharing_mode(vk::SharingMode::EXCLUSIVE)
			.pre_transform(capabilities.current_transform)
			.composite_alpha(composite_alpha)
			.present_mode(present_mode)
			.clipped(true)
			.old_swapchain(old_swapchain.map_or(vk::SwapchainKHR::null(), |swapchain| swapchain.owner.handle));

		let handle = unsafe { loader.create_swapchain(&create_info, None) }
			.map_err(|e| anyhow!("vkCreateSwapchainKHR failed: {e:?}"))?;
		let owner = Arc::new(RawSwapchainOwner {
			context: self.clone(),
			loader,
			handle,
		});
		let handles = match unsafe { owner.loader.get_swapchain_images(handle) } {
			Ok(images) => images,
			Err(e) => {
				drop(owner);
				return Err(anyhow!("vkGetSwapchainImagesKHR failed: {e:?}"));
			}
		};

		let Some(format) = format_from_vk(surface_format.format) else {
			drop(owner);
			bail!(
				"selected swapchain format is not representable by WGfx: {:?}",
				surface_format.format
			);
		};

		let raw = Arc::new(RawSwapchain {
			owner: owner.clone(),
			extent: [extent.width, extent.height],
			format,
		});
		let images = handles
			.into_iter()
			.map(|handle| {
				Arc::new(RawImage {
					context: self.clone(),
					handle,
					memory: None,
					owns_image: false,
					swapchain_owner: Some(owner.clone()),
					layout: Mutex::new(vk::ImageLayout::UNDEFINED),
					external_dma_buf: false,
				})
			})
			.collect();

		Ok((raw, images))
	}

	pub(super) fn create_buffer(self: &Arc<Self>, byte_len: usize, usage: BufferUsage) -> anyhow::Result<Arc<RawBuffer>> {
		let byte_len = byte_len.max(1) as vk::DeviceSize;
		let create_info = vk::BufferCreateInfo::default()
			.size(byte_len)
			.usage(buffer_usage_to_vk(usage))
			.sharing_mode(vk::SharingMode::EXCLUSIVE);
		let handle =
			unsafe { self.device.create_buffer(&create_info, None) }.map_err(|e| anyhow!("vkCreateBuffer failed: {e:?}"))?;
		let requirements = unsafe { self.device.get_buffer_memory_requirements(handle) };

		let required = vk::MemoryPropertyFlags::HOST_VISIBLE;
		let preferred = vk::MemoryPropertyFlags::HOST_COHERENT | vk::MemoryPropertyFlags::DEVICE_LOCAL;
		let Some(memory_type_index) = self.find_memory_type(requirements.memory_type_bits, required, preferred) else {
			unsafe { self.device.destroy_buffer(handle, None) };
			bail!("no HOST_VISIBLE Vulkan memory type for buffer");
		};

		let flags = self.memory_properties.memory_types[memory_type_index as usize].property_flags;
		let coherent = flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT);
		let alloc_info = vk::MemoryAllocateInfo::default()
			.allocation_size(requirements.size)
			.memory_type_index(memory_type_index);
		let memory = match unsafe { self.device.allocate_memory(&alloc_info, None) } {
			Ok(memory) => memory,
			Err(e) => {
				unsafe { self.device.destroy_buffer(handle, None) };
				return Err(anyhow!("vkAllocateMemory for buffer failed: {e:?}"));
			}
		};

		if let Err(e) = unsafe { self.device.bind_buffer_memory(handle, memory, 0) } {
			unsafe {
				self.device.free_memory(memory, None);
				self.device.destroy_buffer(handle, None);
			}
			return Err(anyhow!("vkBindBufferMemory failed: {e:?}"));
		}

		let mapped = match unsafe {
			self
				.device
				.map_memory(memory, 0, requirements.size, vk::MemoryMapFlags::empty())
		} {
			Ok(ptr) => {
				if let Some(ptr) = NonNull::new(ptr.cast::<u8>()) {
					ptr
				} else {
					unsafe {
						self.device.free_memory(memory, None);
						self.device.destroy_buffer(handle, None);
					}
					bail!("vkMapMemory returned null");
				}
			}
			Err(e) => {
				unsafe {
					self.device.free_memory(memory, None);
					self.device.destroy_buffer(handle, None);
				}
				return Err(anyhow!("vkMapMemory failed: {e:?}"));
			}
		};

		Ok(Arc::new(RawBuffer {
			context: self.clone(),
			handle,
			memory,
			mapped,
			coherent,
			write_lock: Mutex::new(()),
		}))
	}

	pub(super) fn create_image(self: &Arc<Self>, info: ImageCreateInfo) -> anyhow::Result<Arc<RawImage>> {
		if info.tiling == ImageTiling::DrmFormatModifier {
			bail!("DRM modifier images must be created through WGfx's DMA-BUF import path");
		}

		let used_on_transfer_queue =
			info.usage.contains(ImageUsage::TRANSFER_SRC) || info.usage.contains(ImageUsage::TRANSFER_DST);
		let used_on_graphics_queue =
			info.usage.contains(ImageUsage::SAMPLED) || info.usage.contains(ImageUsage::COLOR_ATTACHMENT);
		let queue_family_indices = [self.queue_gfx.family_index, self.queue_xfer.family_index];
		let use_concurrent_sharing =
			self.queue_gfx.family_index != self.queue_xfer.family_index && used_on_transfer_queue && used_on_graphics_queue;

		let mut image_info = vk::ImageCreateInfo::default()
			.image_type(vk::ImageType::TYPE_2D)
			.format(format_to_vk(info.format))
			.extent(vk::Extent3D {
				width: info.extent[0],
				height: info.extent[1],
				depth: info.extent[2],
			})
			.mip_levels(1)
			.array_layers(info.array_layers)
			.samples(vk::SampleCountFlags::TYPE_1)
			.tiling(image_tiling_to_vk(info.tiling))
			.usage(image_usage_to_vk(info.usage))
			.initial_layout(vk::ImageLayout::UNDEFINED);
		image_info = if use_concurrent_sharing {
			image_info
				.sharing_mode(vk::SharingMode::CONCURRENT)
				.queue_family_indices(&queue_family_indices)
		} else {
			image_info.sharing_mode(vk::SharingMode::EXCLUSIVE)
		};

		let handle =
			unsafe { self.device.create_image(&image_info, None) }.map_err(|e| anyhow!("vkCreateImage failed: {e:?}"))?;
		let requirements = unsafe { self.device.get_image_memory_requirements(handle) };
		let memory_type = self
			.find_memory_type(
				requirements.memory_type_bits,
				vk::MemoryPropertyFlags::empty(),
				vk::MemoryPropertyFlags::DEVICE_LOCAL,
			)
			.context("no compatible Vulkan memory type for image")?;
		let alloc_info = vk::MemoryAllocateInfo::default()
			.allocation_size(requirements.size)
			.memory_type_index(memory_type);
		let memory = match unsafe { self.device.allocate_memory(&alloc_info, None) } {
			Ok(memory) => memory,
			Err(e) => {
				unsafe { self.device.destroy_image(handle, None) };
				return Err(anyhow!("vkAllocateMemory for image failed: {e:?}"));
			}
		};

		if let Err(e) = unsafe { self.device.bind_image_memory(handle, memory, 0) } {
			unsafe {
				self.device.free_memory(memory, None);
				self.device.destroy_image(handle, None);
			}
			return Err(anyhow!("vkBindImageMemory failed: {e:?}"));
		}

		Ok(Arc::new(RawImage {
			context: self.clone(),
			handle,
			memory: Some(memory),
			owns_image: true,
			swapchain_owner: None,
			layout: Mutex::new(vk::ImageLayout::UNDEFINED),
			external_dma_buf: false,
		}))
	}

	fn query_dmabuf_image_support(&self, format: Format, modifier: u64) -> anyhow::Result<bool> {
		if !self.caps.external_memory_dma_buf || !self.caps.image_drm_format_modifier {
			return Ok(false);
		}

		let mut drm_info = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT {
			drm_format_modifier: modifier,
			sharing_mode: vk::SharingMode::EXCLUSIVE,
			..Default::default()
		};
		let mut external_info = vk::PhysicalDeviceExternalImageFormatInfo {
			handle_type: vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
			..Default::default()
		};
		let image_info = vk::PhysicalDeviceImageFormatInfo2::default()
			.format(format_to_vk(format))
			.ty(vk::ImageType::TYPE_2D)
			.tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
			.usage(vk::ImageUsageFlags::SAMPLED)
			.flags(vk::ImageCreateFlags::empty())
			.push_next(&mut drm_info)
			.push_next(&mut external_info);
		let mut external_props = vk::ExternalImageFormatProperties::default();
		let query_result = {
			let mut props = vk::ImageFormatProperties2::default().push_next(&mut external_props);
			unsafe {
				self
					.instance
					.get_physical_device_image_format_properties2(self.physical_device, &image_info, &mut props)
			}
		};

		match query_result {
			Ok(()) => Ok(
				external_props
					.external_memory_properties
					.external_memory_features
					.contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE),
			),
			Err(vk::Result::ERROR_FORMAT_NOT_SUPPORTED) => Ok(false),
			Err(e) => Err(anyhow!(
				"vkGetPhysicalDeviceImageFormatProperties2 failed for DMA-buf modifier {modifier:#x}: {e:?}"
			)),
		}
	}

	pub(super) fn dmabuf_import_modifiers(&self, format: Format) -> anyhow::Result<Vec<u64>> {
		if !self.caps.external_memory_dma_buf || !self.caps.image_drm_format_modifier {
			return Ok(Vec::new());
		}

		let vk_format = format_to_vk(format);
		let mut modifier_list = vk::DrmFormatModifierPropertiesListEXT::default();
		{
			let mut properties = vk::FormatProperties2::default().push_next(&mut modifier_list);
			unsafe {
				self
					.instance
					.get_physical_device_format_properties2(self.physical_device, vk_format, &mut properties);
			}
		}

		let mut modifier_properties =
			vec![vk::DrmFormatModifierPropertiesEXT::default(); modifier_list.drm_format_modifier_count as usize];
		modifier_list.p_drm_format_modifier_properties = modifier_properties.as_mut_ptr();
		{
			let mut properties = vk::FormatProperties2::default().push_next(&mut modifier_list);
			unsafe {
				self
					.instance
					.get_physical_device_format_properties2(self.physical_device, vk_format, &mut properties);
			}
		}
		modifier_properties.truncate(modifier_list.drm_format_modifier_count as usize);

		let mut modifiers = Vec::new();
		for prop in modifier_properties {
			if prop.drm_format_modifier_plane_count != 1
				|| !prop
					.drm_format_modifier_tiling_features
					.contains(vk::FormatFeatureFlags::SAMPLED_IMAGE)
			{
				continue;
			}
			if self.query_dmabuf_image_support(format, prop.drm_format_modifier)? {
				modifiers.push(prop.drm_format_modifier);
			}
		}
		Ok(modifiers)
	}

	pub(super) fn import_dmabuf(self: &Arc<Self>, info: &DmabufImportInfo) -> anyhow::Result<Arc<RawImage>> {
		if !self.caps.external_memory_dma_buf {
			bail!(
				"DMA-buf import requires VK_EXT_external_memory_dma_buf, VK_KHR_external_memory_fd, and VK_EXT_queue_family_foreign"
			);
		}
		if info.modifier.is_some() && !self.caps.image_drm_format_modifier {
			bail!("explicit DMA-buf modifiers require VK_EXT_image_drm_format_modifier");
		}
		if let Some(modifier) = info.modifier
			&& !self.query_dmabuf_image_support(info.format, modifier)?
		{
			bail!(
				"Vulkan reports DMA-buf import unsupported for format {:?}, modifier {modifier:#x}",
				info.format
			);
		}

		let plane_layouts = info
			.plane_layouts
			.iter()
			.map(|layout| vk::SubresourceLayout {
				offset: layout.offset,
				size: 0,
				row_pitch: layout.row_pitch,
				array_pitch: 0,
				depth_pitch: 0,
			})
			.collect::<Vec<_>>();

		let mut external_image = vk::ExternalMemoryImageCreateInfo {
			handle_types: vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
			..Default::default()
		};
		let mut explicit_modifier = info
			.modifier
			.map(|modifier| vk::ImageDrmFormatModifierExplicitCreateInfoEXT {
				drm_format_modifier: modifier,
				drm_format_modifier_plane_count: plane_layouts.len() as u32,
				p_plane_layouts: plane_layouts.as_ptr(),
				..Default::default()
			});
		let mut image_info = vk::ImageCreateInfo::default()
			.image_type(vk::ImageType::TYPE_2D)
			.format(format_to_vk(info.format))
			.extent(vk::Extent3D {
				width: info.extent[0],
				height: info.extent[1],
				depth: info.extent[2],
			})
			.mip_levels(1)
			.array_layers(1)
			.samples(vk::SampleCountFlags::TYPE_1)
			.tiling(if info.modifier.is_some() {
				vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT
			} else {
				vk::ImageTiling::OPTIMAL
			})
			.usage(vk::ImageUsageFlags::SAMPLED)
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.initial_layout(vk::ImageLayout::UNDEFINED);

		external_image.p_next = image_info.p_next;
		image_info.p_next = std::ptr::from_ref(&external_image).cast();
		if let Some(explicit_modifier) = explicit_modifier.as_mut() {
			explicit_modifier.p_next = image_info.p_next;
			image_info.p_next = std::ptr::from_ref(explicit_modifier).cast();
		}

		let handle = unsafe { self.device.create_image(&image_info, None) }
			.map_err(|e| anyhow!("vkCreateImage for DMA-buf import failed: {e:?}"))?;
		let requirements = unsafe { self.device.get_image_memory_requirements(handle) };

		let fd_loader = ash::khr::external_memory_fd::Device::new(&self.instance, &self.device);
		let mut fd_properties = vk::MemoryFdPropertiesKHR::default();
		if let Err(e) = unsafe {
			fd_loader.get_memory_fd_properties(
				vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
				info.fd,
				&mut fd_properties,
			)
		} {
			unsafe { self.device.destroy_image(handle, None) };
			return Err(anyhow!("vkGetMemoryFdPropertiesKHR failed: {e:?}"));
		}

		let memory_type_bits = requirements.memory_type_bits & fd_properties.memory_type_bits;
		if memory_type_bits == 0 {
			unsafe { self.device.destroy_image(handle, None) };
			bail!(
				"DMA-buf has no memory type compatible with the Vulkan image (image bits: {:#x}, fd bits: {:#x})",
				requirements.memory_type_bits,
				fd_properties.memory_type_bits
			);
		}
		let Some(memory_type_index) = self.find_memory_type(
			memory_type_bits,
			vk::MemoryPropertyFlags::empty(),
			vk::MemoryPropertyFlags::DEVICE_LOCAL,
		) else {
			unsafe { self.device.destroy_image(handle, None) };
			bail!("failed to get a compatible DMA-buf memory type index");
		};

		let original = ManuallyDrop::new(unsafe { File::from_raw_fd(info.fd) });
		let duplicate = match original.try_clone() {
			Ok(file) => file,
			Err(e) => {
				unsafe { self.device.destroy_image(handle, None) };
				return Err(e).context("duplicating DMA-buf file descriptor");
			}
		};
		let mut import_info = vk::ImportMemoryFdInfoKHR {
			handle_type: vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
			fd: duplicate.as_raw_fd(),
			..Default::default()
		};
		let mut dedicated = vk::MemoryDedicatedAllocateInfo {
			image: handle,
			..Default::default()
		};
		let alloc_info = vk::MemoryAllocateInfo::default()
			.allocation_size(requirements.size)
			.memory_type_index(memory_type_index)
			.push_next(&mut dedicated)
			.push_next(&mut import_info);
		let memory = match unsafe { self.device.allocate_memory(&alloc_info, None) } {
			Ok(memory) => {
				std::mem::forget(duplicate);
				memory
			}
			Err(e) => {
				unsafe { self.device.destroy_image(handle, None) };
				return Err(anyhow!("vkAllocateMemory for DMA-buf import failed: {e:?}"));
			}
		};

		if let Err(e) = unsafe { self.device.bind_image_memory(handle, memory, 0) } {
			unsafe {
				self.device.free_memory(memory, None);
				self.device.destroy_image(handle, None);
			}
			return Err(anyhow!("vkBindImageMemory for DMA-buf import failed: {e:?}"));
		}

		Ok(Arc::new(RawImage {
			context: self.clone(),
			handle,
			memory: Some(memory),
			owns_image: true,
			swapchain_owner: None,
			layout: Mutex::new(vk::ImageLayout::GENERAL),
			external_dma_buf: true,
		}))
	}

	pub(super) fn export_dmabuf_image(
		self: &Arc<Self>,
		extent: [u32; 3],
		format: Format,
		modifier: u64,
	) -> anyhow::Result<(Arc<RawImage>, File, u64, u64)> {
		if !self.caps.external_memory_dma_buf || !self.caps.image_drm_format_modifier {
			bail!("DMA-buf export requires external-memory FD, DRM-format-modifier, and foreign queue-family support");
		}

		let modifiers = [modifier];
		let mut modifier_info = vk::ImageDrmFormatModifierListCreateInfoEXT {
			drm_format_modifier_count: 1,
			p_drm_format_modifiers: modifiers.as_ptr(),
			..Default::default()
		};
		let mut external_info = vk::ExternalMemoryImageCreateInfo {
			handle_types: vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
			..Default::default()
		};
		let image_info = vk::ImageCreateInfo::default()
			.image_type(vk::ImageType::TYPE_2D)
			.format(format_to_vk(format))
			.extent(vk::Extent3D {
				width: extent[0],
				height: extent[1],
				depth: extent[2],
			})
			.mip_levels(1)
			.array_layers(1)
			.samples(vk::SampleCountFlags::TYPE_1)
			.tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
			.usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::SAMPLED)
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.initial_layout(vk::ImageLayout::UNDEFINED)
			.push_next(&mut modifier_info)
			.push_next(&mut external_info);
		let handle = unsafe { self.device.create_image(&image_info, None) }
			.map_err(|e| anyhow!("vkCreateImage for DMA-buf export failed: {e:?}"))?;
		let requirements = unsafe { self.device.get_image_memory_requirements(handle) };
		let Some(memory_type_index) = self.find_memory_type(
			requirements.memory_type_bits,
			vk::MemoryPropertyFlags::empty(),
			vk::MemoryPropertyFlags::DEVICE_LOCAL,
		) else {
			unsafe { self.device.destroy_image(handle, None) };
			bail!("no compatible Vulkan memory type for DMA-buf export image");
		};

		let mut export_info = vk::ExportMemoryAllocateInfo {
			handle_types: vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
			..Default::default()
		};
		let mut dedicated = vk::MemoryDedicatedAllocateInfo {
			image: handle,
			..Default::default()
		};
		let alloc_info = vk::MemoryAllocateInfo::default()
			.allocation_size(requirements.size)
			.memory_type_index(memory_type_index)
			.push_next(&mut dedicated)
			.push_next(&mut export_info);
		let memory = match unsafe { self.device.allocate_memory(&alloc_info, None) } {
			Ok(memory) => memory,
			Err(e) => {
				unsafe { self.device.destroy_image(handle, None) };
				return Err(anyhow!("vkAllocateMemory for DMA-buf export failed: {e:?}"));
			}
		};
		if let Err(e) = unsafe { self.device.bind_image_memory(handle, memory, 0) } {
			unsafe {
				self.device.free_memory(memory, None);
				self.device.destroy_image(handle, None);
			}
			return Err(anyhow!("vkBindImageMemory for DMA-buf export failed: {e:?}"));
		}

		let fd_loader = ash::khr::external_memory_fd::Device::new(&self.instance, &self.device);
		let fd_info = vk::MemoryGetFdInfoKHR {
			memory,
			handle_type: vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
			..Default::default()
		};
		let fd = match unsafe { fd_loader.get_memory_fd(&fd_info) } {
			Ok(fd) => unsafe { File::from_raw_fd(fd) },
			Err(e) => {
				unsafe {
					self.device.destroy_image(handle, None);
					self.device.free_memory(memory, None);
				}
				return Err(anyhow!("vkGetMemoryFdKHR failed: {e:?}"));
			}
		};
		let layout = unsafe {
			self.device.get_image_subresource_layout(
				handle,
				vk::ImageSubresource {
					aspect_mask: vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
					mip_level: 0,
					array_layer: 0,
				},
			)
		};

		Ok((
			Arc::new(RawImage {
				context: self.clone(),
				handle,
				memory: Some(memory),
				owns_image: true,
				swapchain_owner: None,
				layout: Mutex::new(vk::ImageLayout::GENERAL),
				external_dma_buf: true,
			}),
			fd,
			layout.offset,
			layout.row_pitch,
		))
	}

	pub(super) fn wrap_external_image(
		self: &Arc<Self>,
		raw_handle: u64,
		initial_layout: Option<ImageLayout>,
	) -> Arc<RawImage> {
		Arc::new(RawImage {
			context: self.clone(),
			handle: vk::Image::from_raw(raw_handle),
			memory: None,
			owns_image: false,
			swapchain_owner: None,
			layout: Mutex::new(initial_layout.map_or(vk::ImageLayout::UNDEFINED, image_layout_to_vk)),
			external_dma_buf: false,
		})
	}

	pub(super) fn create_image_view(
		self: &Arc<Self>,
		image: Arc<RawImage>,
		format: Format,
		layers: u32,
		info: ImageViewCreateInfo,
	) -> anyhow::Result<Arc<RawImageView>> {
		let create_info = vk::ImageViewCreateInfo::default()
			.image(image.handle)
			.view_type(if layers > 1 {
				vk::ImageViewType::TYPE_2D_ARRAY
			} else {
				vk::ImageViewType::TYPE_2D
			})
			.format(format_to_vk(format))
			.components(vk::ComponentMapping {
				r: vk::ComponentSwizzle::IDENTITY,
				g: vk::ComponentSwizzle::IDENTITY,
				b: vk::ComponentSwizzle::IDENTITY,
				a: vk::ComponentSwizzle::IDENTITY,
			})
			.subresource_range(vk::ImageSubresourceRange {
				aspect_mask: vk::ImageAspectFlags::COLOR,
				base_mip_level: info.base_mip_level,
				level_count: info.mip_level_count,
				base_array_layer: info.base_array_layer,
				layer_count: info.array_layer_count,
			});
		let handle = unsafe { self.device.create_image_view(&create_info, None) }
			.map_err(|e| anyhow!("vkCreateImageView failed: {e:?}"))?;
		Ok(Arc::new(RawImageView {
			context: self.clone(),
			image,
			handle,
		}))
	}

	pub(super) fn create_sampler(self: &Arc<Self>, info: SamplerCreateInfo) -> anyhow::Result<Arc<RawSampler>> {
		if (matches!(info.mag_filter, Filter::Cubic) || matches!(info.min_filter, Filter::Cubic)) && !self.caps.filter_cubic
		{
			bail!("cubic filtering requested but neither VK_EXT_filter_cubic nor VK_IMG_filter_cubic is available");
		}

		let create_info = vk::SamplerCreateInfo::default()
			.mag_filter(filter_to_vk(info.mag_filter))
			.min_filter(filter_to_vk(info.min_filter))
			.mipmap_mode(vk::SamplerMipmapMode::LINEAR)
			.address_mode_u(address_mode_to_vk(info.address_mode[0]))
			.address_mode_v(address_mode_to_vk(info.address_mode[1]))
			.address_mode_w(address_mode_to_vk(info.address_mode[2]))
			.min_lod(0.0)
			.max_lod(0.0);
		let handle = unsafe { self.device.create_sampler(&create_info, None) }
			.map_err(|e| anyhow!("vkCreateSampler failed: {e:?}"))?;
		Ok(Arc::new(RawSampler {
			context: self.clone(),
			handle,
		}))
	}

	pub(super) fn create_shader_module(
		self: &Arc<Self>,
		words: &[u32],
		stage: ShaderStage,
	) -> anyhow::Result<Arc<RawShaderModule>> {
		let create_info = vk::ShaderModuleCreateInfo::default().code(words);
		let handle = unsafe { self.device.create_shader_module(&create_info, None) }
			.map_err(|e| anyhow!("vkCreateShaderModule failed: {e:?}"))?;
		Ok(Arc::new(RawShaderModule {
			context: self.clone(),
			handle,
			stage,
		}))
	}

	pub(super) fn create_pipeline(
		self: &Arc<Self>,
		vert: &RawShaderModule,
		frag: &RawShaderModule,
		spec: &PipelineSpec,
	) -> anyhow::Result<Arc<RawPipeline>> {
		if vert.stage != ShaderStage::Vertex || frag.stage != ShaderStage::Fragment {
			bail!("graphics pipeline requires one vertex shader and one fragment shader");
		}

		let set_layouts = self.create_descriptor_set_layouts(&spec.descriptor_bindings, &spec.updatable_sets)?;
		let set_layout_handles = set_layouts.iter().map(|l| l.handle).collect::<Vec<_>>();
		let layout_info = vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layout_handles);
		let layout = unsafe { self.device.create_pipeline_layout(&layout_info, None) }
			.map_err(|e| anyhow!("vkCreatePipelineLayout failed: {e:?}"))?;

		let entry_name = c"main";
		let stages = [
			vk::PipelineShaderStageCreateInfo::default()
				.stage(vk::ShaderStageFlags::VERTEX)
				.module(vert.handle)
				.name(entry_name),
			vk::PipelineShaderStageCreateInfo::default()
				.stage(vk::ShaderStageFlags::FRAGMENT)
				.module(frag.handle)
				.name(entry_name),
		];

		let binding = [vk::VertexInputBindingDescription {
			binding: 0,
			stride: spec.vertex_stride,
			input_rate: if spec.instanced {
				vk::VertexInputRate::INSTANCE
			} else {
				vk::VertexInputRate::VERTEX
			},
		}];
		let attributes = spec
			.vertex_attributes
			.iter()
			.map(|a| vk::VertexInputAttributeDescription {
				location: a.location,
				binding: 0,
				format: vertex_format_to_vk(a.format),
				offset: a.offset,
			})
			.collect::<Vec<_>>();
		let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
			.vertex_binding_descriptions(&binding)
			.vertex_attribute_descriptions(&attributes);
		let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default().topology(topology_to_vk(spec.topology));
		let viewport_state = vk::PipelineViewportStateCreateInfo::default()
			.viewport_count(1)
			.scissor_count(1);
		let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
			.polygon_mode(vk::PolygonMode::FILL)
			.cull_mode(vk::CullModeFlags::NONE)
			.front_face(vk::FrontFace::COUNTER_CLOCKWISE)
			.line_width(1.0);
		let multisample =
			vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(vk::SampleCountFlags::TYPE_1);
		let color_attachment = spec.blend.map_or_else(
			|| vk::PipelineColorBlendAttachmentState {
				blend_enable: vk::FALSE,
				src_color_blend_factor: vk::BlendFactor::ONE,
				dst_color_blend_factor: vk::BlendFactor::ZERO,
				color_blend_op: vk::BlendOp::ADD,
				src_alpha_blend_factor: vk::BlendFactor::ONE,
				dst_alpha_blend_factor: vk::BlendFactor::ZERO,
				alpha_blend_op: vk::BlendOp::ADD,
				color_write_mask: vk::ColorComponentFlags::R
					| vk::ColorComponentFlags::G
					| vk::ColorComponentFlags::B
					| vk::ColorComponentFlags::A,
			},
			blend_to_vk,
		);
		let color_attachments = [color_attachment];
		let color_blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&color_attachments);
		let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
		let dynamic_state = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
		let color_formats = [format_to_vk(spec.format)];
		let mut rendering = vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&color_formats);
		let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
			.stages(&stages)
			.vertex_input_state(&vertex_input)
			.input_assembly_state(&input_assembly)
			.viewport_state(&viewport_state)
			.rasterization_state(&rasterization)
			.multisample_state(&multisample)
			.color_blend_state(&color_blend)
			.dynamic_state(&dynamic_state)
			.layout(layout)
			.push_next(&mut rendering);

		let handle = match unsafe {
			self
				.device
				.create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
		} {
			Ok(mut pipelines) => pipelines.remove(0),
			Err((pipelines, e)) => {
				for pipeline in pipelines {
					unsafe { self.device.destroy_pipeline(pipeline, None) };
				}
				unsafe { self.device.destroy_pipeline_layout(layout, None) };
				return Err(anyhow!("vkCreateGraphicsPipelines failed: {e:?}"));
			}
		};

		Ok(Arc::new(RawPipeline {
			context: self.clone(),
			handle,
			layout,
			set_layouts,
		}))
	}

	fn create_descriptor_set_layouts(
		self: &Arc<Self>,
		bindings: &[DescriptorBinding],
		updatable_sets: &[usize],
	) -> anyhow::Result<Vec<Arc<RawDescriptorSetLayout>>> {
		if bindings.is_empty() {
			if !updatable_sets.is_empty() {
				bail!("pipeline requests update-after-bind sets but has no descriptors");
			}
			return Ok(Vec::new());
		}
		let max_set = bindings.iter().map(|b| b.set).max().unwrap_or(0);
		let mut out = Vec::with_capacity(max_set as usize + 1);

		for set in 0..=max_set {
			let logical = bindings.iter().filter(|b| b.set == set).copied().collect::<Vec<_>>();
			let update_after_bind = updatable_sets.contains(&(set as usize));
			if update_after_bind {
				for binding in &logical {
					let supported = match binding.descriptor_type {
						DescriptorType::CombinedImageSampler => self.caps.descriptor_sampled_image_update_after_bind,
						DescriptorType::UniformBuffer => self.caps.descriptor_uniform_buffer_update_after_bind,
						DescriptorType::StorageBuffer => self.caps.descriptor_storage_buffer_update_after_bind,
					};
					if !supported {
						bail!(
							"descriptor set {set} binding {} requests update-after-bind for unsupported {:?}",
							binding.binding,
							binding.descriptor_type
						);
					}
				}
			}
			let raw_bindings = logical
				.iter()
				.map(|b| {
					vk::DescriptorSetLayoutBinding::default()
						.binding(b.binding)
						.descriptor_type(descriptor_type_to_vk(b.descriptor_type))
						.descriptor_count(b.descriptor_count)
						.stage_flags(shader_stages_to_vk(b.stages))
				})
				.collect::<Vec<_>>();
			let mut binding_flags = vec![vk::DescriptorBindingFlags::empty(); raw_bindings.len()];
			if update_after_bind {
				binding_flags.fill(vk::DescriptorBindingFlags::UPDATE_AFTER_BIND);
			}
			let mut flags_info = vk::DescriptorSetLayoutBindingFlagsCreateInfo::default().binding_flags(&binding_flags);
			let mut create_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&raw_bindings);
			if update_after_bind {
				create_info = create_info
					.flags(vk::DescriptorSetLayoutCreateFlags::UPDATE_AFTER_BIND_POOL)
					.push_next(&mut flags_info);
			}
			let handle = unsafe { self.device.create_descriptor_set_layout(&create_info, None) }
				.map_err(|e| anyhow!("vkCreateDescriptorSetLayout failed for set {set}: {e:?}"))?;
			out.push(Arc::new(RawDescriptorSetLayout {
				context: self.clone(),
				handle,
				bindings: logical,
				update_after_bind,
			}));
		}

		Ok(out)
	}

	pub(super) fn create_descriptor_set(
		self: &Arc<Self>,
		layout: Arc<RawDescriptorSetLayout>,
	) -> anyhow::Result<Arc<RawDescriptorSet>> {
		let mut counts = BTreeMap::<DescriptorType, u32>::new();
		for binding in &layout.bindings {
			*counts.entry(binding.descriptor_type).or_default() += binding.descriptor_count;
		}
		let pool_sizes = counts
			.into_iter()
			.map(|(ty, descriptor_count)| vk::DescriptorPoolSize {
				ty: descriptor_type_to_vk(ty),
				descriptor_count,
			})
			.collect::<Vec<_>>();
		let flags = if layout.update_after_bind {
			vk::DescriptorPoolCreateFlags::UPDATE_AFTER_BIND
		} else {
			vk::DescriptorPoolCreateFlags::empty()
		};
		let pool_info = vk::DescriptorPoolCreateInfo::default()
			.flags(flags)
			.max_sets(1)
			.pool_sizes(&pool_sizes);
		let pool = unsafe { self.device.create_descriptor_pool(&pool_info, None) }
			.map_err(|e| anyhow!("vkCreateDescriptorPool failed: {e:?}"))?;
		let layouts = [layout.handle];
		let alloc_info = vk::DescriptorSetAllocateInfo::default()
			.descriptor_pool(pool)
			.set_layouts(&layouts);
		let handle = match unsafe { self.device.allocate_descriptor_sets(&alloc_info) } {
			Ok(mut sets) => sets.remove(0),
			Err(e) => {
				unsafe { self.device.destroy_descriptor_pool(pool, None) };
				return Err(anyhow!("vkAllocateDescriptorSets failed: {e:?}"));
			}
		};
		Ok(Arc::new(RawDescriptorSet {
			context: self.clone(),
			layout,
			pool,
			handle,
			sampled_images: Mutex::new(BTreeMap::new()),
		}))
	}

	pub(super) fn update_descriptor_buffer(
		&self,
		set: &RawDescriptorSet,
		binding: u32,
		buffer: &RawBuffer,
	) -> anyhow::Result<()> {
		let descriptor_type = set.binding_type(binding)?;
		if !matches!(
			descriptor_type,
			DescriptorType::UniformBuffer | DescriptorType::StorageBuffer
		) {
			bail!("descriptor binding {binding} is not a buffer binding");
		}
		let buffer_info = [vk::DescriptorBufferInfo::default()
			.buffer(buffer.handle)
			.offset(0)
			.range(vk::WHOLE_SIZE)];
		let writes = [vk::WriteDescriptorSet::default()
			.dst_set(set.handle)
			.dst_binding(binding)
			.descriptor_type(descriptor_type_to_vk(descriptor_type))
			.buffer_info(&buffer_info)];
		unsafe { self.device.update_descriptor_sets(&writes, &[]) };
		Ok(())
	}

	pub(super) fn update_descriptor_image_sampler(
		&self,
		set: &RawDescriptorSet,
		binding: u32,
		view: &RawImageView,
		sampler: &RawSampler,
	) -> anyhow::Result<()> {
		let descriptor_type = set.binding_type(binding)?;
		if descriptor_type != DescriptorType::CombinedImageSampler {
			bail!("descriptor binding {binding} is not a combined image sampler");
		}
		let image_info = [vk::DescriptorImageInfo::default()
			.sampler(sampler.handle)
			.image_view(view.handle)
			.image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
		let writes = [vk::WriteDescriptorSet::default()
			.dst_set(set.handle)
			.dst_binding(binding)
			.descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
			.image_info(&image_info)];
		unsafe { self.device.update_descriptor_sets(&writes, &[]) };
		set.sampled_images.lock().insert(binding, view.image.clone());
		Ok(())
	}

	pub(super) fn submit_graphics_completion_marker(self: &Arc<Self>) -> anyhow::Result<RawQueueCompletionMarker> {
		let mut command_buffer = self.create_command_buffer(QueueType::Graphics, CommandBufferUsage::OneTimeSubmit)?;
		command_buffer.record_queue_completion_barrier();
		command_buffer.finish()?;

		let fence = unsafe { self.device.create_fence(&vk::FenceCreateInfo::default(), None) }
			.map_err(|e| anyhow!("vkCreateFence failed: {e:?}"))?;
		let buffers = [command_buffer.handle];
		let submits = [vk::SubmitInfo::default().command_buffers(&buffers)];
		if let Err(e) = unsafe { self.device.queue_submit(self.queue_gfx.handle, &submits, fence) } {
			unsafe { self.device.destroy_fence(fence, None) };
			return Err(anyhow!("vkQueueSubmit for completion marker failed: {e:?}"));
		}

		Ok(RawQueueCompletionMarker {
			context: self.clone(),
			fence,
			_command_buffer: command_buffer,
		})
	}

	pub(super) fn create_command_buffer(
		self: &Arc<Self>,
		queue_type: QueueType,
		usage: CommandBufferUsage,
	) -> anyhow::Result<RawCommandBuffer> {
		let queue = self.queue(queue_type);
		let pool_info = vk::CommandPoolCreateInfo::default()
			.flags(vk::CommandPoolCreateFlags::TRANSIENT)
			.queue_family_index(queue.family_index);
		let pool = unsafe { self.device.create_command_pool(&pool_info, None) }
			.map_err(|e| anyhow!("vkCreateCommandPool failed: {e:?}"))?;
		let alloc_info = vk::CommandBufferAllocateInfo::default()
			.command_pool(pool)
			.level(vk::CommandBufferLevel::PRIMARY)
			.command_buffer_count(1);
		let handle = match unsafe { self.device.allocate_command_buffers(&alloc_info) } {
			Ok(mut buffers) => buffers.remove(0),
			Err(e) => {
				unsafe { self.device.destroy_command_pool(pool, None) };
				return Err(anyhow!("vkAllocateCommandBuffers failed: {e:?}"));
			}
		};
		let begin_info = vk::CommandBufferBeginInfo::default().flags(command_usage_to_vk(usage));
		if let Err(e) = unsafe { self.device.begin_command_buffer(handle, &begin_info) } {
			unsafe { self.device.destroy_command_pool(pool, None) };
			return Err(anyhow!("vkBeginCommandBuffer failed: {e:?}"));
		}

		Ok(RawCommandBuffer {
			context: self.clone(),
			queue,
			pool,
			handle,
			ended: false,
			active_rendering: None,
			external_images: BTreeMap::new(),
		})
	}
}

impl Drop for Context {
	fn drop(&mut self) {
		unsafe {
			let _ = self.device.device_wait_idle();
			self.device.destroy_device(None);
			if let (Some(surface_loader), Some(surface)) = (&self.surface_loader, self.surface) {
				surface_loader.destroy_surface(surface, None);
			}
			self.instance.destroy_instance(None);
		}
	}
}

struct DeviceCandidate {
	physical_device: vk::PhysicalDevice,
	info: DeviceInfo,
	caps: DeviceCaps,
	gfx_family: u32,
	has_capture_queue: bool,
	enabled_extensions: Vec<String>,
	score: i32,
}

pub(super) struct RawSwapchainOwner {
	context: Arc<Context>,
	loader: ash::khr::swapchain::Device,
	handle: vk::SwapchainKHR,
}

impl Drop for RawSwapchainOwner {
	fn drop(&mut self) {
		unsafe {
			let _ = self.context.device.device_wait_idle();
			self.loader.destroy_swapchain(self.handle, None);
		}
	}
}

pub(super) struct RawSwapchain {
	owner: Arc<RawSwapchainOwner>,
	extent: [u32; 2],
	format: Format,
}

impl RawSwapchain {
	pub(super) const fn extent(&self) -> [u32; 2] {
		self.extent
	}

	pub(super) const fn format(&self) -> Format {
		self.format
	}

	pub(super) fn acquire(self: &Arc<Self>) -> Result<RawAcquiredFrame, RawSwapchainError> {
		let semaphore_info = vk::SemaphoreCreateInfo::default();
		let semaphore = unsafe { self.owner.context.device.create_semaphore(&semaphore_info, None) }
			.map_err(|e| RawSwapchainError::Other(format!("vkCreateSemaphore failed: {e:?}")))?;

		match unsafe {
			self
				.owner
				.loader
				.acquire_next_image(self.owner.handle, u64::MAX, semaphore, vk::Fence::null())
		} {
			Ok((image_index, suboptimal)) => Ok(RawAcquiredFrame {
				swapchain: self.clone(),
				image_index,
				acquire_semaphore: Some(semaphore),
				acquire_suboptimal: suboptimal,
			}),
			Err(error) => {
				unsafe { self.owner.context.device.destroy_semaphore(semaphore, None) };
				Err(RawSwapchainError::from_vk(error))
			}
		}
	}
}

pub(super) struct RawAcquiredFrame {
	swapchain: Arc<RawSwapchain>,
	image_index: u32,
	acquire_semaphore: Option<vk::Semaphore>,
	acquire_suboptimal: bool,
}

impl RawAcquiredFrame {
	pub(super) const fn image_index(&self) -> u32 {
		self.image_index
	}

	pub(super) fn submit_and_present(
		mut self,
		command_buffer: &mut RawCommandBuffer,
		image: &Arc<RawImage>,
	) -> Result<RawPresentStatus, RawSwapchainError> {
		let context = &self.swapchain.owner.context;
		if command_buffer.queue.family_index != context.queue_gfx.family_index {
			return Err(RawSwapchainError::Other(
				"swapchain presentation requires a graphics command buffer".to_owned(),
			));
		}

		command_buffer
			.finish()
			.map_err(|e| RawSwapchainError::Other(format!("finishing command buffer: {e:#}")))?;

		let mut present_transition = self
			.swapchain
			.owner
			.context
			.create_command_buffer(QueueType::Graphics, CommandBufferUsage::OneTimeSubmit)
			.map_err(|e| RawSwapchainError::Other(format!("creating present transition command buffer: {e:#}")))?;
		present_transition.transition_image(image, ImageLayout::Present);
		present_transition
			.finish()
			.map_err(|e| RawSwapchainError::Other(format!("finishing present transition command buffer: {e:#}")))?;

		let render_finished = unsafe {
			context
				.device
				.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
		}
		.map_err(|e| RawSwapchainError::Other(format!("vkCreateSemaphore failed: {e:?}")))?;
		let acquire_semaphore = self
			.acquire_semaphore
			.take()
			.expect("acquired frame lost its semaphore");

		let wait_semaphores = [acquire_semaphore];
		let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
		let command_buffers = [command_buffer.handle, present_transition.handle];
		let signal_semaphores = [render_finished];
		let submits = [vk::SubmitInfo::default()
			.wait_semaphores(&wait_semaphores)
			.wait_dst_stage_mask(&wait_stages)
			.command_buffers(&command_buffers)
			.signal_semaphores(&signal_semaphores)];

		let submit_result = unsafe {
			context
				.device
				.queue_submit(context.queue_gfx.handle, &submits, vk::Fence::null())
		};
		if let Err(error) = submit_result {
			unsafe {
				context.device.destroy_semaphore(render_finished, None);
				context.device.destroy_semaphore(acquire_semaphore, None);
			}
			return Err(RawSwapchainError::Other(format!("vkQueueSubmit failed: {error:?}")));
		}

		let swapchains = [self.swapchain.owner.handle];
		let image_indices = [self.image_index];
		let present_wait = [render_finished];
		let present_info = vk::PresentInfoKHR::default()
			.wait_semaphores(&present_wait)
			.swapchains(&swapchains)
			.image_indices(&image_indices);
		let present_result = unsafe {
			self
				.swapchain
				.owner
				.loader
				.queue_present(context.queue_gfx.handle, &present_info)
		};

		// uidev historically waited for every submitted frame. Keep that
		// behavior for now, while ensuring temporary semaphores and the
		// transition command pool are no longer in use before destruction.
		let idle_result = unsafe { context.device.queue_wait_idle(context.queue_gfx.handle) };
		unsafe {
			context.device.destroy_semaphore(render_finished, None);
			context.device.destroy_semaphore(acquire_semaphore, None);
		}
		idle_result.map_err(|e| RawSwapchainError::Other(format!("vkQueueWaitIdle failed: {e:?}")))?;

		match present_result {
			Ok(present_suboptimal) => Ok(if self.acquire_suboptimal || present_suboptimal {
				RawPresentStatus::Suboptimal
			} else {
				RawPresentStatus::Optimal
			}),
			Err(error) => Err(RawSwapchainError::from_vk(error)),
		}
	}
}

impl Drop for RawAcquiredFrame {
	fn drop(&mut self) {
		if let Some(semaphore) = self.acquire_semaphore.take() {
			unsafe {
				self.swapchain.owner.context.device.destroy_semaphore(semaphore, None);
			}
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RawPresentStatus {
	Optimal,
	Suboptimal,
}

#[derive(Debug)]
pub(super) enum RawSwapchainError {
	OutOfDate,
	SurfaceLost,
	Other(String),
}

impl RawSwapchainError {
	fn from_vk(error: vk::Result) -> Self {
		match error {
			vk::Result::ERROR_OUT_OF_DATE_KHR => Self::OutOfDate,
			vk::Result::ERROR_SURFACE_LOST_KHR => Self::SurfaceLost,
			other => Self::Other(format!("Vulkan swapchain operation failed: {other:?}")),
		}
	}
}

pub(super) struct RawBuffer {
	context: Arc<Context>,
	handle: vk::Buffer,
	memory: vk::DeviceMemory,
	mapped: NonNull<u8>,
	coherent: bool,
	pub(super) write_lock: Mutex<()>,
}

unsafe impl Send for RawBuffer {}
unsafe impl Sync for RawBuffer {}

impl RawBuffer {
	pub(super) const fn mapped_ptr(&self) -> NonNull<u8> {
		self.mapped
	}

	pub(super) fn flush(&self) -> anyhow::Result<()> {
		if self.coherent {
			return Ok(());
		}
		let range = [vk::MappedMemoryRange::default()
			.memory(self.memory)
			.offset(0)
			.size(vk::WHOLE_SIZE)];
		unsafe { self.context.device.flush_mapped_memory_ranges(&range) }
			.map_err(|e| anyhow!("vkFlushMappedMemoryRanges failed: {e:?}"))
	}
}

impl Drop for RawBuffer {
	fn drop(&mut self) {
		unsafe {
			self.context.device.unmap_memory(self.memory);
			self.context.device.destroy_buffer(self.handle, None);
			self.context.device.free_memory(self.memory, None);
		}
	}
}

pub(super) struct RawImage {
	context: Arc<Context>,
	handle: vk::Image,
	memory: Option<vk::DeviceMemory>,
	owns_image: bool,
	#[allow(dead_code)]
	swapchain_owner: Option<Arc<RawSwapchainOwner>>,
	layout: Mutex<vk::ImageLayout>,
	external_dma_buf: bool,
}

unsafe impl Send for RawImage {}
unsafe impl Sync for RawImage {}

impl RawImage {
	pub(super) fn raw_handle(&self) -> u64 {
		self.handle.as_raw()
	}

	fn layout(&self) -> vk::ImageLayout {
		*self.layout.lock()
	}

	fn set_layout(&self, layout: vk::ImageLayout) {
		*self.layout.lock() = layout;
	}

	const fn is_external_dma_buf(&self) -> bool {
		self.external_dma_buf
	}
}

impl Drop for RawImage {
	fn drop(&mut self) {
		unsafe {
			if self.owns_image {
				self.context.device.destroy_image(self.handle, None);
			}
			if let Some(memory) = self.memory {
				self.context.device.free_memory(memory, None);
			}
		}
	}
}

pub(super) struct RawImageView {
	context: Arc<Context>,
	#[allow(dead_code)]
	image: Arc<RawImage>,
	handle: vk::ImageView,
}

impl Drop for RawImageView {
	fn drop(&mut self) {
		unsafe { self.context.device.destroy_image_view(self.handle, None) };
	}
}

pub(super) struct RawSampler {
	context: Arc<Context>,
	handle: vk::Sampler,
}

impl Drop for RawSampler {
	fn drop(&mut self) {
		unsafe { self.context.device.destroy_sampler(self.handle, None) };
	}
}

pub(super) struct RawShaderModule {
	context: Arc<Context>,
	handle: vk::ShaderModule,
	stage: ShaderStage,
}

impl Drop for RawShaderModule {
	fn drop(&mut self) {
		unsafe { self.context.device.destroy_shader_module(self.handle, None) };
	}
}

pub(super) struct RawDescriptorSetLayout {
	context: Arc<Context>,
	handle: vk::DescriptorSetLayout,
	bindings: Vec<DescriptorBinding>,
	update_after_bind: bool,
}

impl Drop for RawDescriptorSetLayout {
	fn drop(&mut self) {
		unsafe { self.context.device.destroy_descriptor_set_layout(self.handle, None) };
	}
}

pub(super) struct RawPipeline {
	context: Arc<Context>,
	handle: vk::Pipeline,
	layout: vk::PipelineLayout,
	set_layouts: Vec<Arc<RawDescriptorSetLayout>>,
}

impl RawPipeline {
	pub(super) fn set_layout(&self, set: usize) -> anyhow::Result<Arc<RawDescriptorSetLayout>> {
		self
			.set_layouts
			.get(set)
			.cloned()
			.with_context(|| format!("pipeline has no descriptor set {set}"))
	}
}

impl Drop for RawPipeline {
	fn drop(&mut self) {
		unsafe {
			self.context.device.destroy_pipeline(self.handle, None);
			self.context.device.destroy_pipeline_layout(self.layout, None);
		}
	}
}

pub(super) struct RawDescriptorSet {
	context: Arc<Context>,
	layout: Arc<RawDescriptorSetLayout>,
	pool: vk::DescriptorPool,
	handle: vk::DescriptorSet,
	sampled_images: Mutex<BTreeMap<u32, Arc<RawImage>>>,
}

impl RawDescriptorSet {
	fn binding_type(&self, binding: u32) -> anyhow::Result<DescriptorType> {
		self
			.layout
			.bindings
			.iter()
			.find(|b| b.binding == binding)
			.map(|b| b.descriptor_type)
			.with_context(|| format!("descriptor layout has no binding {binding}"))
	}
}

impl Drop for RawDescriptorSet {
	fn drop(&mut self) {
		unsafe { self.context.device.destroy_descriptor_pool(self.pool, None) };
	}
}

pub(super) struct PipelineSpec {
	pub(super) format: Format,
	pub(super) blend: Option<AttachmentBlend>,
	pub(super) topology: PrimitiveTopology,
	pub(super) instanced: bool,
	pub(super) vertex_stride: u32,
	pub(super) vertex_attributes: Vec<VertexAttribute>,
	pub(super) descriptor_bindings: Vec<DescriptorBinding>,
	pub(super) updatable_sets: Vec<usize>,
}

pub(super) struct RawCommandBuffer {
	context: Arc<Context>,
	queue: RawQueue,
	pool: vk::CommandPool,
	handle: vk::CommandBuffer,
	ended: bool,
	active_rendering: Option<RawRenderingState>,
	external_images: BTreeMap<u64, RawExternalImageState>,
}

pub(super) struct RawQueueCompletionMarker {
	context: Arc<Context>,
	fence: vk::Fence,
	_command_buffer: RawCommandBuffer,
}

impl RawQueueCompletionMarker {
	pub(super) fn is_complete(&self) -> anyhow::Result<bool> {
		unsafe { self.context.device.get_fence_status(self.fence) }.map_err(|e| anyhow!("vkGetFenceStatus failed: {e:?}"))
	}
}

impl Drop for RawQueueCompletionMarker {
	fn drop(&mut self) {
		unsafe { self.context.device.destroy_fence(self.fence, None) };
	}
}

struct RawRenderingState {
	image: Arc<RawImage>,
	image_view: vk::ImageView,
	extent: [u32; 3],
}

struct RawExternalImageState {
	image: Arc<RawImage>,
	layout: vk::ImageLayout,
}

impl RawCommandBuffer {
	pub(super) fn transition_image(&mut self, image: &Arc<RawImage>, new_layout: ImageLayout) {
		let new_layout = image_layout_to_vk(new_layout);

		if image.is_external_dma_buf() {
			let key = image.handle.as_raw();
			if let Some(state) = self.external_images.get_mut(&key) {
				let old_layout = state.layout;
				if old_layout == new_layout {
					return;
				}

				let barrier = [vk::ImageMemoryBarrier::default()
					.src_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
					.dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
					.old_layout(old_layout)
					.new_layout(new_layout)
					.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
					.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
					.image(image.handle)
					.subresource_range(vk::ImageSubresourceRange {
						aspect_mask: vk::ImageAspectFlags::COLOR,
						base_mip_level: 0,
						level_count: 1,
						base_array_layer: 0,
						layer_count: vk::REMAINING_ARRAY_LAYERS,
					})];
				unsafe {
					self.context.device.cmd_pipeline_barrier(
						self.handle,
						vk::PipelineStageFlags::ALL_COMMANDS,
						vk::PipelineStageFlags::ALL_COMMANDS,
						vk::DependencyFlags::empty(),
						&[],
						&[],
						&barrier,
					);
				}
				state.layout = new_layout;
				return;
			}

			// DMA-BUFs are owned by the external producer/consumer between Vulkan
			// command buffers. Acquire ownership and preserve the externally
			// produced contents by transitioning from GENERAL rather than
			// UNDEFINED. The matching release is recorded by finish().
			let barrier = [vk::ImageMemoryBarrier::default()
				.src_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
				.dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
				.old_layout(vk::ImageLayout::GENERAL)
				.new_layout(new_layout)
				.src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
				.dst_queue_family_index(self.queue.family_index)
				.image(image.handle)
				.subresource_range(vk::ImageSubresourceRange {
					aspect_mask: vk::ImageAspectFlags::COLOR,
					base_mip_level: 0,
					level_count: 1,
					base_array_layer: 0,
					layer_count: vk::REMAINING_ARRAY_LAYERS,
				})];
			unsafe {
				self.context.device.cmd_pipeline_barrier(
					self.handle,
					vk::PipelineStageFlags::ALL_COMMANDS,
					vk::PipelineStageFlags::ALL_COMMANDS,
					vk::DependencyFlags::empty(),
					&[],
					&[],
					&barrier,
				);
			}
			self.external_images.insert(
				key,
				RawExternalImageState {
					image: image.clone(),
					layout: new_layout,
				},
			);
			return;
		}

		let old_layout = image.layout();
		if old_layout == new_layout {
			return;
		}

		let src_access = if old_layout == vk::ImageLayout::UNDEFINED {
			vk::AccessFlags::empty()
		} else {
			vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE
		};
		let barrier = [vk::ImageMemoryBarrier::default()
			.src_access_mask(src_access)
			.dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
			.old_layout(old_layout)
			.new_layout(new_layout)
			.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
			.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
			.image(image.handle)
			.subresource_range(vk::ImageSubresourceRange {
				aspect_mask: vk::ImageAspectFlags::COLOR,
				base_mip_level: 0,
				level_count: 1,
				base_array_layer: 0,
				layer_count: vk::REMAINING_ARRAY_LAYERS,
			})];
		unsafe {
			self.context.device.cmd_pipeline_barrier(
				self.handle,
				vk::PipelineStageFlags::ALL_COMMANDS,
				vk::PipelineStageFlags::ALL_COMMANDS,
				vk::DependencyFlags::empty(),
				&[],
				&[],
				&barrier,
			);
		}
		image.set_layout(new_layout);
	}

	fn image_layout(&self, image: &Arc<RawImage>) -> vk::ImageLayout {
		if image.is_external_dma_buf() {
			self
				.external_images
				.get(&image.handle.as_raw())
				.map_or(vk::ImageLayout::GENERAL, |state| state.layout)
		} else {
			image.layout()
		}
	}

	fn release_external_images(&mut self) {
		if self.external_images.is_empty() {
			return;
		}

		let barriers = self
			.external_images
			.values()
			.map(|state| {
				vk::ImageMemoryBarrier::default()
					.src_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
					.dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
					.old_layout(state.layout)
					.new_layout(vk::ImageLayout::GENERAL)
					.src_queue_family_index(self.queue.family_index)
					.dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
					.image(state.image.handle)
					.subresource_range(vk::ImageSubresourceRange {
						aspect_mask: vk::ImageAspectFlags::COLOR,
						base_mip_level: 0,
						level_count: 1,
						base_array_layer: 0,
						layer_count: vk::REMAINING_ARRAY_LAYERS,
					})
			})
			.collect::<Vec<_>>();

		unsafe {
			self.context.device.cmd_pipeline_barrier(
				self.handle,
				vk::PipelineStageFlags::ALL_COMMANDS,
				vk::PipelineStageFlags::ALL_COMMANDS,
				vk::DependencyFlags::empty(),
				&[],
				&[],
				&barriers,
			);
		}
	}

	pub(super) fn begin_rendering(&mut self, view: &RawImageView, extent: [u32; 3], clear: RawClearMode) {
		debug_assert!(self.active_rendering.is_none());
		self.transition_image(&view.image, ImageLayout::ColorAttachment);
		self.begin_rendering_inner(view.handle, extent, clear);
		self.active_rendering = Some(RawRenderingState {
			image: view.image.clone(),
			image_view: view.handle,
			extent,
		});
	}

	fn begin_rendering_inner(&mut self, image_view: vk::ImageView, extent: [u32; 3], clear: RawClearMode) {
		let (load_op, clear_value) = match clear {
			RawClearMode::Keep => (vk::AttachmentLoadOp::LOAD, vk::ClearValue::default()),
			RawClearMode::DontCare => (vk::AttachmentLoadOp::DONT_CARE, vk::ClearValue::default()),
			RawClearMode::Clear(color) => (
				vk::AttachmentLoadOp::CLEAR,
				vk::ClearValue {
					color: vk::ClearColorValue { float32: color },
				},
			),
		};
		let color_attachments = [vk::RenderingAttachmentInfo::default()
			.image_view(image_view)
			.image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
			.load_op(load_op)
			.store_op(vk::AttachmentStoreOp::STORE)
			.clear_value(clear_value)];
		let rendering_info = vk::RenderingInfo::default()
			.render_area(vk::Rect2D {
				offset: vk::Offset2D { x: 0, y: 0 },
				extent: vk::Extent2D {
					width: extent[0],
					height: extent[1],
				},
			})
			.layer_count(1)
			.color_attachments(&color_attachments);
		unsafe { self.context.device.cmd_begin_rendering(self.handle, &rendering_info) };
	}

	pub(super) fn end_rendering(&mut self) {
		if self.active_rendering.take().is_some() {
			unsafe { self.context.device.cmd_end_rendering(self.handle) };
		}
	}

	fn transition_sampled_images(&mut self, descriptor_sets: &[Arc<RawDescriptorSet>]) -> anyhow::Result<()> {
		let sampled_images = descriptor_sets
			.iter()
			.flat_map(|set| set.sampled_images.lock().values().cloned().collect::<Vec<_>>())
			.filter(|image| self.image_layout(image) != vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
			.collect::<Vec<_>>();

		if sampled_images.is_empty() {
			return Ok(());
		}

		if let Some(rendering) = self.active_rendering.as_ref()
			&& sampled_images
				.iter()
				.any(|image| image.handle == rendering.image.handle)
		{
			bail!("the active color attachment cannot also be sampled by this WGfx draw");
		}

		// Image layout transitions are not generally legal inside a dynamic
		// rendering instance. Split the rendering instance, perform all needed
		// transitions outside it, then resume with LOAD so attachment contents
		// are preserved.
		let rendering = self.active_rendering.take();
		if rendering.is_some() {
			unsafe { self.context.device.cmd_end_rendering(self.handle) };
		}

		for image in sampled_images {
			self.transition_image(&image, ImageLayout::ShaderReadOnly);
		}

		if let Some(rendering) = rendering {
			self.begin_rendering_inner(rendering.image_view, rendering.extent, RawClearMode::Keep);
			self.active_rendering = Some(rendering);
		}

		Ok(())
	}

	#[allow(clippy::too_many_arguments)]
	pub(super) fn draw(
		&mut self,
		pipeline: &RawPipeline,
		vertex_buffer: &RawBuffer,
		descriptor_sets: &[Arc<RawDescriptorSet>],
		dimensions: [f32; 2],
		offset: [f32; 2],
		scissor: Scissor,
		vertices: std::ops::Range<u32>,
		instances: std::ops::Range<u32>,
	) -> anyhow::Result<()> {
		// Descriptor writes declare sampled images as SHADER_READ_ONLY_OPTIMAL.
		// Make the actual image layout match immediately before the draw. This
		// keeps descriptor-declared layouts automatic even for
		// long-lived/updatable descriptor sets. DMA-BUF images are also
		// acquired from FOREIGN_EXT here on first use in this command buffer.
		self.transition_sampled_images(descriptor_sets)?;

		let viewport = [vk::Viewport {
			x: offset[0],
			y: offset[1],
			width: dimensions[0],
			height: dimensions[1],
			min_depth: 0.0,
			max_depth: 1.0,
		}];
		let scissors = [vk::Rect2D {
			offset: vk::Offset2D {
				x: scissor.offset[0],
				y: scissor.offset[1],
			},
			extent: vk::Extent2D {
				width: scissor.extent[0],
				height: scissor.extent[1],
			},
		}];
		let sets = descriptor_sets.iter().map(|s| s.handle).collect::<Vec<_>>();
		unsafe {
			let device = &self.context.device;
			device.cmd_set_viewport(self.handle, 0, &viewport);
			device.cmd_set_scissor(self.handle, 0, &scissors);
			device.cmd_bind_pipeline(self.handle, vk::PipelineBindPoint::GRAPHICS, pipeline.handle);
			if !sets.is_empty() {
				device.cmd_bind_descriptor_sets(
					self.handle,
					vk::PipelineBindPoint::GRAPHICS,
					pipeline.layout,
					0,
					&sets,
					&[],
				);
			}
			device.cmd_bind_vertex_buffers(self.handle, 0, &[vertex_buffer.handle], &[0]);
			device.cmd_draw(
				self.handle,
				vertices.end - vertices.start,
				instances.end - instances.start,
				vertices.start,
				instances.start,
			);
		}
		Ok(())
	}

	pub(super) fn copy_buffer_to_image(
		&mut self,
		buffer: &RawBuffer,
		image: &Arc<RawImage>,
		offset: [u32; 3],
		extent: [u32; 3],
	) {
		self.transition_image(image, ImageLayout::TransferDst);
		let regions = [vk::BufferImageCopy::default()
			.buffer_offset(0)
			.buffer_row_length(0)
			.buffer_image_height(0)
			.image_subresource(vk::ImageSubresourceLayers {
				aspect_mask: vk::ImageAspectFlags::COLOR,
				mip_level: 0,
				base_array_layer: 0,
				layer_count: 1,
			})
			.image_offset(vk::Offset3D {
				x: offset[0] as i32,
				y: offset[1] as i32,
				z: offset[2] as i32,
			})
			.image_extent(vk::Extent3D {
				width: extent[0],
				height: extent[1],
				depth: extent[2],
			})];
		unsafe {
			self.context.device.cmd_copy_buffer_to_image(
				self.handle,
				buffer.handle,
				image.handle,
				vk::ImageLayout::TRANSFER_DST_OPTIMAL,
				&regions,
			);
		}
		self.transition_image(image, ImageLayout::ShaderReadOnly);
	}

	pub(super) fn clear_image(&mut self, image: &Arc<RawImage>) {
		self.transition_image(image, ImageLayout::TransferDst);
		let ranges = [vk::ImageSubresourceRange {
			aspect_mask: vk::ImageAspectFlags::COLOR,
			base_mip_level: 0,
			level_count: 1,
			base_array_layer: 0,
			layer_count: vk::REMAINING_ARRAY_LAYERS,
		}];
		let color = vk::ClearColorValue { uint32: [0, 0, 0, 0] };
		unsafe {
			self.context.device.cmd_clear_color_image(
				self.handle,
				image.handle,
				vk::ImageLayout::TRANSFER_DST_OPTIMAL,
				&color,
				&ranges,
			);
		}
		self.transition_image(image, ImageLayout::ShaderReadOnly);
	}

	pub(super) fn copy_image(
		&mut self,
		src: &Arc<RawImage>,
		src_offset: [u32; 3],
		dst: &Arc<RawImage>,
		dst_offset: [u32; 3],
		extent: [u32; 3],
	) {
		self.transition_image(src, ImageLayout::TransferSrc);
		self.transition_image(dst, ImageLayout::TransferDst);
		let regions = [vk::ImageCopy::default()
			.src_subresource(vk::ImageSubresourceLayers {
				aspect_mask: vk::ImageAspectFlags::COLOR,
				mip_level: 0,
				base_array_layer: 0,
				layer_count: 1,
			})
			.src_offset(vk::Offset3D {
				x: src_offset[0] as i32,
				y: src_offset[1] as i32,
				z: src_offset[2] as i32,
			})
			.dst_subresource(vk::ImageSubresourceLayers {
				aspect_mask: vk::ImageAspectFlags::COLOR,
				mip_level: 0,
				base_array_layer: 0,
				layer_count: 1,
			})
			.dst_offset(vk::Offset3D {
				x: dst_offset[0] as i32,
				y: dst_offset[1] as i32,
				z: dst_offset[2] as i32,
			})
			.extent(vk::Extent3D {
				width: extent[0],
				height: extent[1],
				depth: extent[2],
			})];
		unsafe {
			self.context.device.cmd_copy_image(
				self.handle,
				src.handle,
				vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
				dst.handle,
				vk::ImageLayout::TRANSFER_DST_OPTIMAL,
				&regions,
			);
		}
		self.transition_image(src, ImageLayout::ShaderReadOnly);
		self.transition_image(dst, ImageLayout::ShaderReadOnly);
	}

	fn record_queue_completion_barrier(&mut self) {
		debug_assert!(!self.ended);
		debug_assert!(self.active_rendering.is_none());
		let memory_barriers = [vk::MemoryBarrier::default()
			.src_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
			.dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)];
		unsafe {
			self.context.device.cmd_pipeline_barrier(
				self.handle,
				vk::PipelineStageFlags::ALL_COMMANDS,
				vk::PipelineStageFlags::ALL_COMMANDS,
				vk::DependencyFlags::empty(),
				&memory_barriers,
				&[],
				&[],
			);
		}
	}

	pub(super) fn finish(&mut self) -> anyhow::Result<()> {
		if !self.ended {
			if self.active_rendering.is_some() {
				bail!("cannot finish a command buffer while dynamic rendering is active");
			}
			self.release_external_images();
			unsafe { self.context.device.end_command_buffer(self.handle) }
				.map_err(|e| anyhow!("vkEndCommandBuffer failed: {e:?}"))?;
			self.ended = true;
		}
		Ok(())
	}

	pub(super) fn submit_and_wait(&mut self) -> anyhow::Result<()> {
		self.finish()?;
		let buffers = [self.handle];
		let submits = [vk::SubmitInfo::default().command_buffers(&buffers)];
		let fence_info = vk::FenceCreateInfo::default();
		let fence = unsafe { self.context.device.create_fence(&fence_info, None) }
			.map_err(|e| anyhow!("vkCreateFence failed: {e:?}"))?;
		let submit_result = unsafe { self.context.device.queue_submit(self.queue.handle, &submits, fence) }
			.map_err(|e| anyhow!("vkQueueSubmit failed: {e:?}"));
		let wait_result = if submit_result.is_ok() {
			unsafe { self.context.device.wait_for_fences(&[fence], true, u64::MAX) }
				.map_err(|e| anyhow!("vkWaitForFences failed: {e:?}"))
		} else {
			Ok(())
		};
		unsafe { self.context.device.destroy_fence(fence, None) };
		submit_result?;
		wait_result
	}
}

impl Drop for RawCommandBuffer {
	fn drop(&mut self) {
		unsafe { self.context.device.destroy_command_pool(self.pool, None) };
	}
}

#[derive(Debug, Clone, Copy)]
pub(super) enum RawClearMode {
	DontCare,
	Keep,
	Clear([f32; 4]),
}

fn window_surface_extensions(display: RawDisplayHandle) -> anyhow::Result<Vec<String>> {
	let platform = match display {
		RawDisplayHandle::Wayland(_) => "VK_KHR_wayland_surface",
		RawDisplayHandle::Xlib(_) => "VK_KHR_xlib_surface",
		RawDisplayHandle::Xcb(_) => "VK_KHR_xcb_surface",
		_ => bail!("unsupported native display backend for WGfx window surface"),
	};

	Ok(vec!["VK_KHR_surface".to_owned(), platform.to_owned()])
}

fn create_window_surface(
	entry: &Entry,
	instance: &ash::Instance,
	display: RawDisplayHandle,
	window: RawWindowHandle,
) -> anyhow::Result<vk::SurfaceKHR> {
	match (display, window) {
		(RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(window)) => {
			let loader = ash::khr::wayland_surface::Instance::new(entry, instance);
			let info = vk::WaylandSurfaceCreateInfoKHR::default()
				.display(display.display.as_ptr().cast())
				.surface(window.surface.as_ptr().cast());
			unsafe { loader.create_wayland_surface(&info, None) }
				.map_err(|e| anyhow!("vkCreateWaylandSurfaceKHR failed: {e:?}"))
		}
		(RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(window)) => {
			let loader = ash::khr::xlib_surface::Instance::new(entry, instance);
			let dpy = display
				.display
				.context("Xlib display handle has no display pointer")?
				.as_ptr()
				.cast();
			let info = vk::XlibSurfaceCreateInfoKHR::default().dpy(dpy).window(window.window);
			unsafe { loader.create_xlib_surface(&info, None) }.map_err(|e| anyhow!("vkCreateXlibSurfaceKHR failed: {e:?}"))
		}
		(RawDisplayHandle::Xcb(display), RawWindowHandle::Xcb(window)) => {
			let loader = ash::khr::xcb_surface::Instance::new(entry, instance);
			let connection = display
				.connection
				.context("Xcb display handle has no connection pointer")?
				.as_ptr()
				.cast();
			let info = vk::XcbSurfaceCreateInfoKHR::default()
				.connection(connection)
				.window(window.window.get());
			unsafe { loader.create_xcb_surface(&info, None) }.map_err(|e| anyhow!("vkCreateXcbSurfaceKHR failed: {e:?}"))
		}
		_ => bail!("native display/window handle pair is unsupported or mismatched"),
	}
}

fn choose_surface_format(
	loader: &ash::khr::surface::Instance,
	physical_device: vk::PhysicalDevice,
	surface: vk::SurfaceKHR,
	preferred: Format,
) -> anyhow::Result<vk::SurfaceFormatKHR> {
	let formats = unsafe { loader.get_physical_device_surface_formats(physical_device, surface) }
		.map_err(|e| anyhow!("vkGetPhysicalDeviceSurfaceFormatsKHR failed: {e:?}"))?;
	if formats.is_empty() {
		bail!("Vulkan window surface reports no formats");
	}

	if formats.len() == 1 && formats[0].format == vk::Format::UNDEFINED {
		return Ok(vk::SurfaceFormatKHR {
			format: format_to_vk(preferred),
			color_space: formats[0].color_space,
		});
	}

	formats
		.iter()
		.copied()
		.find(|surface_format| {
			format_from_vk(surface_format.format).is_some() && surface_format_is_8bit(surface_format.format)
		})
		.or_else(|| {
			formats
				.iter()
				.copied()
				.find(|surface_format| format_from_vk(surface_format.format).is_some())
		})
		.context("window surface has no format supported by WGfx")
}

const fn surface_format_is_8bit(format: vk::Format) -> bool {
	matches!(
		format,
		vk::Format::R8G8B8_UNORM
			| vk::Format::R8G8B8A8_UNORM
			| vk::Format::R8G8B8A8_SRGB
			| vk::Format::B8G8R8_UNORM
			| vk::Format::B8G8R8A8_UNORM
			| vk::Format::B8G8R8A8_SRGB
	)
}

fn strings_to_cstrings(strings: &[String]) -> anyhow::Result<Vec<CString>> {
	strings
		.iter()
		.map(|s| CString::new(s.as_str()).with_context(|| format!("Vulkan extension contains NUL: {s:?}")))
		.collect()
}

fn matches_preference(info: &DeviceInfo, preference: &DevicePreference) -> bool {
	match preference {
		DevicePreference::Default => true,
		DevicePreference::VendorDevice { vendor_id, device_id } => {
			info.vendor_id == *vendor_id && info.device_id == *device_id
		}
		DevicePreference::Uuid(uuid) => info.uuid == *uuid,
		DevicePreference::DrmRenderNode { major, minor } => info.drm_render_node == Some((*major, *minor)),
	}
}

fn device_score(device_type: DeviceType, has_capture_queue: bool) -> i32 {
	let base = match device_type {
		DeviceType::DiscreteGpu => 500,
		DeviceType::IntegratedGpu => 400,
		DeviceType::VirtualGpu => 300,
		DeviceType::Other => 200,
		DeviceType::Cpu => 100,
	};
	base + i32::from(has_capture_queue) * 10
}

const fn device_type_from_vk(value: vk::PhysicalDeviceType) -> DeviceType {
	match value {
		vk::PhysicalDeviceType::INTEGRATED_GPU => DeviceType::IntegratedGpu,
		vk::PhysicalDeviceType::DISCRETE_GPU => DeviceType::DiscreteGpu,
		vk::PhysicalDeviceType::VIRTUAL_GPU => DeviceType::VirtualGpu,
		vk::PhysicalDeviceType::CPU => DeviceType::Cpu,
		_ => DeviceType::Other,
	}
}

const fn format_to_vk(format: Format) -> vk::Format {
	match format {
		Format::R8_UNORM => vk::Format::R8_UNORM,
		Format::R8G8B8_UNORM => vk::Format::R8G8B8_UNORM,
		Format::R8G8B8A8_UNORM => vk::Format::R8G8B8A8_UNORM,
		Format::R8G8B8A8_SRGB => vk::Format::R8G8B8A8_SRGB,
		Format::B8G8R8_UNORM => vk::Format::B8G8R8_UNORM,
		Format::B8G8R8A8_UNORM => vk::Format::B8G8R8A8_UNORM,
		Format::B8G8R8A8_SRGB => vk::Format::B8G8R8A8_SRGB,
		Format::A2B10G10R10_UNORM_PACK32 => vk::Format::A2B10G10R10_UNORM_PACK32,
		Format::R16G16B16A16_SFLOAT => vk::Format::R16G16B16A16_SFLOAT,
		Format::R32G32B32A32_SFLOAT => vk::Format::R32G32B32A32_SFLOAT,
		Format::BC1_RGBA_UNORM_BLOCK => vk::Format::BC1_RGBA_UNORM_BLOCK,
		Format::BC1_RGBA_SRGB_BLOCK => vk::Format::BC1_RGBA_SRGB_BLOCK,
		Format::BC2_UNORM_BLOCK => vk::Format::BC2_UNORM_BLOCK,
		Format::BC2_SRGB_BLOCK => vk::Format::BC2_SRGB_BLOCK,
		Format::BC3_UNORM_BLOCK => vk::Format::BC3_UNORM_BLOCK,
		Format::BC3_SRGB_BLOCK => vk::Format::BC3_SRGB_BLOCK,
		Format::BC4_UNORM_BLOCK => vk::Format::BC4_UNORM_BLOCK,
		Format::BC4_SNORM_BLOCK => vk::Format::BC4_SNORM_BLOCK,
		Format::BC5_UNORM_BLOCK => vk::Format::BC5_UNORM_BLOCK,
		Format::BC5_SNORM_BLOCK => vk::Format::BC5_SNORM_BLOCK,
		Format::BC6H_UFLOAT_BLOCK => vk::Format::BC6H_UFLOAT_BLOCK,
		Format::BC6H_SFLOAT_BLOCK => vk::Format::BC6H_SFLOAT_BLOCK,
		Format::BC7_UNORM_BLOCK => vk::Format::BC7_UNORM_BLOCK,
		Format::BC7_SRGB_BLOCK => vk::Format::BC7_SRGB_BLOCK,
	}
}

const fn format_from_vk(format: vk::Format) -> Option<Format> {
	Some(match format {
		vk::Format::R8_UNORM => Format::R8_UNORM,
		vk::Format::R8G8B8_UNORM => Format::R8G8B8_UNORM,
		vk::Format::R8G8B8A8_UNORM => Format::R8G8B8A8_UNORM,
		vk::Format::R8G8B8A8_SRGB => Format::R8G8B8A8_SRGB,
		vk::Format::B8G8R8_UNORM => Format::B8G8R8_UNORM,
		vk::Format::B8G8R8A8_UNORM => Format::B8G8R8A8_UNORM,
		vk::Format::B8G8R8A8_SRGB => Format::B8G8R8A8_SRGB,
		vk::Format::A2B10G10R10_UNORM_PACK32 => Format::A2B10G10R10_UNORM_PACK32,
		vk::Format::R16G16B16A16_SFLOAT => Format::R16G16B16A16_SFLOAT,
		vk::Format::R32G32B32A32_SFLOAT => Format::R32G32B32A32_SFLOAT,
		vk::Format::BC1_RGBA_UNORM_BLOCK => Format::BC1_RGBA_UNORM_BLOCK,
		vk::Format::BC1_RGBA_SRGB_BLOCK => Format::BC1_RGBA_SRGB_BLOCK,
		vk::Format::BC2_UNORM_BLOCK => Format::BC2_UNORM_BLOCK,
		vk::Format::BC2_SRGB_BLOCK => Format::BC2_SRGB_BLOCK,
		vk::Format::BC3_UNORM_BLOCK => Format::BC3_UNORM_BLOCK,
		vk::Format::BC3_SRGB_BLOCK => Format::BC3_SRGB_BLOCK,
		vk::Format::BC4_UNORM_BLOCK => Format::BC4_UNORM_BLOCK,
		vk::Format::BC4_SNORM_BLOCK => Format::BC4_SNORM_BLOCK,
		vk::Format::BC5_UNORM_BLOCK => Format::BC5_UNORM_BLOCK,
		vk::Format::BC5_SNORM_BLOCK => Format::BC5_SNORM_BLOCK,
		vk::Format::BC6H_UFLOAT_BLOCK => Format::BC6H_UFLOAT_BLOCK,
		vk::Format::BC6H_SFLOAT_BLOCK => Format::BC6H_SFLOAT_BLOCK,
		vk::Format::BC7_UNORM_BLOCK => Format::BC7_UNORM_BLOCK,
		vk::Format::BC7_SRGB_BLOCK => Format::BC7_SRGB_BLOCK,
		_ => return None,
	})
}

fn buffer_usage_to_vk(usage: BufferUsage) -> vk::BufferUsageFlags {
	let mut out = vk::BufferUsageFlags::empty();
	if usage.contains(BufferUsage::TRANSFER_SRC) {
		out |= vk::BufferUsageFlags::TRANSFER_SRC;
	}
	if usage.contains(BufferUsage::TRANSFER_DST) {
		out |= vk::BufferUsageFlags::TRANSFER_DST;
	}
	if usage.contains(BufferUsage::UNIFORM_BUFFER) {
		out |= vk::BufferUsageFlags::UNIFORM_BUFFER;
	}
	if usage.contains(BufferUsage::STORAGE_BUFFER) {
		out |= vk::BufferUsageFlags::STORAGE_BUFFER;
	}
	if usage.contains(BufferUsage::INDEX_BUFFER) {
		out |= vk::BufferUsageFlags::INDEX_BUFFER;
	}
	if usage.contains(BufferUsage::VERTEX_BUFFER) {
		out |= vk::BufferUsageFlags::VERTEX_BUFFER;
	}
	out
}

fn image_usage_to_vk(usage: ImageUsage) -> vk::ImageUsageFlags {
	let mut out = vk::ImageUsageFlags::empty();
	if usage.contains(ImageUsage::TRANSFER_SRC) {
		out |= vk::ImageUsageFlags::TRANSFER_SRC;
	}
	if usage.contains(ImageUsage::TRANSFER_DST) {
		out |= vk::ImageUsageFlags::TRANSFER_DST;
	}
	if usage.contains(ImageUsage::SAMPLED) {
		out |= vk::ImageUsageFlags::SAMPLED;
	}
	if usage.contains(ImageUsage::COLOR_ATTACHMENT) {
		out |= vk::ImageUsageFlags::COLOR_ATTACHMENT;
	}
	out
}

const fn image_tiling_to_vk(tiling: ImageTiling) -> vk::ImageTiling {
	match tiling {
		ImageTiling::Optimal => vk::ImageTiling::OPTIMAL,
		ImageTiling::Linear => vk::ImageTiling::LINEAR,
		ImageTiling::DrmFormatModifier => vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT,
	}
}

const fn filter_to_vk(filter: Filter) -> vk::Filter {
	match filter {
		Filter::Nearest => vk::Filter::NEAREST,
		Filter::Linear => vk::Filter::LINEAR,
		Filter::Cubic => vk::Filter::CUBIC_EXT,
	}
}

const fn address_mode_to_vk(mode: SamplerAddressMode) -> vk::SamplerAddressMode {
	match mode {
		SamplerAddressMode::Repeat => vk::SamplerAddressMode::REPEAT,
		SamplerAddressMode::MirroredRepeat => vk::SamplerAddressMode::MIRRORED_REPEAT,
		SamplerAddressMode::ClampToEdge => vk::SamplerAddressMode::CLAMP_TO_EDGE,
		SamplerAddressMode::ClampToBorder => vk::SamplerAddressMode::CLAMP_TO_BORDER,
	}
}

const fn command_usage_to_vk(usage: CommandBufferUsage) -> vk::CommandBufferUsageFlags {
	match usage {
		CommandBufferUsage::OneTimeSubmit => vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT,
		CommandBufferUsage::MultipleSubmit => vk::CommandBufferUsageFlags::empty(),
		CommandBufferUsage::SimultaneousUse => vk::CommandBufferUsageFlags::SIMULTANEOUS_USE,
	}
}

const fn topology_to_vk(topology: PrimitiveTopology) -> vk::PrimitiveTopology {
	match topology {
		PrimitiveTopology::PointList => vk::PrimitiveTopology::POINT_LIST,
		PrimitiveTopology::LineList => vk::PrimitiveTopology::LINE_LIST,
		PrimitiveTopology::LineStrip => vk::PrimitiveTopology::LINE_STRIP,
		PrimitiveTopology::TriangleList => vk::PrimitiveTopology::TRIANGLE_LIST,
		PrimitiveTopology::TriangleStrip => vk::PrimitiveTopology::TRIANGLE_STRIP,
	}
}

const fn blend_factor_to_vk(factor: BlendFactor) -> vk::BlendFactor {
	match factor {
		BlendFactor::Zero => vk::BlendFactor::ZERO,
		BlendFactor::One => vk::BlendFactor::ONE,
		BlendFactor::SrcAlpha => vk::BlendFactor::SRC_ALPHA,
		BlendFactor::OneMinusSrcAlpha => vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
	}
}

const fn blend_op_to_vk(op: BlendOp) -> vk::BlendOp {
	match op {
		BlendOp::Add => vk::BlendOp::ADD,
		BlendOp::Max => vk::BlendOp::MAX,
	}
}

fn blend_to_vk(blend: AttachmentBlend) -> vk::PipelineColorBlendAttachmentState {
	vk::PipelineColorBlendAttachmentState {
		blend_enable: vk::TRUE,
		src_color_blend_factor: blend_factor_to_vk(blend.src_color_blend_factor),
		dst_color_blend_factor: blend_factor_to_vk(blend.dst_color_blend_factor),
		color_blend_op: blend_op_to_vk(blend.color_blend_op),
		src_alpha_blend_factor: blend_factor_to_vk(blend.src_alpha_blend_factor),
		dst_alpha_blend_factor: blend_factor_to_vk(blend.dst_alpha_blend_factor),
		alpha_blend_op: blend_op_to_vk(blend.alpha_blend_op),
		color_write_mask: vk::ColorComponentFlags::R
			| vk::ColorComponentFlags::G
			| vk::ColorComponentFlags::B
			| vk::ColorComponentFlags::A,
	}
}

const fn descriptor_type_to_vk(ty: DescriptorType) -> vk::DescriptorType {
	match ty {
		DescriptorType::CombinedImageSampler => vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
		DescriptorType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
		DescriptorType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
	}
}

fn shader_stages_to_vk(stages: ShaderStages) -> vk::ShaderStageFlags {
	let mut out = vk::ShaderStageFlags::empty();
	if stages.contains(ShaderStages::VERTEX) {
		out |= vk::ShaderStageFlags::VERTEX;
	}
	if stages.contains(ShaderStages::FRAGMENT) {
		out |= vk::ShaderStageFlags::FRAGMENT;
	}
	out
}

const fn vertex_format_to_vk(format: VertexFormat) -> vk::Format {
	match format {
		VertexFormat::R32Uint => vk::Format::R32_UINT,
		VertexFormat::R32Sfloat => vk::Format::R32_SFLOAT,
		VertexFormat::R32g32Sfloat => vk::Format::R32G32_SFLOAT,
	}
}

const fn image_layout_to_vk(layout: ImageLayout) -> vk::ImageLayout {
	match layout {
		ImageLayout::TransferSrc => vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
		ImageLayout::TransferDst => vk::ImageLayout::TRANSFER_DST_OPTIMAL,
		ImageLayout::ShaderReadOnly => vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
		ImageLayout::ColorAttachment => vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
		ImageLayout::Present => vk::ImageLayout::PRESENT_SRC_KHR,
		ImageLayout::General => vk::ImageLayout::GENERAL,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn usage_conversion_is_internal_and_lossless_for_known_bits() {
		let flags = buffer_usage_to_vk(BufferUsage::TRANSFER_DST | BufferUsage::VERTEX_BUFFER);
		assert!(flags.contains(vk::BufferUsageFlags::TRANSFER_DST));
		assert!(flags.contains(vk::BufferUsageFlags::VERTEX_BUFFER));
	}
}
