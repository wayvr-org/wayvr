use crate::subsystem::hid::{VirtualKey, WheelDelta, XkbKeymap};
use glam::DVec2;

pub mod dummy;
pub mod uinput;
pub mod wl_virtual;

pub trait HidProvider: Sync + Send {
    // Pointer Functions
    fn mouse_move(&mut self, pos: DVec2);
    fn mouse_move_rel(&mut self, rel_pos: DVec2);
    fn send_button(&mut self, button: u16, down: bool);
    fn wheel(&mut self, delta: WheelDelta);
    fn set_desktop_extent(&mut self, extent: DVec2);
    fn set_desktop_origin(&mut self, origin: DVec2);

    // Keyboard Functions
    fn set_modifiers(&mut self, mods: u8);
    fn send_key(&mut self, key: VirtualKey, down: bool);
    fn set_keymap(&mut self, keymap: &XkbKeymap);

    // Common Functions
    fn commit(&mut self);
}
