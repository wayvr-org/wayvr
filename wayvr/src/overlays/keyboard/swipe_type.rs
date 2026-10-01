use super::layout::KeyCapType;
use crate::state::AppState;
use crate::subsystem::clipboard;
use crate::subsystem::clipboard::ClipboardProvider;
use crate::subsystem::hid::{CTRL, KeyModifier, SHIFT, VirtualKey};
use crate::subsystem::input::InputFocus;
use anyhow::bail;
use glam::Vec2;
use std::mem;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;
use super_swipe_type::SwipePoint;
use super_swipe_type::keyboard_manager::QwertyKeyboardGrid;
use super_swipe_type::swipe_orchestrator::SwipeOrchestrator;
use wgui::event::{DeviceBitmask, MouseButtonIndex};
use wgui::log::LogErr;

const PREDICTION_SUGGESTION_COUNT: usize = 5;

enum PredictionTask {
    Predict {
        swipe: Vec<SwipePoint>,
        last_word: Option<String>,
    },
    Shutdown,
}
#[derive(Clone)]
pub struct PredictionSlot(Arc<PredictionSlotInner>);

struct PredictionSlotInner {
    present: AtomicBool,
    value: Mutex<Option<Vec<String>>>,
}

impl PredictionSlot {
    pub fn new() -> Self {
        Self(Arc::new(PredictionSlotInner {
            present: AtomicBool::new(false),
            value: Mutex::new(None),
        }))
    }

    pub fn set(&self, value: Option<Vec<String>>) {
        *self.0.value.lock().unwrap() = value;
        self.0.present.store(true, Ordering::Release);
    }

    /// Returns the latest pending value, if any, and clears the slot.
    /// `Some(Some(words))` is a fresh prediction, `Some(None)` is an explicit
    /// clear (mirrors the previous `send(None)`), `None` means nothing pending.
    pub fn take(&self) -> Option<Option<Vec<String>>> {
        if !self.0.present.swap(false, Ordering::Acquire) {
            return None;
        }
        let value = self.0.value.lock().unwrap().take();
        Some(value)
    }
}

impl Default for PredictionSlot {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SwipeTypingManager {
    keyboard_gird: QwertyKeyboardGrid,
    current_swipe: Vec<SwipePoint>,
    prediction_slot: PredictionSlot,
    prediction_task_sender: Sender<PredictionTask>,
    worker_thread: Option<JoinHandle<()>>,
    swipe_start_time: Option<Instant>,
    clipboard: Option<Box<dyn ClipboardProvider>>,
    swipe_left_first_key: bool,
    first_swipe_char: char,
    current_swipe_device: Option<DeviceBitmask>,
    current_swipe_mouse_button_index: Option<MouseButtonIndex>,
    last_swiped_word: Option<String>,
}

impl SwipeTypingManager {
    pub fn select_alternate_prediction(
        &mut self,
        word: &String,
        app: &mut AppState,
        original_keyboard_mods: KeyModifier,
    ) {
        Self::undo(app, original_keyboard_mods);
        self.select_word(word, app, original_keyboard_mods);
    }

    pub fn select_word(
        &mut self,
        word: &String,
        app: &mut AppState,
        original_keyboard_mods: KeyModifier,
    ) {
        self.last_swiped_word = Some(word.clone());
        let text_to_paste = format!("{word} ");

        match app.hid_provider.get_input_focus() {
            InputFocus::PhysicalScreen => {
                if let Some(clipboard) = self.clipboard.as_mut() {
                    clipboard.set_clipboard_utf8(&text_to_paste);
                    Self::paste(app, original_keyboard_mods);
                }
            }
            InputFocus::WayVR => {
                if let Some(wvr_server) = app.wvr_server.as_mut() {
                    wvr_server.set_clipboard_text(text_to_paste);
                    Self::paste(app, original_keyboard_mods);
                }
            }
        }
    }

    fn undo(app: &mut AppState, original_keyboard_mods: KeyModifier) {
        app.hid_provider
            .set_modifiers_routed(app.wvr_server.as_mut(), CTRL);
        app.hid_provider
            .send_key_routed(app.wvr_server.as_mut(), VirtualKey::Z, true);
        app.hid_provider
            .send_key_routed(app.wvr_server.as_mut(), VirtualKey::Z, false);
        app.hid_provider
            .set_modifiers_routed(app.wvr_server.as_mut(), original_keyboard_mods);
    }

