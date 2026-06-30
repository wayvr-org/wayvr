use crate::overlays::toast::Toast;
use crate::subsystem::hid::provider::HidProvider;
use crate::subsystem::hid::{
    EV_ABS, EV_KEY, EV_REL, EV_SYN, MODS_TO_KEYS, MOUSE_EXTENT, MOUSE_LEFT, MOUSE_MIDDLE,
    MouseAction, MouseButtonAction, VirtualKey, WheelDelta, XkbKeymap, get_time, new_event,
};
use glam::Vec2;
use input_linux::{
    AbsoluteAxis, AbsoluteInfo, AbsoluteInfoSetup, EventKind, InputId, Key, RelativeAxis,
    UInputHandle,
};
use std::fs::File;
use std::intrinsics::transmute;
use std::sync::atomic::AtomicBool;
use strum::IntoEnumIterator;
use wlx_common::overlays::ToastTopic;

pub struct UInputProvider {
    keyboard_handle: UInputHandle<File>,
    mouse_handle: UInputHandle<File>,
    desktop_extent: Vec2,
    desktop_origin: Vec2,
    cur_modifiers: u8,
    current_action: MouseAction,
}

impl UInputProvider {
    pub fn try_new() -> Option<Self> {
        let keyboard_file = File::create("/dev/uinput").ok()?;
        let keyboard_handle = UInputHandle::new(keyboard_file);

        let mouse_file = File::create("/dev/uinput").ok()?;
        let mouse_handle = UInputHandle::new(mouse_file);

        let kbd_id = InputId {
            bustype: 0x03,
            vendor: 0x4711,
            product: 0x0829,
            version: 5,
        };
        let mouse_id = InputId {
            bustype: 0x03,
            vendor: 0x4711,
            product: 0x0830,
            version: 5,
        };
        let kbd_name = b"WayVR Keyboard\0";
        let mouse_name = b"WayVR Mouse\0";

        let abs_info = vec![
            AbsoluteInfoSetup {
                axis: input_linux::AbsoluteAxis::X,
                info: AbsoluteInfo {
                    value: 0,
                    minimum: 0,
                    maximum: MOUSE_EXTENT as _,
                    fuzz: 0,
                    flat: 0,
                    resolution: 10,
                },
            },
            AbsoluteInfoSetup {
                axis: input_linux::AbsoluteAxis::Y,
                info: AbsoluteInfo {
                    value: 0,
                    minimum: 0,
                    maximum: MOUSE_EXTENT as _,
                    fuzz: 0,
                    flat: 0,
                    resolution: 10,
                },
            },
        ];

        keyboard_handle.set_evbit(EventKind::Key).ok()?;
        for key in VirtualKey::iter() {
            let mapped_key: Key = unsafe { std::mem::transmute((key as u16) - 8) };
            keyboard_handle.set_keybit(mapped_key).ok()?;
        }

        keyboard_handle.create(&kbd_id, kbd_name, 0, &[]).ok()?;

        mouse_handle.set_evbit(EventKind::Absolute).ok()?;
        mouse_handle.set_evbit(EventKind::Relative).ok()?;
        mouse_handle.set_absbit(AbsoluteAxis::X).ok()?;
        mouse_handle.set_absbit(AbsoluteAxis::Y).ok()?;
        mouse_handle.set_relbit(RelativeAxis::WheelHiRes).ok()?;
        mouse_handle
            .set_relbit(RelativeAxis::HorizontalWheelHiRes)
            .ok()?;
        mouse_handle.set_evbit(EventKind::Key).ok()?;

        for btn in MOUSE_LEFT..=MOUSE_MIDDLE {
            let mouse_btn: Key = unsafe { transmute(btn) };
            mouse_handle.set_keybit(mouse_btn).ok()?;
        }
        mouse_handle
            .create(&mouse_id, mouse_name, 0, &abs_info)
            .ok()?;

        Some(Self {
            keyboard_handle,
            mouse_handle,
            desktop_extent: Vec2::ZERO,
            desktop_origin: Vec2::ZERO,
            current_action: MouseAction::default(),
            cur_modifiers: 0,
        })
    }
    fn send_button_internal(&self, button: u16, down: bool) {
        let time = get_time();
        let events = [
            new_event(time, EV_KEY, button, down.into()),
            new_event(time, EV_SYN, 0, 0),
        ];
        if let Err(res) = self.mouse_handle.write(&events) {
            log::error!("send_button: {res}");
        }
    }
    fn mouse_move_internal(&mut self, pos: Vec2) {
        #[cfg(debug_assertions)]
        log::trace!("Mouse move: {pos:?}");

        let pos = (pos - self.desktop_origin) * (MOUSE_EXTENT / self.desktop_extent);

        let time = get_time();
        let events = [
            new_event(time, EV_ABS, AbsoluteAxis::X as _, pos.x as i32),
            new_event(time, EV_ABS, AbsoluteAxis::Y as _, pos.y as i32),
            new_event(time, EV_SYN, 0, 0),
        ];
        if let Err(res) = self.mouse_handle.write(&events) {
            log::error!("{res}");
        }
    }

