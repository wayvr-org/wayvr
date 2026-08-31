use wayvr_ipc::packet_client::{WlxWindowStateField, WlxWindowStateGetParams, WlxWindowStateValue};
use wlx_common::windowing::OverlayWindowState;

use crate::windowing::{manager::OverlayWindowManager, window::OverlayWindowConfig};

pub fn get_field(state: &OverlayWindowState, field: WlxWindowStateField) -> WlxWindowStateValue {
    match field {
        WlxWindowStateField::Alpha => WlxWindowStateValue::Float(state.alpha),
        WlxWindowStateField::Grabbable => WlxWindowStateValue::Bool(state.grabbable),
        WlxWindowStateField::Interactable => WlxWindowStateValue::Bool(state.interactable),
        WlxWindowStateField::Positioning => {
            WlxWindowStateValue::Positioning(state.positioning.into())
        }
        WlxWindowStateField::Curvature => {
            WlxWindowStateValue::Float(state.curvature.unwrap_or(0.0))
        }
        WlxWindowStateField::Additive => WlxWindowStateValue::Bool(state.additive),
        WlxWindowStateField::BlockInput => WlxWindowStateValue::Bool(state.block_input),
        WlxWindowStateField::AlignToHmd => WlxWindowStateValue::Bool(state.align_to_hmd),
    }
}

pub fn set_field(
    config: &mut OverlayWindowConfig,
    field: WlxWindowStateField,
    value: WlxWindowStateValue,
) {
    let Some(state) = config.active_state.as_mut() else {
        log::warn!(
            "Overlay '{}' is not visible, window state field {field:?} was not modified",
            config.name
        );
        return;
    };

    match (field, value) {
        (WlxWindowStateField::Alpha, WlxWindowStateValue::Float(value)) => {
            if !(0.1..=1.0).contains(&value) {
                log::warn!("Alpha {value} is out of range, clamping to 0.1..1.0");
            }
            state.alpha = value.clamp(0.1, 1.0);
            config.dirty = true;
        }
        (WlxWindowStateField::Grabbable, WlxWindowStateValue::Bool(value)) => {
            state.grabbable = value;
        }
        (WlxWindowStateField::Interactable, WlxWindowStateValue::Bool(value)) => {
            state.interactable = value;
        }
        (WlxWindowStateField::Positioning, WlxWindowStateValue::Positioning(value)) => {
            state.positioning = value.into();
            config.dirty = true;
        }
        (WlxWindowStateField::Curvature, WlxWindowStateValue::Float(value)) => {
            state.curvature = if value < 0.005 { None } else { Some(value) };
            config.dirty = true;
        }
        (WlxWindowStateField::Additive, WlxWindowStateValue::Bool(value)) => {
            state.additive = value;
            config.dirty = true;
        }
        (WlxWindowStateField::BlockInput, WlxWindowStateValue::Bool(value)) => {
            state.block_input = value;
        }
        (WlxWindowStateField::AlignToHmd, WlxWindowStateValue::Bool(value)) => {
            state.align_to_hmd = value;
            config.dirty = true;
        }
        (field, value) => {
            log::warn!("Invalid value {value:?} for window state field {field:?}");
        }
    }
}

pub fn get_state<T>(
    overlays: &mut OverlayWindowManager<T>,
    params: &WlxWindowStateGetParams,
) -> Result<WlxWindowStateValue, String>
where
    T: Default,
{
    let Some(id) = overlays.lookup(&params.overlay) else {
        return Err(format!("overlay '{}' not found", params.overlay));
    };

    let overlay = overlays
        .mut_by_id(id)
        .ok_or_else(|| format!("overlay '{}' not found", params.overlay))?;

    let state = overlay
        .config
        .active_state
        .as_ref()
        .ok_or_else(|| format!("overlay '{}' is not visible", params.overlay))?;

    Ok(get_field(state, params.field))
}