    fn paste(app: &mut AppState, original_keyboard_mods: KeyModifier) {
        app.hid_provider
            .set_modifiers_routed(app.wvr_server.as_mut(), CTRL);
        app.hid_provider
            .send_key_routed(app.wvr_server.as_mut(), VirtualKey::V, true);
        app.hid_provider
            .send_key_routed(app.wvr_server.as_mut(), VirtualKey::V, false);
        app.hid_provider
            .set_modifiers_routed(app.wvr_server.as_mut(), original_keyboard_mods);
    }
    pub fn new(model_path: PathBuf) -> anyhow::Result<(SwipeTypingManager, PredictionSlot)> {
        let prediction_slot = PredictionSlot::new();
        let worker_slot = prediction_slot.clone();
        let (task_sender, task_receiver) = channel::<PredictionTask>();

        // Spawn persistent worker thread
        let worker_thread = thread::spawn(move || {
            let mut swipe_engine = match SwipeOrchestrator::new_from_path(&model_path) {
                Ok(engine) => engine,
                Err(e) => {
                    log::error!("Failed to initialize SwipeOrchestrator: {e}");
                    return;
                }
            };

            while let Ok(task) = task_receiver.recv() {
                match task {
                    PredictionTask::Predict { swipe, last_word } => {
                        match swipe_engine.predict(swipe, &last_word) {
                            Ok(candidates) => {
                                let words: Vec<String> = candidates
                                    .into_iter()
                                    .take(PREDICTION_SUGGESTION_COUNT)
                                    .map(|c| c.word)
                                    .collect();

                                worker_slot.set(Some(words));
                            }
                            Err(e) => {
                                log::error!("Prediction failed: {e}");
                            }
                        }
                    }
                    PredictionTask::Shutdown => break,
                }
            }
        });
        let clipboard_provider: Option<Box<dyn ClipboardProvider>> = {
            #[cfg(feature = "wayland")]
            let wl = clipboard::wl::Provider::new()
                .log_err("Could not create Wayland clipboard provider")
                .ok()
                .map(|p| Box::new(p) as Box<dyn ClipboardProvider>);
            #[cfg(not(feature = "wayland"))]
            let wl = None;

            #[cfg(feature = "x11")]
            let x11 = clipboard::x11::Provider::new()
                .log_err("Could not create X11 clipboard provider")
                .ok()
                .map(|p| Box::new(p) as Box<dyn ClipboardProvider>);
            #[cfg(not(feature = "x11"))]
            let x11 = None;

            wl.or(x11)
        };
        Ok((
            Self {
                keyboard_gird: QwertyKeyboardGrid::new(),
                current_swipe: Vec::new(),
                prediction_slot: prediction_slot.clone(),
                prediction_task_sender: task_sender,
                worker_thread: Some(worker_thread),
                swipe_start_time: None,
                clipboard: clipboard_provider,
                swipe_left_first_key: false,
                first_swipe_char: char::default(),
                current_swipe_device: None,
                current_swipe_mouse_button_index: None,
                last_swiped_word: None,
            },
            prediction_slot,
        ))
    }

    pub fn predict(&mut self) -> anyhow::Result<()> {
        if self.is_current_swipe_empty() {
            bail!("nothing to predict");
        }

        let current_swipe = mem::take(&mut self.current_swipe);
        let last_word = self.last_swiped_word.clone();
        self.reset_swipe();

        self.prediction_task_sender.send(PredictionTask::Predict {
            swipe: current_swipe,
            last_word,
        })?;

        Ok(())
    }

    pub fn reset(&mut self) {
        self.reset_swipe();
        self.prediction_slot.set(None);
        self.last_swiped_word = None;
    }

    fn reset_swipe(&mut self) {
        self.swipe_start_time = None;
        self.current_swipe = Vec::new();
        self.first_swipe_char = char::default();
        self.swipe_left_first_key = false;
        self.current_swipe_device = None;
        self.current_swipe_mouse_button_index = None;
    }

    fn start_swipe(
        &mut self,
        key_label: char,
        device: DeviceBitmask,
        index: Option<MouseButtonIndex>,
    ) -> Instant {
        let now = Instant::now();
        self.swipe_start_time = Some(now);
        self.first_swipe_char = key_label.to_ascii_lowercase();
        self.current_swipe_device = Some(device);
        self.current_swipe_mouse_button_index = index;
        now
    }

    pub const fn did_swipe_leave_first_key(&self) -> bool {
        self.swipe_left_first_key
    }

    pub const fn is_current_swipe_empty(&self) -> bool {
        self.current_swipe.is_empty()
    }

