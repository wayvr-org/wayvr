use glam::DVec2;
use idmap::{IdMap, idmap};
use idmap_derive::IntegerId;
use libc::{input_event, timeval};
use serde::Deserialize;
use std::sync::LazyLock;
use strum::{EnumIter, EnumString, FromRepr};
use xkbcommon::xkb;

#[cfg(feature = "wayland")]
pub mod wayland;

pub mod provider;
#[cfg(feature = "x11")]
mod x11;

#[derive(Debug)]
pub struct WheelDelta {
    pub x: f32,
    pub y: f32,
}

struct MouseButtonAction {
    button: u16,
    down: bool,
}

#[derive(Default)]
struct MouseAction {
    last_requested_pos: Option<DVec2>,
    pos: Option<DVec2>,
    button: Option<MouseButtonAction>,
    scroll: Option<WheelDelta>,
}

pub const MOUSE_LEFT: u16 = 0x110;
pub const MOUSE_RIGHT: u16 = 0x111;
pub const MOUSE_MIDDLE: u16 = 0x112;

const MOUSE_EXTENT: f64 = 32768.;

const EV_SYN: u16 = 0x0;
const EV_KEY: u16 = 0x1;
const EV_REL: u16 = 0x2;
const EV_ABS: u16 = 0x3;

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
pub const ALTGR: KeyModifier = 0x80;

#[allow(non_camel_case_types)]
#[repr(u16)]
#[derive(
    Debug, Deserialize, PartialEq, Eq, Clone, Copy, IntegerId, EnumString, EnumIter, FromRepr,
)]
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
    #[strum(serialize = "AltGr", serialize = "Meta")]
    AltGr,
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
        VirtualKey::AltGr => ALTGR,
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
        ALTGR => vec![VirtualKey::AltGr],
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
            xkb::Keycode::from(VirtualKey::AltGr as u32),
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
