use std::{
    cell::Cell,
    collections::HashMap,
    process::{Child, Command},
    sync::atomic::Ordering,
};

use crate::{
    KEYMAP_CHANGE,
    backend::{
        input::{HoverResult, PointerHit},
        task::{OverlayTask, TaskType},
    },
    gui::panel::{GuiPanel, overlay_list::OverlayList, set_list::SetList},
    overlays::keyboard::builder::create_keyboard_panel,
    state::AppState,
    subsystem::{
        dbus::DbusConnector,
        hid::{
            ALT, CTRL, KeyModifier, META, SHIFT, SUPER, VirtualKey, WheelDelta, XkbKeymap,
            get_keymap_wl, get_keymap_x11,
        },
    },
    windowing::{
        backend::{FrameMeta, OverlayBackend, OverlayEventData, RenderResources, ShouldRender},
        window::{OverlayCategory, OverlayWindowConfig},
    },
};
use anyhow::Context;
use glam::{Affine3A, Quat, Vec3, vec3, Vec2};
use regex::Regex;
use slotmap::{SlotMap, new_key_type};
use wgui::{
    drawing,
    event::{InternalStateChangeEvent, MouseButtonEvent, MouseButtonIndex},
};
use wlx_common::windowing::{OverlayWindowState, Positioning};
use wlx_common::{
    config::AltModifier,
    overlays::{BackendAttrib, BackendAttribValue},
};
use crate::overlays::keyboard::builder::update_swipe_prediction_bar;
use crate::overlays::keyboard::layout::KeyCapType;
use crate::overlays::keyboard::swipe_type::SwipeTypingManager;

pub mod builder;
mod layout;
mod swipe_type;

pub const KEYBOARD_NAME: &str = "kbd";
const AUTO_RELEASE_MODS: [KeyModifier; 5] = [SHIFT, CTRL, ALT, SUPER, META];
const SYSTEM_LAYOUT_ALIASES: [&str; 5] = ["mozc", "pinyin", "hangul", "sayura", "unikey"];

