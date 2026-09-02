use crate::backend::wayvr::window;
use wayvr_ipc::ipc::Serial;

#[derive(Clone)]
pub enum WayVRSignal {
    DeviceHaptics(usize, crate::backend::input::Haptics),
    SwitchSet(Option<usize>),
    Handsfree(wayvr_ipc::packet_client::HandsfreeParams),
    ShowHide,
    CustomTask(crate::backend::task::ModifyPanelTask),
    WindowVisibilityChanged(window::WindowHandle, bool),
    // (connection id, serial, params)
    GetWindowState(
        u64,
        Serial,
        wayvr_ipc::packet_client::WlxWindowStateGetParams,
    ),
    // (connection id, serial, params)
    GetWindowAttrib(
        u64,
        Serial,
        wayvr_ipc::packet_client::WlxWindowAttribGetParams,
    ),
    // (connection id, serial, params)
    SetWindowAttrib(
        u64,
        Serial,
        wayvr_ipc::packet_client::WlxWindowAttribSetParams,
    ),
    // (connection id, serial, params)
    ListOverlays(u64, Serial, wayvr_ipc::packet_client::WlxOverlayListParams),
    SetWindowState(wayvr_ipc::packet_client::WlxWindowStateSetParams),
    SetOverlayVisible(String, bool),
}