    pub const fn current_swipe_mouse_button_index(&self) -> Option<MouseButtonIndex> {
        self.current_swipe_mouse_button_index
    }

    /// Handle a key press. Letter keys feed the swipe engine and are consumed;
    /// anything else is dispatched as a normal key.
    pub fn handle_key_press(
        &mut self,
        key_cap_type: &KeyCapType,
        within_key_pos: &Option<Vec2>,
        key_label: &[String],
        device: DeviceBitmask,
        index: MouseButtonIndex,
    ) -> super::KeyPressOutcome {
        if !matches!(key_cap_type, KeyCapType::Letter | KeyCapType::LetterAltGr) {
            return super::KeyPressOutcome::Dispatch;
        }

        if let Some(pos) = within_key_pos
            && let Some(label) = key_label.first()
        {
            self.add_swipe(
                pos,
                label.chars().next().unwrap_or_default(),
                device,
                Some(index),
            );
        }

        super::KeyPressOutcome::Consumed
    }

    /// Handle pointer motion over a key while swiping.
    pub fn handle_key_motion(
        &mut self,
        key_cap_type: &KeyCapType,
        within_key_pos: &Option<Vec2>,
        key_label: &[String],
        device: DeviceBitmask,
    ) {
        if !self.is_current_swipe_empty()
            && matches!(key_cap_type, KeyCapType::Letter | KeyCapType::LetterAltGr)
            && let Some(pos) = within_key_pos
            && pos.x >= 0.0
            && pos.x <= 1.0
            && pos.y >= 0.0
            && pos.y <= 1.0
            && let Some(label) = key_label.first()
        {
            self.add_swipe(pos, label.chars().next().unwrap_or_default(), device, None);
        }
    }

    /// Handle a key release. A swipe that left the first key triggers a
    /// prediction; a tap on the same key dispatches the key; anything else is
    /// a normal key-up.
    pub fn handle_key_release(
        &mut self,
        key_cap_type: &KeyCapType,
        alt_modifier: KeyModifier,
    ) -> super::KeyReleaseOutcome {
        if !matches!(key_cap_type, KeyCapType::Letter | KeyCapType::LetterAltGr) {
            self.reset();
            return super::KeyReleaseOutcome::Normal;
        }

        if self.did_swipe_leave_first_key() {
            if let Err(e) = self.predict() {
                log::error!("{}", e);
            }
            return super::KeyReleaseOutcome::Predict;
        }

        let modifier = match self.current_swipe_mouse_button_index() {
            Some(MouseButtonIndex::Right) => SHIFT,
            Some(MouseButtonIndex::Middle) => alt_modifier,
            _ => 0,
        };
        self.reset();
        super::KeyReleaseOutcome::TapKey { modifier }
    }

    pub fn add_swipe(
        &mut self,
        within_key_pos_normalized: &Vec2,
        key_label: char,
        device: DeviceBitmask,
        index: Option<MouseButtonIndex>,
    ) {
        if let Some(pos) = self
            .keyboard_gird
            .key_positions
            .get(&key_label.to_ascii_lowercase())
        {
            if let Some(current_device) = self.current_swipe_device
                && current_device != device
            {
                return;
            }

            if self.first_swipe_char != char::default()
                && self.first_swipe_char != key_label.to_ascii_lowercase()
            {
                self.swipe_left_first_key = true;
            }

            let key_pos = Vec2 {
                x: pos.x as f32,
                y: pos.y as f32,
            };

            let start_time = match self.swipe_start_time {
                Some(time) => time,
                None => self.start_swipe(key_label, device, index),
            };

            let within_key_pos_from_center = Vec2 {
                x: within_key_pos_normalized.x - 0.5,
                y: within_key_pos_normalized.y - 0.5,
            };
            let key_dimensions = Vec2 {
                x: QwertyKeyboardGrid::get_key_width() as f32,
                y: QwertyKeyboardGrid::get_key_height() as f32,
            };

            let point = within_key_pos_from_center * key_dimensions + key_pos;
            let duration = Instant::now().duration_since(start_time).mul_f32(0.8); // multiply by .8 because library is trained on mobile swipes which happen on a smaller keyboard and are faster
            self.current_swipe
                .push(SwipePoint::new(point.x.into(), point.y.into(), duration));
        }
    }
}

impl Drop for SwipeTypingManager {
    fn drop(&mut self) {
        let _ = self.prediction_task_sender.send(PredictionTask::Shutdown);
        if let Some(handle) = self.worker_thread.take() {
            let _ = handle.join();
        }
    }
}
