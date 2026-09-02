use std::collections::HashMap;

use anyhow::Context;
use serde::Serialize;
use wayvr_ipc::{
    client::{WayVRClient, WayVRClientMutex},
    ipc,
    packet_client::{self, HandsfreeParams, PositionMode},
    packet_server,
};

pub struct WayVRClientState {
    pub wayvr_client: WayVRClientMutex,
    pub serial_generator: ipc::SerialGenerator,
    pub pretty_print: bool,
}

fn handle_empty_result(result: anyhow::Result<()>) {
    if let Err(e) = result {
        log::error!("{e:?}");
    }
}

fn handle_result<T: Serialize>(pretty_print: bool, result: anyhow::Result<T>) {
    match result {
        Ok(t) => {
            let maybe_json = if pretty_print {
                serde_json::to_string_pretty(&t)
            } else {
                serde_json::to_string(&t)
            };

            match maybe_json {
                Ok(json_string) => println!("{}", json_string),
                Err(e) => log::error!("Failed to serialize JSON: {e:?}"),
            }
        }
        Err(e) => log::error!("{e:?}"),
    }
}

pub async fn wlx_overlay_list(state: &mut WayVRClientState, visible: bool, hidden: bool) {
    let result = WayVRClient::fn_wlx_overlay_list(
        state.wayvr_client.clone(),
        state.serial_generator.increment_get(),
        packet_client::WlxOverlayListParams { visible, hidden },
    )
    .await;

    match result {
        Ok(names) => {
            for name in names {
                println!("{name}");
            }
        }
        Err(e) => log::error!("failed to list overlays: {e:?}"),
    }
}

pub async fn wvr_window_set_visible(
    state: &mut WayVRClientState,
    handle: packet_server::WvrWindowHandle,
    visible: bool,
) {
    handle_empty_result(
        WayVRClient::fn_wvr_window_set_visible(state.wayvr_client.clone(), handle, visible)
            .await
            .context("failed to set window visibility"),
    )
}

pub async fn wvr_process_get(
    state: &mut WayVRClientState,
    handle: packet_server::WvrProcessHandle,
) {
    handle_result(
        state.pretty_print,
        WayVRClient::fn_wvr_process_get(
            state.wayvr_client.clone(),
            state.serial_generator.increment_get(),
            handle,
        )
        .await
        .context("failed to get process"),
    );
}

pub async fn wvr_process_list(state: &mut WayVRClientState) {
    handle_result(
        state.pretty_print,
        WayVRClient::fn_wvr_process_list(
            state.wayvr_client.clone(),
            state.serial_generator.increment_get(),
        )
        .await
        .context("failed to list processes"),
    )
}

pub async fn wvr_process_terminate(
    state: &mut WayVRClientState,
    handle: packet_server::WvrProcessHandle,
) {
    handle_empty_result(
        WayVRClient::fn_wvr_process_terminate(state.wayvr_client.clone(), handle)
            .await
            .context("failed to terminate process"),
    )
}

#[allow(clippy::too_many_arguments)]
pub async fn wvr_process_launch(
    state: &mut WayVRClientState,
    exec: String,
    name: String,
    env: Vec<String>,
    resolution: [u32; 2],
    pos_mode: PositionMode,
    icon: Option<String>,
    args: String,
    userdata: HashMap<String, String>,
) {
    handle_result(
        state.pretty_print,
        WayVRClient::fn_wvr_process_launch(
            state.wayvr_client.clone(),
            state.serial_generator.increment_get(),
            packet_client::WvrProcessLaunchParams {
                env,
                exec,
                name,
                args,
                resolution,
                pos_mode,
                icon,
                userdata,
            },
        )
        .await
        .context("failed to launch process"),
    )
}

pub async fn wlr_input_capture(state: &mut WayVRClientState, grab: bool) {
    handle_empty_result(
        WayVRClient::fn_wvr_input_capture(state.wayvr_client.clone(), grab)
            .await
            .context("failed to change input capture"),
    )
}

