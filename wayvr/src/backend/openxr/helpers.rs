use anyhow::{bail, ensure};
use glam::{Affine3A, Quat, Vec3, Vec3A};
use openxr::{self as xr, SessionCreateFlags, Version, sys::Handle};
use xr::OverlaySessionCreateFlagsEXTX;

macro_rules! next_chain_insert {
    ($layer:expr, $payload:expr) => {{
        let payload_ptr = $payload.as_mut() as *mut _ as *mut xr::sys::BaseInStructure;
        let new_elem = payload_ptr.as_mut().unwrap();
        let mut raw = $layer.into_raw();
        new_elem.next = raw.next as _;
        raw.next = payload_ptr as *const _;
        raw
    }};
}
pub(crate) use next_chain_insert;

pub(super) fn init_xr() -> Result<(xr::Instance, xr::SystemId), anyhow::Error> {
    let entry = xr::Entry::linked();

    let Ok(available_extensions) = entry.enumerate_extensions() else {
        bail!("Failed to enumerate OpenXR extensions.");
    };
    ensure!(
        available_extensions.khr_vulkan_enable2,
        "Missing KHR_vulkan_enable2 extension."
    );
    ensure!(
        available_extensions.extx_overlay,
        "Missing EXTX_overlay extension."
    );

    let mut enabled_extensions = xr::ExtensionSet::default();
    enabled_extensions.khr_vulkan_enable2 = true;
    enabled_extensions.extx_overlay = true;
    if available_extensions.khr_binding_modification && available_extensions.ext_dpad_binding {
        enabled_extensions.khr_binding_modification = true;
        enabled_extensions.ext_dpad_binding = true;
    } else {
        log::warn!("Missing EXT_dpad_binding extension.");
    }
    if available_extensions.ext_hp_mixed_reality_controller {
        enabled_extensions.ext_hp_mixed_reality_controller = true;
    } else {
        log::warn!("Missing EXT_hp_mixed_reality_controller extension.");
    }
    if available_extensions.ext_hand_interaction {
        enabled_extensions.ext_hand_interaction = true;
    } else {
        log::warn!("Missing EXT_hand_interaction extension.");
    }
    if available_extensions.ext_eye_gaze_interaction {
        enabled_extensions.ext_eye_gaze_interaction = true;
    } else {
        log::warn!("Missing EXT_eye_gaze_interaction extension.");
    }
    if available_extensions.khr_composition_layer_cylinder {
        enabled_extensions.khr_composition_layer_cylinder = true;
    } else {
        log::warn!("Missing EXT_composition_layer_cylinder extension.");
    }
    if available_extensions.khr_composition_layer_equirect2 {
        enabled_extensions.khr_composition_layer_equirect2 = true;
    } else {
        log::warn!("Missing EXT_composition_layer_equirect2 extension.");
    }
    if available_extensions.khr_composition_layer_color_scale_bias {
        enabled_extensions.khr_composition_layer_color_scale_bias = true;
    } else {
        log::warn!("Missing XR_KHR_composition_layer_color_scale_bias extension.");
    }

    let xr_mndx_system_buttons = "XR_MNDX_system_buttons".as_bytes().to_vec();
    if available_extensions.other.contains(&xr_mndx_system_buttons) {
        enabled_extensions.other.push(xr_mndx_system_buttons);
    }

    //#[cfg(not(debug_assertions))]
    let layers = [];
    //#[cfg(debug_assertions)]
    //let layers = [
    //    "XR_APILAYER_LUNARG_api_dump",
    //    "XR_APILAYER_LUNARG_standard_validation",
    //];

    let Ok(xr_instance) = entry.create_instance(
        &xr::ApplicationInfo {
            api_version: Version::new(1, 0, 32),
            application_name: "wayvr",
            application_version: 0,
            engine_name: "wayvr",
            engine_version: 0,
        },
        &enabled_extensions,
        &layers,
    ) else {
        bail!("Failed to create OpenXR instance.");
    };

    let Ok(instance_props) = xr_instance.properties() else {
        bail!("Failed to query OpenXR instance properties.");
    };
    log::info!(
        "Using OpenXR runtime: {} {}",
        instance_props.runtime_name,
        instance_props.runtime_version
    );

    let Ok(system) = xr_instance.system(xr::FormFactor::HEAD_MOUNTED_DISPLAY) else {
        bail!("Failed to access OpenXR HMD system.");
    };

    let vk_target_version_xr = xr::Version::new(1, 1, 0);

    let Ok(reqs) = xr_instance.graphics_requirements::<xr::Vulkan>(system) else {
        bail!("Failed to query OpenXR Vulkan requirements.");
    };

    if vk_target_version_xr < reqs.min_api_version_supported
        || vk_target_version_xr.major() > reqs.max_api_version_supported.major()
    {
        bail!(
            "OpenXR runtime requires Vulkan version > {}, < {}.0.0",
            reqs.min_api_version_supported,
            reqs.max_api_version_supported.major() + 1
        );
    }

    Ok((xr_instance, system))
}

