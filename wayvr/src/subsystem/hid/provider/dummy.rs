use crate::subsystem::hid::provider::HidProvider;
use crate::subsystem::hid::{VirtualKey, WheelDelta, XkbKeymap};
use glam::Vec2;

pub struct DummyProvider;

impl HidProvider for DummyProvider {
    fn mouse_move(&mut self, _pos: Vec2) {}
    fn send_button(&mut self, _button: u16, _down: bool) {}
    fn wheel(&mut self, _delta: WheelDelta) {}
    fn set_desktop_extent(&mut self, _extent: Vec2) {}
    fn set_desktop_origin(&mut self, _origin: Vec2) {}
    fn set_modifiers(&mut self, _modifiers: u8) {}
    fn send_key(&self, _key: VirtualKey, _down: bool) {}

    fn set_keymap(&mut self, _keymap: &XkbKeymap) {}

    fn commit(&mut self) {}
}
