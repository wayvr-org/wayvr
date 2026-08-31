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
    SetWindowState(wayvr_ipc::packet_client::WlxWindowStateSetParams),
}
