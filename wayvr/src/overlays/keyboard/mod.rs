use std::{
    cell::Cell,
    collections::HashMap,
    process::{Child, Command},
    sync::{Arc, atomic::Ordering},
};

use crate::overlays::keyboard::layout::KeyCapType;
use crate::overlays::keyboard::swipe_type::{PredictionSlot, SwipeTypingManager};
use crate::overlays::toast::Toast;
use crate::{
    KEYMAP_CHANGE,
    backend::{
        input::{HoverResult, PointerHit},
        task::{GlobalChange, OverlayTask, TaskType},
    },
    config::none_if_0,
    gui::panel::{GuiPanel, overlay_list::OverlayList, set_list::SetList},
    overlays::keyboard::builder::create_keyboard_panel,
    state::AppState,
    subsystem::{
        dbus::DbusConnector,
        hid::{
            ALT, ALTGR, CTRL, KeyModifier, SHIFT, SUPER, VirtualKey, WheelDelta, XkbKeymap,
            get_keymap_wl, get_keymap_x11,
        },
    },
    windowing::{
        backend::{FrameMeta, OverlayBackend, OverlayEventData, RenderResources, ShouldRender},
        window::{OverlayCategory, OverlayWindowConfig},
    },
};
use anyhow::Context;
use glam::{Affine3A, Quat, Vec2, Vec3, vec3};
use regex::Regex;
use slotmap::{SlotMap, new_key_type};
use smallvec::SmallVec;
use wgui::event::DeviceBitmask;
use wgui::{
    color::WguiColor,
    event::{InternalStateChangeEvent, MouseButtonEvent, MouseButtonIndex},
    i18n::Translation,
    layout::WidgetID,
};
#[cfg(feature = "swipe-to-type")]
use wlx_common::data_dir;
use wlx_common::windowing::{OverlayWindowState, Positioning};
use wlx_common::{
    config::AltModifier,
    overlays::{BackendAttrib, BackendAttribValue, ToastTopic},
};

pub mod builder;
mod layout;
mod prediction_bar;

#[cfg(feature = "swipe-to-type")]
mod swipe_type;

#[cfg(not(feature = "swipe-to-type"))]
mod swipe_type {
    use glam::Vec2;
    use wgui::event::{DeviceBitmask, MouseButtonIndex};

    pub struct SwipeTypingManager;

    #[derive(Clone, Default)]
    pub struct PredictionSlot;

    #[allow(dead_code)] // stub used only to satisfy compilation when the feature is disabled
    impl PredictionSlot {
        pub fn new() -> Self {
            Self
        }
        pub fn set(&self, _value: Option<Vec<String>>) {}
        pub fn take(&self) -> Option<Option<Vec<String>>> {
            None
        }
    }