pub(super) unsafe fn create_overlay_session(
    instance: &xr::Instance,
    system: xr::SystemId,
    info: &xr::vulkan::SessionCreateInfo,
) -> Result<xr::sys::Session, xr::sys::Result> {
    let overlay = xr::sys::SessionCreateInfoOverlayEXTX {
        ty: xr::sys::SessionCreateInfoOverlayEXTX::TYPE,
        next: std::ptr::null(),
        create_flags: OverlaySessionCreateFlagsEXTX::EMPTY,
        session_layers_placement: 5,
    };
    let binding = xr::sys::GraphicsBindingVulkanKHR {
        ty: xr::sys::GraphicsBindingVulkanKHR::TYPE,
        next: (&raw const overlay).cast(),
        instance: info.instance,
        physical_device: info.physical_device,
        device: info.device,
        queue_family_index: info.queue_family_index,
        queue_index: info.queue_index,
    };
    let info = xr::sys::SessionCreateInfo {
        ty: xr::sys::SessionCreateInfo::TYPE,
        next: (&raw const binding).cast(),
        create_flags: SessionCreateFlags::default(),
        system_id: system,
    };
    let mut out = xr::sys::Session::NULL;
    let x =
        unsafe { (instance.fp().create_session)(instance.as_raw(), &raw const info, &raw mut out) };
    if x.into_raw() >= 0 { Ok(out) } else { Err(x) }
}

type Vec3M = mint::Vector3<f32>;
type QuatM = mint::Quaternion<f32>;

pub(super) fn ipd_from_views(views: &[xr::View]) -> f32 {
    let p0: Vec3 = Vec3M::from(views[0].pose.position).into();
    let p1: Vec3 = Vec3M::from(views[1].pose.position).into();

    (p0.distance(p1) * 10000.0).round() * 0.1
}

pub(super) fn transform_to_norm_quat(transform: &Affine3A) -> Quat {
    let norm_mat3 = transform
        .matrix3
        .mul_scalar(1.0 / transform.matrix3.x_axis.length());
    Quat::from_mat3a(&norm_mat3).normalize()
}

pub(super) fn translation_rotation_to_posef(translation: Vec3A, mut rotation: Quat) -> xr::Posef {
    if !rotation.is_finite() {
        rotation = Quat::IDENTITY;
    }

    xr::Posef {
        orientation: xr::Quaternionf {
            x: rotation.x,
            y: rotation.y,
            z: rotation.z,
            w: rotation.w,
        },
        position: xr::Vector3f {
            x: translation.x,
            y: translation.y,
            z: translation.z,
        },
    }
}

pub(super) fn transform_to_posef(transform: &Affine3A) -> xr::Posef {
    let translation = transform.translation;
    let rotation = transform_to_norm_quat(transform);
    translation_rotation_to_posef(translation, rotation)
}

pub(super) fn posef_to_transform(pose: &xr::Posef) -> Affine3A {
    let rotation = QuatM::from(pose.orientation).into();
    let translation = Vec3M::from(pose.position).into();
    Affine3A::from_rotation_translation(rotation, translation)
}
