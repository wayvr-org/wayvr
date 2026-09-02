use wayvr_ipc::packet_client::{
    WlxMouseTransform, WlxStereoMode, WlxWindowAttrib, WlxWindowAttribGetParams,
    WlxWindowAttribSetParams, WlxWindowAttribValue,
};
use wlx_common::overlays::{BackendAttrib, BackendAttribValue, MouseTransform, StereoMode};

use crate::{
    state::AppState,
    windowing::manager::OverlayWindowManager,
};

pub fn get_attrib<T>(
    overlays: &mut OverlayWindowManager<T>,
    params: &WlxWindowAttribGetParams,
) -> Result<WlxWindowAttribValue, String>
where
    T: Default,
{
    let Some(id) = overlays.lookup(&params.overlay) else {
        return Err(format!("overlay '{}' not found", params.overlay));
    };

    let overlay = overlays
        .mut_by_id(id)
        .ok_or_else(|| format!("overlay '{}' not found", params.overlay))?;

    let attrib = match params.attrib {
        WlxWindowAttrib::Stereo => BackendAttrib::Stereo,
        WlxWindowAttrib::StereoFullFrame => BackendAttrib::StereoFullFrame,
        WlxWindowAttrib::StereoAdjustMouse => BackendAttrib::StereoAdjustMouse,
        WlxWindowAttrib::MouseTransform => BackendAttrib::MouseTransform,
        WlxWindowAttrib::WindowSize => BackendAttrib::WindowSize,
    };

    let value = overlay.config.backend.get_attrib(attrib).ok_or_else(|| {
        format!(
            "overlay '{}' does not support attribute {:#?}",
            params.overlay, params.attrib
        )
    })?;

    Ok(from_backend_value(value))
}

pub fn set_attrib<T>(
    app: &mut AppState,
    overlays: &mut OverlayWindowManager<T>,
    params: WlxWindowAttribSetParams,
) -> Result<(), String>
where
    T: Default,
{
    let Some(id) = overlays.lookup(&params.overlay) else {
        return Err(format!("overlay '{}' not found", params.overlay));
    };

    let overlay = overlays
        .mut_by_id(id)
        .ok_or_else(|| format!("overlay '{}' not found", params.overlay))?;

    if !overlay
        .config
        .backend
        .set_attrib(app, to_backend_value(params.value))
    {
        return Err(format!(
            "overlay '{}' does not support setting attribute {:#?}",
            params.overlay, params.attrib
        ));
    }

    Ok(())
}

const fn to_backend_value(value: WlxWindowAttribValue) -> BackendAttribValue {
    match value {
        WlxWindowAttribValue::Stereo(mode) => BackendAttribValue::Stereo(match mode {
            WlxStereoMode::None => StereoMode::None,
            WlxStereoMode::LeftRight => StereoMode::LeftRight,
            WlxStereoMode::RightLeft => StereoMode::RightLeft,
            WlxStereoMode::TopBottom => StereoMode::TopBottom,
            WlxStereoMode::BottomTop => StereoMode::BottomTop,
        }),
        WlxWindowAttribValue::StereoFullFrame(value) => BackendAttribValue::StereoFullFrame(value),
        WlxWindowAttribValue::StereoAdjustMouse(value) => {
            BackendAttribValue::StereoAdjustMouse(value)
        }
        WlxWindowAttribValue::MouseTransform(transform) => {
            BackendAttribValue::MouseTransform(match transform {
                WlxMouseTransform::Default => MouseTransform::Default,
                WlxMouseTransform::Normal => MouseTransform::Normal,
                WlxMouseTransform::Rotated90 => MouseTransform::Rotated90,
                WlxMouseTransform::Rotated180 => MouseTransform::Rotated180,
                WlxMouseTransform::Rotated270 => MouseTransform::Rotated270,
                WlxMouseTransform::Flipped => MouseTransform::Flipped,
                WlxMouseTransform::Flipped90 => MouseTransform::Flipped90,
                WlxMouseTransform::Flipped180 => MouseTransform::Flipped180,
                WlxMouseTransform::Flipped270 => MouseTransform::Flipped270,
            })
        }
        WlxWindowAttribValue::WindowSize(size) => BackendAttribValue::WindowSize(size),
    }
}

fn from_backend_value(value: BackendAttribValue) -> WlxWindowAttribValue {
    match value {
        BackendAttribValue::Stereo(mode) => WlxWindowAttribValue::Stereo(match mode {
            StereoMode::None => WlxStereoMode::None,
            StereoMode::LeftRight => WlxStereoMode::LeftRight,
            StereoMode::RightLeft => WlxStereoMode::RightLeft,
            StereoMode::TopBottom => WlxStereoMode::TopBottom,
            StereoMode::BottomTop => WlxStereoMode::BottomTop,
        }),
        BackendAttribValue::StereoFullFrame(value) => WlxWindowAttribValue::StereoFullFrame(value),
        BackendAttribValue::StereoAdjustMouse(value) => {
            WlxWindowAttribValue::StereoAdjustMouse(value)
        }
        BackendAttribValue::MouseTransform(transform) => {
            WlxWindowAttribValue::MouseTransform(match transform {
                MouseTransform::Default => WlxMouseTransform::Default,
                MouseTransform::Normal => WlxMouseTransform::Normal,
                MouseTransform::Rotated90 => WlxMouseTransform::Rotated90,
                MouseTransform::Rotated180 => WlxMouseTransform::Rotated180,
                MouseTransform::Rotated270 => WlxMouseTransform::Rotated270,
                MouseTransform::Flipped => WlxMouseTransform::Flipped,
                MouseTransform::Flipped90 => WlxMouseTransform::Flipped90,
                MouseTransform::Flipped180 => WlxMouseTransform::Flipped180,
                MouseTransform::Flipped270 => WlxMouseTransform::Flipped270,
            })
        }
        BackendAttribValue::WindowSize(size) => WlxWindowAttribValue::WindowSize(size),
        // Icon and Resizable are not exposed via IPC
        _ => unreachable!("attribute not exposed via IPC"),
    }
}