pub fn create_keyboard(app: &mut AppState, wayland: bool) -> anyhow::Result<OverlayWindowConfig> {
    let layout = layout::Layout::load_from_disk();
    let default_state = KeyboardState {
        modifiers: 0,
        alt_modifier: alt_modifier_to_key(app.session.config.keyboard_middle_click_mode),
        processes: vec![],
        overlay_list: OverlayList::default(),
        set_list: SetList::default(),
        clock_12h: app.session.config.clock_12h,
        swipe_typing_manager: None,
        swipe_bar_candidates: Vec::new(),
    };

    let auto_labels = layout.auto_labels.unwrap_or(true);

    let width = layout.row_size * 0.05 * app.session.config.keyboard_scale;

    let mut backend = KeyboardBackend {
        layout_panels: SlotMap::default(),
        layout_ids: HashMap::default(),
        active_layout: KeyboardPanelKey::default(),
        default_state,
        wlx_layout: layout,
        wayland,
        re_fcitx: Regex::new(r"^keyboard-([^-]+)(?:-([^-]+))?$").unwrap(),
    };

    let mut maybe_keymap = backend
        .get_effective_keymap()
        .inspect_err(|e| log::warn!("{e:?}"))
        .or_else(|_| {
            if let Some(layout_variant) = app.session.config.default_keymap.as_ref() {
                let mut splat = layout_variant.split('-');
                XkbKeymap::from_layout_variant(
                    splat.next().unwrap_or(""),
                    splat.next().unwrap_or(""),
                )
                .context("invalid value for default_keymap")
            } else {
                anyhow::bail!("no default_keymap set")
            }
        })
        .ok();

    if let Some(keymap) = maybe_keymap.as_ref() {
        app.hid_provider
            .keymap_changed(app.wvr_server.as_mut(), keymap);
    }

    if !auto_labels {
        maybe_keymap = None;
    }

    backend.active_layout = backend.add_new_keymap(maybe_keymap.as_ref(), app)?;

    Ok(OverlayWindowConfig {
        name: KEYBOARD_NAME.into(),
        category: OverlayCategory::Keyboard,
        default_state: OverlayWindowState {
            grabbable: true,
            positioning: Positioning::Anchored,
            interactable: true,
            curvature: Some(0.15),
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

fn alt_modifier_to_key(m: AltModifier) -> KeyModifier {
    match m {
        AltModifier::Shift => SHIFT,
        AltModifier::Ctrl => CTRL,
        AltModifier::Alt => ALT,
        AltModifier::Super => SUPER,
        AltModifier::Meta => META,
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
}

impl KeyboardBackend {
    fn add_new_keymap(
        &mut self,
        keymap: Option<&XkbKeymap>,
        app: &mut AppState,
    ) -> anyhow::Result<KeyboardPanelKey> {
        let mut state = self.default_state.take();

        state.swipe_typing_manager = match SwipeTypingManager::new() {
            Ok(engine) => Some(engine),
            Err(e) => {
                log::error!("Error occured while trying to load swipe engine: {:?}", e);
                None
            }
        };

        log::info!("swipe engine created");
        let panel =
            create_keyboard_panel(app, keymap, state, &self.wlx_layout)?;

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
            self.internal_switch_keymap(*new_key, keymap);
        } else {
            let new_key = self.add_new_keymap(Some(keymap), app)?;
            self.internal_switch_keymap(new_key, keymap);
        }
        app.tasks
            .enqueue(TaskType::Overlay(OverlayTask::KeyboardChanged));
        Ok(true)
    }

    fn internal_switch_keymap(&mut self, new_key: KeyboardPanelKey, keymap: &XkbKeymap) {
        let mut state_from = self
            .layout_panels
            .get_mut(self.active_layout)
            .unwrap()
            .state
            .take();

        state_from.swipe_typing_manager = match SwipeTypingManager::new() {
            Ok(engine) => Some(engine),
            Err(e) => {
                log::error!("Error occured while trying to load swipe engine: {:?}", e);
                None
            }
        };
        self.active_layout = new_key;

        self.layout_panels
            .get_mut(self.active_layout)
            .unwrap()
            .state = state_from;
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
        if update_swipe_prediction_bar(self.panel(), app)? {
            self.panel().process_custom_elems(app);
        }
        Ok(())
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
    swipe_typing_manager: Option<SwipeTypingManager>,
    swipe_bar_candidates: Vec<String>,
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
            swipe_typing_manager: None,
            swipe_bar_candidates: Vec::new(),
        }
    }
}

fn play_key_click(app: &mut AppState) {
    app.audio_sample_player
        .play_sample(&mut app.audio_system, "key_click");
}

struct KeyState {
    button_state: KeyButtonData,
    color: drawing::Color,
    color2: drawing::Color,
    base_border_color: drawing::Color,
    cur_border_color: Cell<drawing::Color>,
    border: f32,
    drawn_state: Cell<bool>,
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
}

fn handle_mouse_motion(key: &KeyState, key_label: &Vec<String>, key_cap_type: &KeyCapType, keyboard: &mut KeyboardState, within_key_pos: &Option<Vec2>) {
    if let Some(swipe_manager) = keyboard.swipe_typing_manager.as_mut() && *key_cap_type == KeyCapType::Letter {
        if !swipe_manager.is_current_swipe_empty() {
            match &key.button_state {
                KeyButtonData::Key { vk, pressed } => {
                    if let Some(pos) = within_key_pos {
                        // check because mouse motion can trigger despite hover being false
                        if pos.x >= 0.0 && pos.x <= 1.0 && pos.y >= 0.0 && pos.y <= 1.0 {

                            if let Some(label) = key_label.first() {
                                swipe_manager.add_swipe(pos, label.chars().next().unwrap_or_default());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
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
) {
    match &key.button_state {
        KeyButtonData::Key { vk, pressed } => {
            if let Some(swipe_manager) = keyboard.swipe_typing_manager.as_mut() && *key_cap_type == KeyCapType::Letter {
                if let Some(pos) = within_key_pos {
                    if let Some(label) = key_label.first() {
                        swipe_manager.add_swipe(pos, label.chars().next().unwrap_or_default());
                    }
                }
            }
            else {
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
    }
}

fn handle_release(app: &mut AppState, key: &KeyState, k_cap_type: &KeyCapType, keyboard: &mut KeyboardState) -> bool {
    match &key.button_state {
        KeyButtonData::Key { vk, pressed } => {
            if let Some(swipe_manager) = keyboard.swipe_typing_manager.as_mut() && *k_cap_type == KeyCapType::Letter {
                println!("some");
                if swipe_manager.did_swipe_leave_first_key() {
                    println!("left first");
                    match swipe_manager.predict() {
                        Ok(top_prediction) => {
                            println!("best prediction for swipe: {top_prediction}");
                            swipe_manager.select_word(&top_prediction, app, keyboard.modifiers);
                        },
                        Err(e) => {
                            log::error!("{}", e)
                        }
                    }
                }
                else { // pointer must have been released on the same key it was pressed on
                    println!("swipe reset");
                    swipe_manager.reset(); // drop swipe tracking that was started on press

                    app.hid_provider
                        .send_key_routed(app.wvr_server.as_mut(), *vk, true);
                    pressed.set(true);
                    app.hid_provider
                        .send_key_routed(app.wvr_server.as_mut(), *vk, false);
                    play_key_click(app);
                }
            }
            else {
                if let Some(swipe_manager) = keyboard.swipe_typing_manager.as_mut() {
                    swipe_manager.reset();
                }
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