    #[allow(dead_code)] // stub used only to satisfy compilation when the feature is disabled
    impl SwipeTypingManager {
        pub fn new(_model_folder: std::path::PathBuf) -> anyhow::Result<(Self, PredictionSlot)> {
            Ok((Self, PredictionSlot))
        }
        pub fn add_swipe(
            &mut self,
            _within_key_pos_normalized: &Vec2,
            _key_label: char,
            _device: DeviceBitmask,
            _index: Option<MouseButtonIndex>,
        ) {
        }
        pub fn predict(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        pub fn reset(&mut self) {}
        pub fn did_swipe_leave_first_key(&self) -> bool {
            false
        }
        pub fn is_current_swipe_empty(&self) -> bool {
            true
        }
        pub fn current_swipe_mouse_button_index(&self) -> Option<MouseButtonIndex> {
            None
        }
        pub fn handle_key_press(
            &mut self,
            _key_cap_type: &super::layout::KeyCapType,
            _within_key_pos: &Option<Vec2>,
            _key_label: &[String],
            _device: DeviceBitmask,
            _index: MouseButtonIndex,
        ) -> super::KeyPressOutcome {
            super::KeyPressOutcome::Dispatch
        }
        pub fn handle_key_motion(
            &mut self,
            _key_cap_type: &super::layout::KeyCapType,
            _within_key_pos: &Option<Vec2>,
            _key_label: &[String],
            _device: DeviceBitmask,
        ) {
        }
        pub fn handle_key_release(
            &mut self,
            _key_cap_type: &super::layout::KeyCapType,
            _alt_modifier: crate::subsystem::hid::KeyModifier,
        ) -> super::KeyReleaseOutcome {
            super::KeyReleaseOutcome::Normal
        }
        pub fn select_word(
            &mut self,
            _word: &String,
            _app: &mut crate::state::AppState,
            _original_keyboard_mods: crate::subsystem::hid::KeyModifier,
        ) {
        }
        pub fn select_alternate_prediction(
            &mut self,
            _word: &String,
            _app: &mut crate::state::AppState,
            _original_keyboard_mods: crate::subsystem::hid::KeyModifier,
        ) {
        }
    }
}

pub const KEYBOARD_NAME: &str = "kbd";
const AUTO_RELEASE_MODS: [KeyModifier; 5] = [SHIFT, CTRL, ALT, SUPER, ALTGR];

/// Outcome of handling a key press (or motion) against the swipe-to-type engine.
#[allow(dead_code)]
pub enum KeyPressOutcome {
    /// The event was fed into swipe tracking. Do not dispatch a key.
    Consumed,
    /// Not a swipe target. Dispatch as a normal key event.
    Dispatch,
}

/// Outcome of handling a key release against the swipe-to-type engine.
#[allow(dead_code)]
pub enum KeyReleaseOutcome {
    /// A swipe left the first key; prediction was already triggered. Nothing to dispatch.
    Predict,
    /// Pressed and released on the same key. Dispatch a tap with the given modifier.
    TapKey { modifier: KeyModifier },
    /// Not a swipe target. Dispatch a normal key-up.
    Normal,
}
const SYSTEM_LAYOUT_ALIASES: [&str; 5] = ["mozc", "pinyin", "hangul", "sayura", "unikey"];

pub fn create_keyboard(app: &mut AppState) -> anyhow::Result<OverlayWindowConfig> {
    let layout = layout::Layout::load_from_disk();

    let auto_labels = layout.auto_labels.unwrap_or(true);

    let width = layout.row_size * 0.05 * app.session.config.keyboard_scale;

    let mut maybe_keymap = KeyboardBackend::get_initial_keymap(app).ok();

    if let Some(keymap) = maybe_keymap.as_ref() {
        app.hid_provider
            .keymap_changed(app.wvr_server.as_mut(), keymap);
    }

    if !auto_labels {
        maybe_keymap = None;
    }

    let default_state = KeyboardState {
        modifiers: 0,
        alt_modifier: alt_modifier_to_key(app.session.config.keyboard_middle_click_mode),
        processes: vec![],
        overlay_list: OverlayList::default(),
        set_list: SetList::default(),
        clock_12h: app.session.config.clock_12h,
        keymap_switch_layouts: app.session.config.keyboard_layouts.clone(),
        keymap_switch_index: 0,
        keymap_switch_pending: false,
        swipe_typing_manager: None,
        swipe_candidate_slot: None,
    };

    let mut backend = KeyboardBackend {
        layout_panels: SlotMap::default(),
        layout_ids: HashMap::default(),
        active_layout: KeyboardPanelKey::default(),
        default_state,
        wlx_layout: layout,
        wayland: app.feats.desktop_backend.is_wayland(),
        re_fcitx: Regex::new(r"^keyboard-([^-]+)(?:-([^-]+))?$").unwrap(),
        re_keymap: Regex::new(r"^([a-zA-Z][a-zA-Z0-9]*)(?:\(([^)]+)\))?$").unwrap(),
    };

    backend.active_layout = backend.add_new_keymap(maybe_keymap.as_ref(), app)?;

    Ok(OverlayWindowConfig {
        name: KEYBOARD_NAME.into(),
        category: OverlayCategory::Keyboard,
        default_state: OverlayWindowState {
            grabbable: true,
            positioning: Positioning::Anchored,
            interactable: true,
            curvature: none_if_0(app.session.config.default_curvature),
            alpha: app.session.config.default_opacity,
            transform: Affine3A::from_scale_rotation_translation(
                Vec3::ONE * width,
                Quat::from_rotation_x(-10f32.to_radians()),
                vec3(0.0, -0.69, -0.5),
            ),
            ..OverlayWindowState::default()
        },
        ..OverlayWindowConfig::from_backend(Box::new(backend))
    })
}
#[cfg(feature = "swipe-to-type")]
pub(self) fn init_swipe_type_manager(state: &mut KeyboardState, model_path: std::path::PathBuf) {
    match SwipeTypingManager::new(model_path) {
        Ok((engine, slot)) => {
            state.swipe_typing_manager = Some(engine);
            state.swipe_candidate_slot = Some(slot);
        }
        Err(e) => {
            log::error!("Error occurred while trying to load swipe engine: {}", e);
        }
    };
}
const fn alt_modifier_to_key(m: AltModifier) -> KeyModifier {
    match m {
        AltModifier::Shift => SHIFT,
        AltModifier::Ctrl => CTRL,
        AltModifier::Alt => ALT,
        AltModifier::Super => SUPER,
        AltModifier::AltGr => ALTGR,
        _ => 0,
    }
}

new_key_type! {
    struct KeyboardPanelKey;
}

struct KeyboardBackend {
    layout_panels: SlotMap<KeyboardPanelKey, GuiPanel<KeyboardState>>,
    layout_ids: HashMap<String, KeyboardPanelKey>,
    active_layout: KeyboardPanelKey,
    default_state: KeyboardState,
    wlx_layout: layout::Layout,
    wayland: bool,
    re_fcitx: Regex,
    re_keymap: Regex,
}

impl KeyboardBackend {
    fn add_new_keymap(
        &mut self,
        keymap: Option<&XkbKeymap>,
        app: &mut AppState,
    ) -> anyhow::Result<KeyboardPanelKey> {
        let mut state = self.default_state.take();

        if app.session.config.keyboard_swipe_to_type_enabled {
            #[cfg(feature = "swipe-to-type")]
            init_swipe_type_manager(&mut state, data_dir::get_path("swipe_type").join("en.tar"));
            log::info!("swipe engine created");
        }

        let mut panel = create_keyboard_panel(app, keymap, state, &self.wlx_layout)?;

        if !app.session.config.keyboard_swipe_to_type_enabled {
            prediction_bar::set_visible(&mut panel, false);
        }

        let id = self.layout_panels.insert(panel);
        if let Some(layout_name) = keymap.and_then(|k| k.get_name()) {
            self.layout_ids.insert(layout_name.into(), id);
        } else {
            log::error!("XKB keymap without a layout!");
        }
        Ok(id)
    }

