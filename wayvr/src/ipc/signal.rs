use crate::backend::wayvr::window;

#[derive(Clone)]
pub enum WayVRSignal {
    DeviceHaptics(usize, crate::backend::input::Haptics),
    SwitchSet(Option<usize>),
    Handsfree(wayvr_ipc::packet_client::HandsfreeParams),
    ShowHide,
    CustomTask(crate::backend::task::ModifyPanelTask),
    WindowVisibilityChanged(window::WindowHandle, bool),
}
