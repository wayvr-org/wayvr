use wlx_common::config::InputEmulationMethod;

use super::hid::{self, VirtualKey};

use crate::subsystem::hid::provider::HidProvider;
use crate::subsystem::hid::provider::dummy::DummyProvider;
use crate::{backend::wayvr::WvrServerState, overlays::toast::Toast, subsystem::hid::XkbKeymap};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum InputFocus {
    PhysicalScreen,
    WayVR, // (wayland window id data is handled internally),
}

pub struct HidWrapper {
    input_focus: InputFocus,
    pub inner: Box<dyn HidProvider>,
    pub keymap: Option<XkbKeymap>,
}

impl HidWrapper {
    pub fn new(method: InputEmulationMethod) -> (Self, Option<Toast>) {
        let maybe_provider = match method {
            InputEmulationMethod::Uinput => hid::provider::uinput::initialize_uinput(),
            InputEmulationMethod::WlVirtual => hid::provider::wl_virtual::initialize_wl_virtual(),
            InputEmulationMethod::None => hid::provider::dummy::initialize_dummy(),
        };

        let (provider, toast) = maybe_provider
            .map(|provider| (provider, None))
            .unwrap_or_else(|toast| (Box::new(DummyProvider {}), Some(toast)));

        (
            Self {
                input_focus: InputFocus::PhysicalScreen,
                inner: provider,
                keymap: None,
            },
            toast,
        )
    }

    pub fn set_input_focus(
        &mut self,
        wvr_server: Option<&mut WvrServerState>,
        value: InputFocus,
    ) -> bool {
        if self.input_focus != value {
            self.input_focus = value;

            if let Some(wvr_server) = wvr_server {
                wvr_server.set_input_focus(matches!(value, InputFocus::WayVR));
            }

            true
        } else {
            false
        }
    }

    pub fn get_input_focus(&self) -> InputFocus {
        self.input_focus
    }

    pub fn send_key_routed(
        &mut self,
        wvr_server: Option<&mut WvrServerState>,
        key: VirtualKey,
        down: bool,
    ) {
        match self.input_focus {
            InputFocus::PhysicalScreen => self.inner.send_key(key, down),
            InputFocus::WayVR => {
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
        match self.input_focus {
            InputFocus::PhysicalScreen => self.inner.set_modifiers(mods),
            InputFocus::WayVR => {
                if let Some(wvr_server) = wvr_server {
                    wvr_server.set_modifiers(mods);
                }
            }
        }
    }
}
