use super::hid::{self, HidProvider, VirtualKey};

use crate::{
    backend::wayvr::WvrServerState,
    overlays::toast::Toast,
    subsystem::hid::{DummyProvider, XkbKeymap}
};

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
        let hid_result = hid::initialize();
        let hid_provider: Box<dyn HidProvider>;
        let error: Option<Toast>;
        match hid_result {
            Ok(uinput) => {
                hid_provider = Box::new(uinput);
                error = None;
            },
            Err(toast) => {
                hid_provider = Box::new(DummyProvider {});
                error = Some(toast);
            }
        }
        (Self {
            keyboard_focus: KeyboardFocus::PhysicalScreen,
            inner: hid_provider,
            keymap: None,
        }, error)
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