    fn switch_keymap(&mut self, keymap: &XkbKeymap, app: &mut AppState) -> anyhow::Result<bool> {
        if !self.wlx_layout.auto_labels.unwrap_or(true) {
            return Ok(false);
        }

        let Some(layout_name) = keymap.get_name() else {
            log::error!("XKB keymap without a layout!");
            return Ok(false);
        };

        if let Some(new_key) = self.layout_ids.get(layout_name) {
            if self.active_layout.eq(new_key) {
                return Ok(false);
            }
            self.internal_switch_keymap(*new_key, app);
        } else {
            let new_key = self.add_new_keymap(Some(keymap), app)?;
            self.internal_switch_keymap(new_key, app);
        }
        app.tasks
            .enqueue(TaskType::Overlay(OverlayTask::GlobalChange(
                GlobalChange::Keyboard,
            )));
        Ok(true)
    }

    fn internal_switch_keymap(&mut self, new_key: KeyboardPanelKey, app: &AppState) {
        let mut state_from = self
            .layout_panels
            .get_mut(self.active_layout)
            .unwrap()
            .state
            .take();

        if app.session.config.keyboard_swipe_to_type_enabled {
            #[cfg(feature = "swipe-to-type")]
            init_swipe_type_manager(
                &mut state_from,
                data_dir::get_path("swipe_type").join("en.tar"),
            );
        }

        self.active_layout = new_key;

        self.layout_panels
            .get_mut(self.active_layout)
            .unwrap()
            .state = state_from;

        if !app.session.config.keyboard_swipe_to_type_enabled {
            prediction_bar::set_visible(
                self.layout_panels.get_mut(self.active_layout).unwrap(),
                false,
            );
        }
    }