pub async fn wlx_device_haptics(
    state: &mut WayVRClientState,
    device: usize,
    intensity: f32,
    duration: f32,
    frequency: f32,
) {
    handle_empty_result(
        WayVRClient::fn_wlx_device_haptics(
            state.wayvr_client.clone(),
            device,
            packet_client::WlxHapticsParams {
                intensity,
                duration,
                frequency,
            },
        )
        .await
        .context("failed to trigger haptics"),
    )
}

pub async fn wlx_show_hide(state: &mut WayVRClientState) {
    handle_empty_result(
        WayVRClient::fn_wlx_show_hide(state.wayvr_client.clone())
            .await
            .context("failed to trigger overlay show hide"),
    )
}

pub async fn wlx_switch_set(state: &mut WayVRClientState, set: Option<usize>) {
    handle_empty_result(
        WayVRClient::fn_wlx_switch_set(state.wayvr_client.clone(), set)
            .await
            .context("failed to switch to set"),
    )
}

pub async fn wlx_handsfree(state: &mut WayVRClientState, params: HandsfreeParams) {
    handle_empty_result(
        WayVRClient::fn_wlx_handsfree(state.wayvr_client.clone(), params)
            .await
            .context("failed to set handsfree mode"),
    )
}

pub async fn wlx_panel_modify(
    state: &mut WayVRClientState,
    overlay: String,
    element: String,
    command: packet_client::WlxModifyPanelCommand,
) {
    handle_empty_result(
        WayVRClient::fn_wlx_modify_panel(
            state.wayvr_client.clone(),
            packet_client::WlxModifyPanelParams {
                overlay,
                element,
                command,
            },
        )
        .await
        .context("failed to modify panel"),
    )
}

pub async fn wlx_input_state(state: &mut WayVRClientState) {
    handle_result(
        state.pretty_print,
        WayVRClient::fn_wlx_input_state(
            state.wayvr_client.clone(),
            state.serial_generator.increment_get(),
        )
        .await
        .context("failed to get input state"),
    )
}

pub async fn wlx_window_state_get(
    state: &mut WayVRClientState,
    overlay: String,
    field: packet_client::WlxWindowStateField,
) {
    let result = WayVRClient::fn_wlx_window_state_get(
        state.wayvr_client.clone(),
        state.serial_generator.increment_get(),
        packet_client::WlxWindowStateGetParams { overlay, field },
    )
    .await;

    match result {
        Ok(Ok(value)) => handle_result(state.pretty_print, Ok(window_state_value_to_json(value))),
        Ok(Err(reason)) => log::error!("failed to get window state: {reason}"),
        Err(e) => log::error!("failed to get window state: {e:?}"),
    }
}

fn window_state_value_to_json(value: packet_client::WlxWindowStateValue) -> serde_json::Value {
    match value {
        packet_client::WlxWindowStateValue::Bool(value) => serde_json::Value::Bool(value),
        packet_client::WlxWindowStateValue::Float(value) => serde_json::to_value(value).unwrap(),
        packet_client::WlxWindowStateValue::Positioning(value) => {
            serde_json::json!(window_state_positioning_name(&value))
        }
    }
}

fn window_state_positioning_name(value: &packet_client::WlxPositioning) -> &'static str {
    match value {
        packet_client::WlxPositioning::Floating => "floating",
        packet_client::WlxPositioning::Anchored => "anchored",
        packet_client::WlxPositioning::Static => "static",
        packet_client::WlxPositioning::FollowHead { .. } => "follow_head",
        packet_client::WlxPositioning::FollowHand { hand, .. } => match hand {
            packet_client::WlxHand::Left => "follow_hand_left",
            packet_client::WlxHand::Right => "follow_hand_right",
        },
    }
}

pub async fn wlx_window_state_set(
    state: &mut WayVRClientState,
    overlay: String,
    field: packet_client::WlxWindowStateField,
    value: packet_client::WlxWindowStateValue,
) {
    handle_empty_result(
        WayVRClient::fn_wlx_window_state_set(
            state.wayvr_client.clone(),
            packet_client::WlxWindowStateSetParams {
                overlay,
                field,
                value,
            },
        )
        .await
        .context("failed to set window state"),
    )
}
