use glam::Vec2;
use idmap::{IdMap, idmap};
use idmap_derive::IntegerId;
use input_linux::{
    AbsoluteAxis, AbsoluteInfo, AbsoluteInfoSetup, EventKind, InputId, Key, RelativeAxis,
    UInputHandle,
};
use libc::{input_event, timeval};
use serde::Deserialize;
use std::mem::transmute;
use std::sync::LazyLock;
use std::{fs::File, sync::atomic::AtomicBool};
use strum::{EnumIter, EnumString, IntoEnumIterator};
use xkbcommon::xkb;

#[cfg(feature = "wayland")]
pub mod wayland;

#[cfg(feature = "x11")]
mod x11;

pub static USE_UINPUT: AtomicBool = AtomicBool::new(true);

pub(super) fn initialize() -> Box<dyn HidProvider> {
    if !USE_UINPUT.load(std::sync::atomic::Ordering::Relaxed) {
        log::info!("Uinput disabled by user.");
        return Box::new(DummyProvider {});
    }

    if let Some(uinput) = UInputProvider::try_new() {
        log::info!("Initialized uinput.");
        return Box::new(uinput);
    }
    log::error!("Could not create uinput provider. Keyboard/Mouse input will not work!");
    log::error!("Check if the uinput kernel module is loaded: lsmod | grep uinput");
    log::error!(
        " - If not loaded, follow your distro's instructions to load the uinput kernel module."
    );
    log::error!("Check if you're in input group, run: id -nG");
    if let Ok(user) = std::env::var("USER") {
        log::error!(" - To add yourself to the input group, run: sudo usermod -aG input {user}");
        log::error!(" - After adding yourself to the input group, you will need to reboot.");
    }
    Box::new(DummyProvider {})
}

pub struct WheelDelta {
    pub x: f32,
    pub y: f32,
}

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

struct MouseButtonAction {
    button: u16,
    down: bool,
}

#[derive(Default)]
struct MouseAction {
    last_requested_pos: Option<Vec2>,
    pos: Option<Vec2>,
    button: Option<MouseButtonAction>,
    scroll: Option<WheelDelta>,
}

pub struct UInputProvider {
    keyboard_handle: UInputHandle<File>,
    mouse_handle: UInputHandle<File>,
    desktop_extent: Vec2,
    desktop_origin: Vec2,
    cur_modifiers: u8,
    current_action: MouseAction,
}

pub struct DummyProvider;

pub const MOUSE_LEFT: u16 = 0x110;
pub const MOUSE_RIGHT: u16 = 0x111;
pub const MOUSE_MIDDLE: u16 = 0x112;

const MOUSE_EXTENT: f32 = 32768.;

const EV_SYN: u16 = 0x0;
const EV_KEY: u16 = 0x1;
const EV_REL: u16 = 0x2;
const EV_ABS: u16 = 0x3;