    fn get_effective_keymap(&mut self) -> anyhow::Result<XkbKeymap> {
        fn get_system_keymap(wayland: bool) -> anyhow::Result<XkbKeymap> {
            if wayland {
                get_keymap_wl()
            } else {
                get_keymap_x11()
            }
        }

        let Ok(fcitx_layout) = DbusConnector::fcitx_keymap()
            .context("Could not keymap via fcitx5, falling back to wayland")
            .inspect_err(|e| log::info!("{e:?}"))
        else {
            return get_system_keymap(self.wayland);
        };

        if let Some(captures) = self.re_fcitx.captures(&fcitx_layout) {
            XkbKeymap::from_layout_variant(
                captures.get(1).map_or("", |g| g.as_str()),
                captures.get(2).map_or("", |g| g.as_str()),
            )
            .context("layout/variant is invalid")
        } else if SYSTEM_LAYOUT_ALIASES.contains(&fcitx_layout.as_str()) {
            log::debug!("{fcitx_layout} is an IME, switching to system layout.");
            get_system_keymap(self.wayland)
        } else {
            log::warn!("Unknown layout or IME '{fcitx_layout}', using system layout");
            get_system_keymap(self.wayland)
        }
    }

    fn update_swipe_prediction_bar(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        if prediction_bar::update(self.panel(), app)? {
            self.panel().process_custom_elems(app);
        }
        Ok(())
    }

    fn switch_keymap_by_name(
        &mut self,
        keymap_name: &str,
        app: &mut AppState,
    ) -> anyhow::Result<bool> {
        let Some(captures) = self.re_keymap.captures(keymap_name) else {
            anyhow::bail!("invalid layout name for keymap switch: {keymap_name}");
        };
        let layout = captures.get(1).map_or("", |g| g.as_str());
        let variant = captures.get(2).map_or("", |g| g.as_str());
        let keymap = XkbKeymap::from_layout_variant(layout, variant)
            .context("invalid layout/variant for keymap switch")?;
        app.hid_provider
            .keymap_changed(app.wvr_server.as_mut(), &keymap);
        self.switch_keymap(&keymap, app)
    }

    fn get_initial_keymap(app: &AppState) -> anyhow::Result<XkbKeymap> {
        fn get_system_keymap(wayland: bool) -> anyhow::Result<XkbKeymap> {
            if wayland {
                get_keymap_wl()
            } else {
                get_keymap_x11()
            }
        }

        let Ok(fcitx_layout) = DbusConnector::fcitx_keymap()
            .context("Could not keymap via fcitx5, falling back to wayland")
            .inspect_err(|e| log::info!("{e:?}"))
        else {
            return get_system_keymap(app.feats.desktop_backend.is_wayland());
        };

        let re = Regex::new(r"^keyboard-([^-]+)(?:-([^-]+))?$").unwrap();
        if let Some(captures) = re.captures(&fcitx_layout) {
            XkbKeymap::from_layout_variant(
                captures.get(1).map_or("", |g| g.as_str()),
                captures.get(2).map_or("", |g| g.as_str()),
            )
            .context("layout/variant is invalid")
        } else if SYSTEM_LAYOUT_ALIASES.contains(&fcitx_layout.as_str()) {
            log::debug!("{fcitx_layout} is an IME, switching to system layout.");
            get_system_keymap(app.feats.desktop_backend.is_wayland())
        } else {
            log::warn!("Unknown layout or IME '{fcitx_layout}', using system layout");
            get_system_keymap(app.feats.desktop_backend.is_wayland())
        }
    }

