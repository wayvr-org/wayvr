use crate::overlays::toast::Toast;
use crate::subsystem::hid::provider::HidProvider;
use crate::subsystem::hid::{VirtualKey, WheelDelta, XkbKeymap};
use glam::DVec2;

pub struct DummyProvider;

impl HidProvider for DummyProvider {
    fn mouse_move(&mut self, _pos: DVec2) {}
    fn mouse_move_rel(&mut self, _pos: DVec2) {}
    fn send_button(&mut self, _button: u16, _down: bool) {}
    fn wheel(&mut self, _delta: WheelDelta) {}
    fn set_desktop_extent(&mut self, _extent: DVec2) {}
    fn set_desktop_origin(&mut self, _origin: DVec2) {}
    fn set_modifiers(&mut self, _modifiers: u8) {}
    fn send_key(&self, _key: VirtualKey, _down: bool) {}

    fn set_keymap(&mut self, _keymap: &XkbKeymap) {}

    fn commit(&mut self) {}
}

pub fn initialize_dummy() -> anyhow::Result<Box<dyn HidProvider>, Toast> {
    Ok(Box::new(DummyProvider {}))
}