impl UInputProvider {
    fn try_new() -> Option<Self> {
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

    fn set_desktop_extent(&mut self, extent: Vec2) {
        self.desktop_extent = extent;
    }
    fn set_desktop_origin(&mut self, origin: Vec2) {
        self.desktop_origin = origin;
    }
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

impl HidProvider for DummyProvider {
    fn mouse_move(&mut self, _pos: Vec2) {}
    fn send_button(&mut self, _button: u16, _down: bool) {}
    fn wheel(&mut self, _delta: WheelDelta) {}
    fn set_modifiers(&mut self, _modifiers: u8) {}
    fn send_key(&self, _key: VirtualKey, _down: bool) {}
    fn set_desktop_extent(&mut self, _extent: Vec2) {}
    fn set_desktop_origin(&mut self, _origin: Vec2) {}
    fn commit(&mut self) {}
}

#[inline]
fn get_time() -> timeval {
    let mut time = timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    unsafe { libc::gettimeofday(&raw mut time, std::ptr::null_mut()) };
    time
}

#[inline]
const fn new_event(time: timeval, type_: u16, code: u16, value: i32) -> input_event {
    input_event {
        time,
        type_,
        code,
        value,
    }
}

pub type KeyModifier = u8;
pub const SHIFT: KeyModifier = 0x01;
pub const CAPS_LOCK: KeyModifier = 0x02;
pub const CTRL: KeyModifier = 0x04;
pub const ALT: KeyModifier = 0x08;
pub const NUM_LOCK: KeyModifier = 0x10;
pub const SUPER: KeyModifier = 0x40;
pub const META: KeyModifier = 0x80;

#[allow(non_camel_case_types)]
#[repr(u16)]
#[derive(Debug, Deserialize, PartialEq, Eq, Clone, Copy, IntegerId, EnumString, EnumIter)]
#[derive(Hash)]
pub enum VirtualKey {
    Escape = 9,
    N1, // number row
    N2,
    N3,
    N4,
    N5,
    N6,
    N7,
    N8,
    N9,
    N0,
    Minus,
    Plus,
    BackSpace,
    Tab,
    Q,
    W,
    E,
    R,
    T,
    Y,
    U,
    I,
    O,
    P,
    Oem4, // [ {
    Oem6, // ] }
    Return,
    LCtrl,
    A,
    S,
    D,
    F,
    G,
    H,
    J,
    K,
    L,
    Oem1, // ; :
    Oem7, // ' "
    Oem3, // ` ~
    LShift,
    Oem5, // \ |
    Z,
    X,
    C,
    V,
    B,
    N,
    M,
    Comma,  // , <
    Period, // . >
    Oem2,   // / ?
    RShift,
    KP_Multiply,
    LAlt,
    Space,
    Caps,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    NumLock,
    Scroll,
    KP_7, // KeyPad
    KP_8,
    KP_9,
    KP_Subtract,
    KP_4,
    KP_5,
    KP_6,
    KP_Add,
    KP_1,
    KP_2,
    KP_3,
    KP_0,
    KP_Decimal,
    Oem102 = 94, // Optional key usually between LShift and Z
    F11,
    F12,
    AbntC1,
    Katakana,
    Hiragana,
    Henkan,
    Kana,
    Muhenkan,
    KP_Enter = 104,
    RCtrl,
    KP_Divide,
    Print,
    Meta, // Right Alt aka AltGr
    Home = 110,
    Up,
    Prior,
    Left,
    Right,
    End,
    Down,
    Next,
    Insert,
    Delete,
    XF86AudioMute = 121,
    XF86AudioLowerVolume,
    XF86AudioRaiseVolume,
    Pause = 127,
    AbntC2 = 129,
    Hangul,
    Hanja,
    LSuper = 133,
    RSuper,
    Menu,
    Help = 146,
    XF86MenuKB,
    XF86Sleep = 150,
    XF86Xfer = 155,
    XF86Launch1,
    XF86Launch2,
    XF86WWW,
    XF86Mail = 163,
    XF86Favorites,
    XF86MyComputer,
    XF86Back,
    XF86Forward,
    XF86AudioNext = 171,
    XF86AudioPlay,
    XF86AudioPrev,
    XF86AudioStop,
    XF86HomePage = 180,
    XF86Reload,
    F13 = 191,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
    Hyper = 207,
    XF86Launch3,
    XF86Launch4,
    XF86LaunchB,
    XF86Search = 225,
}

pub static KEYS_TO_MODS: LazyLock<IdMap<VirtualKey, KeyModifier>> = LazyLock::new(|| {
    idmap! {
        VirtualKey::LShift => SHIFT,
        VirtualKey::RShift => SHIFT,
        VirtualKey::Caps => CAPS_LOCK,
        VirtualKey::LCtrl => CTRL,
        VirtualKey::RCtrl => CTRL,
        VirtualKey::LAlt => ALT,
        VirtualKey::NumLock => NUM_LOCK,
        VirtualKey::LSuper => SUPER,
        VirtualKey::RSuper => SUPER,
        VirtualKey::Meta => META,
    }
});

pub static MODS_TO_KEYS: LazyLock<IdMap<KeyModifier, Vec<VirtualKey>>> = LazyLock::new(|| {
    idmap! {
        SHIFT => vec![VirtualKey::LShift, VirtualKey::RShift],
        CAPS_LOCK => vec![VirtualKey::Caps],
        CTRL => vec![VirtualKey::LCtrl, VirtualKey::RCtrl],
        ALT => vec![VirtualKey::LAlt],
        NUM_LOCK => vec![VirtualKey::NumLock],
        SUPER => vec![VirtualKey::LSuper, VirtualKey::RSuper],
        META => vec![VirtualKey::Meta],
    }
});

pub enum KeyType {
    Symbol,
    NumPad,
    Special,
    Other,
}

macro_rules! key_between {
    ($key:expr, $start:expr, $end:expr) => {
        $key as u32 >= $start as u32 && $key as u32 <= $end as u32
    };
}

macro_rules! key_is {
    ($key:expr, $val:expr) => {
        $key as u32 == $val as u32
    };
}

pub const fn get_key_type(key: VirtualKey) -> KeyType {
    if key_between!(key, VirtualKey::N1, VirtualKey::Plus)
        || key_between!(key, VirtualKey::Q, VirtualKey::Oem6)
        || key_between!(key, VirtualKey::A, VirtualKey::Oem3)
        || key_between!(key, VirtualKey::Oem5, VirtualKey::Oem2)
        || key_is!(key, VirtualKey::Oem102)
    {
        KeyType::Symbol
    } else if key_between!(key, VirtualKey::KP_7, VirtualKey::KP_0)
        && !key_is!(key, VirtualKey::KP_Subtract)
        && !key_is!(key, VirtualKey::KP_Add)
    {
        KeyType::NumPad
    } else if matches!(
        key,
        VirtualKey::BackSpace
            | VirtualKey::Down
            | VirtualKey::Left
            | VirtualKey::Menu
            | VirtualKey::Return
            | VirtualKey::KP_Enter
            | VirtualKey::Right
            | VirtualKey::LShift
            | VirtualKey::RShift
            | VirtualKey::LSuper
            | VirtualKey::RSuper
            | VirtualKey::Tab
            | VirtualKey::Up
    ) {
        KeyType::Special
    } else {
        KeyType::Other
    }
}

#[derive(Clone)]
pub struct XkbKeymap {
    pub inner: xkb::Keymap,
}

impl XkbKeymap {
    pub fn from_layout_variant(layout: &str, variant: &str) -> Option<Self> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            layout,
            variant,
            None,
            xkb::COMPILE_NO_FLAGS,
        )
        .map(|inner| Self { inner })
    }