    fn auto_switch_keymap(&mut self, app: &mut AppState) -> anyhow::Result<bool> {
        let keymap = self.get_effective_keymap()?;
        app.hid_provider
            .keymap_changed(app.wvr_server.as_mut(), &keymap);
        self.switch_keymap(&keymap, app)
    }

    fn panel(&mut self) -> &mut GuiPanel<KeyboardState> {
        self.layout_panels.get_mut(self.active_layout).unwrap() // want panic
    }
}

impl OverlayBackend for KeyboardBackend {
    fn init(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        self.panel().init(app)
    }
    fn should_render(&mut self, app: &mut AppState) -> anyhow::Result<ShouldRender> {
        while KEYMAP_CHANGE.swap(false, Ordering::Relaxed) {
            if self
                .auto_switch_keymap(app)
                .inspect_err(|e| log::warn!("{e:?}"))
                .unwrap_or(false)
            {
                let panel = self.panel();
                if !panel.initialized {
                    panel.init(app)?;
                }
                return Ok(match panel.should_render(app)? {
                    ShouldRender::Should | ShouldRender::Can => ShouldRender::Should,
                    ShouldRender::Unable => ShouldRender::Unable,
                });
            }
        }

        if self.panel().state.keymap_switch_pending {
            self.panel().state.keymap_switch_pending = false;
            let layouts = self.panel().state.keymap_switch_layouts.clone();
            let index = self.panel().state.keymap_switch_index;
            if self
                .switch_keymap_by_name(&layouts[index], app)
                .inspect_err(|e| log::warn!("{e:?}"))
                .unwrap_or(false)
            {
                let panel = self.panel();
                if !panel.initialized {
                    panel.init(app)?;
                }
                return Ok(match panel.should_render(app)? {
                    ShouldRender::Should | ShouldRender::Can => ShouldRender::Should,
                    ShouldRender::Unable => ShouldRender::Unable,
                });
            }
        }

        self.update_swipe_prediction_bar(app)?;
        self.panel().should_render(app)
    }
    fn render(&mut self, app: &mut AppState, rdr: &mut RenderResources) -> anyhow::Result<()> {
        self.panel().render(app, rdr)
    }
    fn frame_meta(&mut self) -> Option<FrameMeta> {
        self.panel().frame_meta()
    }
    fn pause(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        self.panel().state.modifiers = 0;
        app.hid_provider
            .set_modifiers_routed(app.wvr_server.as_mut(), 0);
        self.panel().pause(app)
    }
    fn resume(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        self.panel().resume(app)?;
        self.panel().push_event(
            app,
            &wgui::event::Event::InternalStateChange(InternalStateChangeEvent { metadata: 0 }),
        );
        Ok(())
    }

    fn notify(&mut self, app: &mut AppState, event_data: OverlayEventData) -> anyhow::Result<()> {
        self.panel().notify(app, event_data)
    }

