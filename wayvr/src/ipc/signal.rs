#[derive(Clone)]
pub enum WayVRSignal {
    BroadcastStateChanged(wayvr_ipc::packet_server::WvrStateChanged),
    DeviceHaptics(usize, crate::backend::input::Haptics),
    SwitchSet(Option<usize>),
    ShowHide,
    CustomTask(crate::backend::task::ModifyPanelTask),
    ScreenFocusToggle(crate::backend::task::ScreenFocusTask),
}
