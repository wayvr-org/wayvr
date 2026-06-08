use super::hid::{self, VirtualKey};

use crate::subsystem::hid::provider::HidProvider;
use crate::subsystem::hid::provider::dummy::DummyProvider;
use crate::{backend::wayvr::WvrServerState, overlays::toast::Toast, subsystem::hid::XkbKeymap};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum KeyboardFocus {
    PhysicalScreen,
    WayVR, // (wayland window id data is handled internally),
}

pub struct HidWrapper {
    pub keyboard_focus: KeyboardFocus,
    pub inner: Box<dyn HidProvider>,
    pub keymap: Option<XkbKeymap>,
}

impl HidWrapper {
    pub fn new() -> (Self, Option<Toast>) {
        let (provider, toast) = hid::provider::wl_virtual::initialize_wl_virtual()
            .or_else(|_| hid::provider::uinput::initialize_uinput())
            .map(|provider| (provider, None))
            .unwrap_or_else(|toast| (Box::new(DummyProvider {}), Some(toast)));

        (
            Self {
                keyboard_focus: KeyboardFocus::PhysicalScreen,
                inner: provider,
                keymap: None,
            },
            toast,
        )
    }

    pub fn send_key_routed(
        &self,
        wvr_server: Option<&mut WvrServerState>,
        key: VirtualKey,
        down: bool,
    ) {
        match self.keyboard_focus {
            KeyboardFocus::PhysicalScreen => self.inner.send_key(key, down),
            KeyboardFocus::WayVR => {
                if let Some(wvr_server) = wvr_server {
                    wvr_server.send_key(key as u32, down);
                }
            }
        }
    }

    pub fn keymap_changed(&mut self, wvr_server: Option<&mut WvrServerState>, keymap: &XkbKeymap) {
        if let Some(wvr_server) = wvr_server {
            let _ = wvr_server
                .set_keymap(&keymap.inner)
                .inspect_err(|e| log::error!("Could not set WayVR keymap: {e:?}"));
        } else {
            self.keymap = Some(keymap.clone());
            self.inner.set_keymap(&keymap);
        }

        log::info!(
            "Keymap changed: {}",
            keymap.inner.layouts().next().unwrap_or("Unknown")
        );
    }

    pub fn set_modifiers_routed(&mut self, wvr_server: Option<&mut WvrServerState>, mods: u8) {
        match self.keyboard_focus {
            KeyboardFocus::PhysicalScreen => self.inner.set_modifiers(mods),
            KeyboardFocus::WayVR => {
                if let Some(wvr_server) = wvr_server {
                    wvr_server.set_modifiers(mods);
                }
            }
        }
    }
}