    fn on_pointer(&mut self, app: &mut AppState, hit: &PointerHit, pressed: bool) {
        self.panel().on_pointer(app, hit, pressed);
        self.panel().push_event(
            app,
            &wgui::event::Event::InternalStateChange(InternalStateChangeEvent { metadata: 0 }),
        );
    }
    fn on_scroll(&mut self, app: &mut AppState, hit: &PointerHit, delta: WheelDelta) {
        self.panel().on_scroll(app, hit, delta);
    }
    fn on_left(&mut self, app: &mut AppState, pointer: usize) {
        self.panel().on_left(app, pointer);
    }
    fn on_hover(&mut self, app: &mut AppState, hit: &PointerHit) -> HoverResult {
        self.panel().on_hover(app, hit)
    }
    fn get_interaction_transform(&mut self) -> Option<glam::Affine2> {
        self.panel().get_interaction_transform()
    }
    fn get_attrib(&self, _attrib: BackendAttrib) -> Option<BackendAttribValue> {
        None
    }
    fn set_attrib(&mut self, _app: &mut AppState, _value: BackendAttribValue) -> bool {
        false
    }
}

struct KeyboardState {
    modifiers: KeyModifier,
    alt_modifier: KeyModifier,
    processes: Vec<Child>,
    overlay_list: OverlayList,
    set_list: SetList,
    clock_12h: bool,
    keymap_switch_layouts: Vec<Arc<str>>,
    keymap_switch_index: usize,
    keymap_switch_pending: bool,
    swipe_typing_manager: Option<SwipeTypingManager>,
    swipe_candidate_slot: Option<PredictionSlot>,
}

macro_rules! take_and_leave_default {
    ($what:expr) => {{
        let mut x = Default::default();
        std::mem::swap(&mut x, $what);
        x
    }};
}

impl KeyboardState {
    fn take(&mut self) -> Self {
        Self {
            modifiers: self.modifiers,
            alt_modifier: self.alt_modifier,
            processes: take_and_leave_default!(&mut self.processes),
            overlay_list: OverlayList::default(),
            set_list: SetList::default(),
            clock_12h: self.clock_12h,
            keymap_switch_layouts: std::mem::take(&mut self.keymap_switch_layouts),
            keymap_switch_index: self.keymap_switch_index,
            keymap_switch_pending: false,
            swipe_typing_manager: None,
            swipe_candidate_slot: None,
        }
    }
}

fn play_key_click(app: &mut AppState) {
    app.audio_sample_player
        .play_sample(&mut app.audio_system, "key_click");
}

struct ChildWidget {
    id: WidgetID,
    base_color: WguiColor,
}

struct KeyState {
    button_state: KeyButtonData,
    color: WguiColor,
    color2: WguiColor,
    base_border_color: WguiColor,
    cur_border_color: Cell<WguiColor>,
    border: f32,
    drawn_state: Cell<bool>,
    labels: SmallVec<[ChildWidget; 3]>,
    sprites: SmallVec<[ChildWidget; 1]>,
}

#[derive(Debug)]
enum KeyButtonData {
    Key {
        vk: VirtualKey,
        pressed: Cell<bool>,
    },
    Modifier {
        modifier: KeyModifier,
        sticky: Cell<bool>,
    },
    Macro {
        verbs: Vec<(VirtualKey, bool)>,
    },
    Exec {
        program: String,
        args: Vec<String>,
        release_program: Option<String>,
        release_args: Vec<String>,
    },
    KeymapSwitch {
        layouts: Vec<Arc<str>>,
    },
}

fn handle_mouse_motion(
    key: &KeyState,
    key_label: &Vec<String>,
    key_cap_type: &KeyCapType,
    keyboard: &mut KeyboardState,
    within_key_pos: &Option<Vec2>,
    device: DeviceBitmask,
) {
    if let KeyButtonData::Key { .. } = &key.button_state
        && let Some(swipe_manager) = keyboard.swipe_typing_manager.as_mut()
    {
        swipe_manager.handle_key_motion(key_cap_type, within_key_pos, key_label, device);
    }
}
fn handle_press(
    app: &mut AppState,
    key: &KeyState,
    key_label: &Vec<String>,
    key_cap_type: &KeyCapType,
    within_key_pos: &Option<Vec2>,
    keyboard: &mut KeyboardState,
    button: MouseButtonEvent,
    device: DeviceBitmask,
) {
    match &key.button_state {
        KeyButtonData::Key { vk, pressed } => {
            let outcome = match keyboard.swipe_typing_manager.as_mut() {
                Some(swipe_manager) => swipe_manager.handle_key_press(
                    key_cap_type,
                    within_key_pos,
                    key_label,
                    device,
                    button.index,
                ),
                None => KeyPressOutcome::Dispatch,
            };

            if matches!(outcome, KeyPressOutcome::Dispatch) {
                keyboard.modifiers |= match button.index {
                    MouseButtonIndex::Right => SHIFT,
                    MouseButtonIndex::Middle => keyboard.alt_modifier,
                    _ => 0,
                };
                app.hid_provider
                    .set_modifiers_routed(app.wvr_server.as_mut(), keyboard.modifiers);
                app.hid_provider
                    .send_key_routed(app.wvr_server.as_mut(), *vk, true);
                pressed.set(true);
                play_key_click(app);
            }
        }
        KeyButtonData::Modifier { modifier, sticky } => {
            sticky.set(keyboard.modifiers & *modifier == 0);
            keyboard.modifiers |= *modifier;
            app.hid_provider
                .set_modifiers_routed(app.wvr_server.as_mut(), keyboard.modifiers);
            play_key_click(app);
        }
        KeyButtonData::Macro { verbs } => {
            for (vk, press) in verbs {
                app.hid_provider
                    .send_key_routed(app.wvr_server.as_mut(), *vk, *press);
            }
            play_key_click(app);
        }
        KeyButtonData::Exec { program, args, .. } => {
            // Reap previous processes
            keyboard
                .processes
                .retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_))));

            if let Ok(child) = Command::new(program).args(args).spawn() {
                keyboard.processes.push(child);
            }
            play_key_click(app);
        }
        KeyButtonData::KeymapSwitch { layouts } => {
            if layouts.is_empty() {
                Toast::new(
                    ToastTopic::System,
                    Some(Translation::from_translation_key(
                        "TOAST.NO_KEYMAPS_CONFIGURED",
                    )),
                    Translation::from_translation_key("TOAST.NO_KEYMAPS_CONFIGURED_HELP"),
                )
                .with_timeout(5.)
                .submit(app);
                return;
            }
            keyboard.keymap_switch_index = (keyboard.keymap_switch_index + 1) % layouts.len();
            keyboard.keymap_switch_pending = true;
            play_key_click(app);
        }
    }
}

