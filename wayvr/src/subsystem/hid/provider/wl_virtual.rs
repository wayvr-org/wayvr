use std::io::Write;
use std::os::fd::AsFd;
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::Context as _;
use glam::Vec2;
use input_linux::sys::{*};
use smithay::reexports::rustix::fs::{memfd_create, MemfdFlags};
use smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;
use smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;
use smithay::reexports::wayland_server::protocol::wl_keyboard::KeymapFormat;
use wayland_client::{delegate_noop, Dispatch, Proxy, QueueHandle};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_pointer::{Axis, AxisSource, ButtonState};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::{WlSeat};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;
use xkbcommon::xkb::{Context, Keymap, CONTEXT_NO_FLAGS, KEYMAP_COMPILE_NO_FLAGS, KEYMAP_FORMAT_TEXT_V1};
use wlx_common::overlays::ToastTopic;
use crate::overlays::toast::Toast;
use crate::subsystem::hid::provider::HidProvider;
use crate::subsystem::hid::{VirtualKey, WheelDelta, *};

pub struct WlVirtualProvider {
    _connection: wayland_client::Connection,
    queue: wayland_client::EventQueue<KbState>,
    state: KbState,

    virtual_pointer: ZwlrVirtualPointerV1,
    desktop_extent: Vec2,
    desktop_origin: Vec2,
    keymap_file: Option<std::fs::File>,

    virtual_keyboard: ZwpVirtualKeyboardV1,
    keyboard_mods_state: u8,
}

struct KbState;

impl HidProvider for WlVirtualProvider {
    fn mouse_move(&mut self, pos: Vec2) {
        #[cfg(debug_assertions)]
        log::trace!("Pointer move: {pos:?}");

        self.virtual_pointer.motion_absolute(
            Self::now_ms(),
            (pos.x - self.desktop_origin.x) as u32,
            (pos.y - self.desktop_origin.y) as u32,
            self.desktop_extent.x as u32,
            self.desktop_extent.y as u32,
        );
        self.virtual_pointer.motion(Self::now_ms(), 0.0, 0.0);
        self.virtual_pointer.frame();
    }

    fn send_button(&mut self, button: u16, down: bool) {
        #[cfg(debug_assertions)]
        log::trace!("Pointer button: {button}, down: {down}");

        self.virtual_pointer.button(
            Self::now_ms(),
            match button {
                i if i == MOUSE_LEFT => BTN_LEFT as u32,
                i if i == MOUSE_RIGHT => BTN_RIGHT as u32,
                i if i == MOUSE_MIDDLE => BTN_MIDDLE as u32,
                _ => panic!("Invalid mouse button: {button}"),
            },
            match down {
                true => ButtonState::Pressed,
                false => ButtonState::Released,
            },
        );
        self.virtual_pointer.frame();
    }

    fn wheel(&mut self, delta: WheelDelta) {
        #[cfg(debug_assertions)]
        log::trace!("Scroll Axis: {delta:?}");

        self.virtual_pointer.axis_source(AxisSource::Wheel);

        if delta.y != 0.0 {
            let steps = -delta.y.round() as i32;
            self.virtual_pointer.axis_discrete(
                Self::now_ms(),
                Axis::VerticalScroll,
                steps as f64 * 15.0,
                steps,
            );
        }
        if delta.x != 0.0 {
            let steps = delta.x.round() as i32;
            self.virtual_pointer.axis_discrete(
                Self::now_ms(),
                Axis::HorizontalScroll,
                steps as f64 * 15.0,
                steps,
            );
        }

        self.virtual_pointer.motion(Self::now_ms(), 0.0, 0.0);
        self.virtual_pointer.frame();
    }

    fn set_desktop_extent(&mut self, extent: Vec2) {
        self.desktop_extent = extent;
    }

    fn set_desktop_origin(&mut self, origin: Vec2) {
        self.desktop_origin = origin;
    }

    fn set_modifiers(&mut self, mods: u8) {
        const LOCKED: u8 = CAPS_LOCK | NUM_LOCK;

        let changed = (self.keyboard_mods_state ^ mods) & !LOCKED;
        for bit in [SHIFT, CTRL, ALT, SUPER] {
            if changed & bit != 0 {
                let down = mods & bit != 0;
                if let Some(kc) = Self::modifier_keycode(bit) {
                    self.virtual_keyboard
                        .key(Self::now_ms(), kc - 8, down as u32);
                }
            }
        }

        let depressed = (mods & !LOCKED) as u32;
        let locked = (mods & LOCKED) as u32;
        self.virtual_keyboard.modifiers(depressed, 0, locked, 0);

        self.keyboard_mods_state = mods;
        self._connection.flush().unwrap();
    }