    pub fn get_name(&self) -> Option<&str> {
        self.inner.layouts().next()
    }

    pub fn label_for_key(&self, key: VirtualKey, modifier: KeyModifier) -> String {
        let mut state = xkb::State::new(&self.inner);
        if modifier > 0
            && let Some(mod_key) = MODS_TO_KEYS.get(modifier)
        {
            state.update_key(
                xkb::Keycode::from(mod_key[0] as u32),
                xkb::KeyDirection::Down,
            );
        }
        state.key_get_utf8(xkb::Keycode::from(key as u32))
    }

    pub fn has_altgr(&self) -> bool {
        let state0 = xkb::State::new(&self.inner);
        let mut state1 = xkb::State::new(&self.inner);
        state1.update_key(
            xkb::Keycode::from(VirtualKey::Meta as u32),
            xkb::KeyDirection::Down,
        );

        for key in [
            VirtualKey::N0,
            VirtualKey::N1,
            VirtualKey::N2,
            VirtualKey::N3,
            VirtualKey::N4,
            VirtualKey::N5,
            VirtualKey::N6,
            VirtualKey::N7,
            VirtualKey::N8,
            VirtualKey::N9,
        ] {
            let sym0 = state0.key_get_one_sym(xkb::Keycode::from(key as u32));
            let sym1 = state1.key_get_one_sym(xkb::Keycode::from(key as u32));
            if sym0 != sym1 {
                return true;
            }
        }
        false
    }
}

#[cfg(feature = "wayland")]
pub use wayland::get_keymap_wl;

#[cfg(not(feature = "wayland"))]
pub fn get_keymap_wl() -> anyhow::Result<XkbKeymap> {
    anyhow::bail!("Wayland support not enabled.")
}

#[cfg(feature = "x11")]
pub use x11::get_keymap_x11;

#[cfg(not(feature = "x11"))]
pub fn get_keymap_x11() -> anyhow::Result<XkbKeymap> {
    anyhow::bail!("X11 support not enabled.")
}