fn handle_release(
    app: &mut AppState,
    key: &KeyState,
    k_cap_type: &KeyCapType,
    keyboard: &mut KeyboardState,
) -> bool {
    match &key.button_state {
        KeyButtonData::Key { vk, pressed } => {
            let outcome = match keyboard.swipe_typing_manager.as_mut() {
                Some(swipe_manager) => {
                    swipe_manager.handle_key_release(k_cap_type, keyboard.alt_modifier)
                }
                None => KeyReleaseOutcome::Normal,
            };

            match outcome {
                KeyReleaseOutcome::Predict => {}
                KeyReleaseOutcome::TapKey { modifier } => {
                    keyboard.modifiers |= modifier;
                    app.hid_provider
                        .set_modifiers_routed(app.wvr_server.as_mut(), keyboard.modifiers);
                    app.hid_provider
                        .send_key_routed(app.wvr_server.as_mut(), *vk, true);
                    pressed.set(true);
                    app.hid_provider
                        .send_key_routed(app.wvr_server.as_mut(), *vk, false);

                    for m in &AUTO_RELEASE_MODS {
                        if keyboard.modifiers & *m != 0 {
                            keyboard.modifiers &= !*m;
                        }
                    }
                    app.hid_provider
                        .set_modifiers_routed(app.wvr_server.as_mut(), keyboard.modifiers);
                    play_key_click(app);
                }
                KeyReleaseOutcome::Normal => {
                    pressed.set(false);

                    for m in &AUTO_RELEASE_MODS {
                        if keyboard.modifiers & *m != 0 {
                            keyboard.modifiers &= !*m;
                        }
                    }
                    app.hid_provider
                        .send_key_routed(app.wvr_server.as_mut(), *vk, false);
                    app.hid_provider
                        .set_modifiers_routed(app.wvr_server.as_mut(), keyboard.modifiers);
                }
            }
            true
        }
        KeyButtonData::Modifier { modifier, sticky } => {
            if sticky.get() {
                false
            } else {
                keyboard.modifiers &= !*modifier;
                app.hid_provider
                    .set_modifiers_routed(app.wvr_server.as_mut(), keyboard.modifiers);
                true
            }
        }
        KeyButtonData::Exec {
            release_program,
            release_args,
            ..
        } => {
            // Reap previous processes
            keyboard
                .processes
                .retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_))));

            if let Some(program) = release_program
                && let Ok(child) = Command::new(program).args(release_args).spawn()
            {
                keyboard.processes.push(child);
            }
            true
        }
        _ => true,
    }
}