    fn send_key(&self, key: VirtualKey, down: bool) {
        #[cfg(debug_assertions)]
        log::trace!("Keyboard key: {key:?} ({}), down: {down}", key as u16);

        self.virtual_keyboard.key(
            Self::now_ms(),
            key as u32 - 8,
            match down {
                true => 1,
                false => 0,
            },
        );

        self._connection.flush().unwrap();
    }

    fn set_keymap(&mut self, keymap: &XkbKeymap) {
        #[cfg(debug_assertions)]
        log::trace!(
            "Keyboard keymap: {:?}",
            keymap.inner.layouts().next().unwrap_or("Unknown")
        );

        let mut bytes = keymap
            .inner
            .get_as_string(KEYMAP_FORMAT_TEXT_V1)
            .into_bytes();
        bytes.push(0);
        let fd = memfd_create("virtual-keyboard-keymap", MemfdFlags::CLOEXEC)
            .expect("Failed to create memfd");

        let mut file = std::fs::File::from(fd);
        file.write_all(&bytes).expect("failed to write the keymap");

        self.virtual_keyboard
            .keymap(KeymapFormat::XkbV1 as u32, file.as_fd(), bytes.len() as u32);
        self.queue.roundtrip(&mut self.state).unwrap();
        self.keymap_file.replace(file);
    }

    fn commit(&mut self) {
        self.queue.roundtrip(&mut self.state).unwrap();
    }
}

impl WlVirtualProvider {
    pub fn try_new() -> anyhow::Result<Self> {
        let state = KbState;

        let connection = wayland_client::Connection::connect_to_env()?;
        let (globals, queue) = registry_queue_init::<KbState>(&connection)?;
        let qh = queue.handle();
        let seat: WlSeat = globals
            .bind(&qh, 4..=9, ())
            .context("compositor doesn't expose a compatible wl_seat (version 4..=9)")?;

        let pointer_manager: ZwlrVirtualPointerManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .context("compositor doesn't support zwlr_virtual_pointer_v1")?;

        let virtual_pointer = pointer_manager.create_virtual_pointer(Some(&seat), &qh, ());

        let keyboard_manager: ZwpVirtualKeyboardManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .context("compositor doesn't support zwp_virtual_keyboard_v1")?;

        let virtual_keyboard = keyboard_manager.create_virtual_keyboard(&seat, &qh, ());

        let mut result = Self {
            keymap_file: None,
            _connection: connection,
            queue,
            state,
            virtual_pointer,
            virtual_keyboard,
            desktop_extent: Vec2::ZERO,
            desktop_origin: Vec2::ZERO,
            keyboard_mods_state: 0,
        };

        result.set_keymap(&XkbKeymap {
            inner: Self::default_keymap(),
        });

        Ok(result)
    }

    fn default_keymap() -> Keymap {
        let xkb_context = Context::new(CONTEXT_NO_FLAGS);
        Keymap::new_from_names(
            &xkb_context,
            "",
            "",
            "us",
            "",
            None,
            KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("Failed to compile XKB keymap")
    }

    fn modifier_keycode(bit: u8) -> Option<u32> {
        let evdev = match bit {
            b if b == SHIFT => KEY_LEFTSHIFT,
            b if b == CTRL => KEY_LEFTCTRL,
            b if b == ALT => KEY_LEFTALT,
            b if b == SUPER => KEY_LEFTMETA,
            _ => return None,
        };
        Some(evdev as u32 + 8)
    }

    fn now_ms() -> u32 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u32
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for KbState {
    fn event(
        _state: &mut Self,
        _proxy: &WlRegistry,
        _event: <WlRegistry as Proxy>::Event,
        _data: &GlobalListContents,
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(KbState: ignore WlSeat);
delegate_noop!(KbState: ignore ZwlrVirtualPointerV1);
delegate_noop!(KbState: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(KbState: ignore ZwpVirtualKeyboardV1);
delegate_noop!(KbState: ignore ZwpVirtualKeyboardManagerV1);

pub fn initialize_wl_virtual() -> anyhow::Result<Box<dyn HidProvider>, Toast> {
    let provider = WlVirtualProvider::try_new().map_err(|e| {
        Toast::new(
            ToastTopic::System,
            String::with_capacity(0),
            format!("Could not initialize wl_virtual: {e}"),
        )
    })?;
    Ok(Box::new(provider))
}
