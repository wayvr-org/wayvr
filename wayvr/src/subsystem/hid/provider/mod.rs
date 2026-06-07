use glam::Vec2;
use crate::subsystem::hid::{VirtualKey, WheelDelta};

pub mod uinput;
pub mod wl_virtual;
pub mod dummy;

pub trait HidProvider: Sync + Send {
    fn mouse_move(&mut self, pos: Vec2);
    fn send_button(&mut self, button: u16, down: bool);
    fn wheel(&mut self, delta: WheelDelta);
    fn set_modifiers(&mut self, mods: u8);
    fn send_key(&self, key: VirtualKey, down: bool);
    fn set_desktop_extent(&mut self, extent: Vec2);
    fn set_desktop_origin(&mut self, origin: Vec2);
    fn commit(&mut self);
}