    fn wheel_internal(&self, delta: WheelDelta) {
        let multiplier = 64.0; /* cherry-picked value, overall scrolling speed can be altered via `scroll_speed` in the config */
        let delta_x = (delta.x * multiplier) as i32;
        let delta_y = (delta.y * multiplier) as i32;

        let time = get_time();
        let events = [
            new_event(time, EV_REL, RelativeAxis::WheelHiRes as _, delta_y),
            new_event(
                time,
                EV_REL,
                RelativeAxis::HorizontalWheelHiRes as _,
                delta_x,
            ),
            new_event(time, EV_SYN, 0, 0),
        ];
        if let Err(res) = self.mouse_handle.write(&events) {
            log::error!("wheel: {res}");
        }
    }
}

impl HidProvider for UInputProvider {
    fn mouse_move(&mut self, pos: Vec2) {
        if self.current_action.pos.is_none() && self.current_action.scroll.is_none() {
            self.current_action.pos = Some(pos);
        }
        self.current_action.last_requested_pos = Some(pos);
    }
    fn send_button(&mut self, button: u16, down: bool) {
        if self.current_action.button.is_none() {
            self.current_action.button = Some(MouseButtonAction { button, down });
            self.current_action.pos = self.current_action.last_requested_pos;
        }
    }
    fn wheel(&mut self, delta: WheelDelta) {
        if self.current_action.scroll.is_none() {
            self.current_action.scroll = Some(delta);
            // Pass mouse motion events only if not scrolling
            // (allows scrolling on all Chromium-based applications)
            self.current_action.pos = None;
        }
    }
    fn set_desktop_extent(&mut self, extent: Vec2) {
        self.desktop_extent = extent;
    }
    fn set_desktop_origin(&mut self, origin: Vec2) {
        self.desktop_origin = origin;
    }
    fn set_modifiers(&mut self, modifiers: u8) {
        let changed = self.cur_modifiers ^ modifiers;
        for i in 0..8 {
            let m = 1 << i;
            if changed & m != 0
                && let Some(vk) = MODS_TO_KEYS.get(m).into_iter().flatten().next()
            {
                self.send_key(*vk, modifiers & m != 0);
            }
        }
        self.cur_modifiers = modifiers;
    }

    fn send_key(&self, key: VirtualKey, down: bool) {
        #[cfg(debug_assertions)]
        log::trace!("send_key: {key:?} {down}");

        let time = get_time();
        let events = [
            new_event(time, EV_KEY, (key as u16) - 8, down.into()),
            new_event(time, EV_SYN, 0, 0),
        ];
        if let Err(res) = self.keyboard_handle.write(&events) {
            log::error!("send_key: {res}");
        }
    }

    fn set_keymap(&mut self, _keymap: &XkbKeymap) {}

    fn commit(&mut self) {
        if let Some(pos) = self.current_action.pos.take() {
            self.mouse_move_internal(pos);
        }
        if let Some(button) = self.current_action.button.take() {
            self.send_button_internal(button.button, button.down);
        }
        if let Some(scroll) = self.current_action.scroll.take() {
            self.wheel_internal(scroll);
        }
    }
}

pub static USE_UINPUT: AtomicBool = AtomicBool::new(true);

pub fn initialize_uinput() -> Result<Box<dyn HidProvider>, Toast> {
    const CHECK_UINPUT_MESSAGE: &str =
        "Could not create uinput provider. Keyboard/Mouse input will not work!

Check if the uinput kernel module is loaded: lsmod | grep uinput
 - If not loaded, follow your distro's instructions to load the uinput kernel module.

Check if you're in input group, run: id -nG";

    if !USE_UINPUT.load(std::sync::atomic::Ordering::Relaxed) {
        const UINPUT_DISABLED: &str = "Uinput disabled by user.";
        log::info!("{UINPUT_DISABLED}");
        return Err(Toast::new(
            ToastTopic::System,
            String::with_capacity(0),
            String::from(UINPUT_DISABLED),
        )
        .with_timeout(5.0));
    }

    if let Some(uinput) = UInputProvider::try_new() {
        log::info!("Initialized uinput.");
        return Ok(Box::new(uinput));
    }
    let mut full_uinput_error = String::from(CHECK_UINPUT_MESSAGE);
    if let Ok(user) = std::env::var("USER") {
        let check_group_message = format!(
            " - To add yourself to the input group, run: sudo usermod -aG input {user}
 - After adding yourself to the input group, you will need to reboot."
        );
        full_uinput_error.push_str(&check_group_message);
    }
    for error_line in full_uinput_error.lines() {
        if !error_line.is_empty() {
            log::error!("{error_line}");
        }
    }
    Err(Toast::new(
        ToastTopic::Error,
        String::with_capacity(0),
        full_uinput_error,
    )
    .with_timeout(30.0))
}
