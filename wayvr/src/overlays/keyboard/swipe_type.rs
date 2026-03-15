use crate::state::AppState;
use crate::subsystem::hid::{KeyModifier, VirtualKey, CTRL};
use anyhow::{anyhow, bail};
use arboard::Clipboard;
use glam::Vec2;
use std::mem;
use std::time::Instant;
use super_swipe_type::keyboard_manager::QwertyKeyboardGrid;
use super_swipe_type::swipe_orchestrator::SwipeOrchestrator;
use super_swipe_type::{SwipeCandidate, SwipePoint};

const PREDICTION_SUGGESTION_COUNT: usize = 4;
pub struct SwipeTypingManager {
    swipe_engine: SwipeOrchestrator,
    keyboard_gird: QwertyKeyboardGrid,
    current_swipe: Vec<SwipePoint>,
    pub swipe_candidates: Vec<String>,
    swipe_start_time: Option<Instant>,
    clipboard: Clipboard,
    swipe_left_first_key: bool,
    first_swipe_char: char,
    current_swipe_device: Option<usize>, // the pointer that started this swipe
    last_swiped_word: Option<String>
}
impl SwipeTypingManager {

    pub fn select_alternate_prediction(&mut self, word: &String, app: &mut AppState, original_keyboard_mods: KeyModifier) {
        Self::undo_paste(app, original_keyboard_mods);
        self.select_word(word, app, original_keyboard_mods);
    }
    /// Attempts to "type" the word by copying to clipboard and pasting
    pub fn select_word(&mut self, word: &String, app: &mut AppState, original_keyboard_mods: KeyModifier) {
        self.last_swiped_word = Some(word.clone());
        if let Ok(_) =self.copy_text_to_primary_clipboard(word.as_ref()) {
            Self::paste(app, original_keyboard_mods);
        }
    }
    fn undo_paste(app: &mut AppState, original_keyboard_mods: KeyModifier) {
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
    fn copy_text_to_primary_clipboard(&mut self, text: &str) -> Result<(), arboard::Error> {
        self.clipboard.set_text(format!("{text} "))
    }
    pub fn new() -> anyhow::Result<SwipeTypingManager> {
        // todo: use the layout_name to choose a sensible language for the swipe engine

        Ok(Self {
            swipe_engine: SwipeOrchestrator::new()?,
            keyboard_gird: QwertyKeyboardGrid::new(), // contains a hashmap<char, vector2> where the vector2 is the center pos of each key in querty
            current_swipe: Vec::new(),
            swipe_candidates: Vec::new(),
            swipe_start_time: None,
            clipboard: Clipboard::new()?,
            swipe_left_first_key: false,
            first_swipe_char: char::default(),
            current_swipe_device: None,
            last_swiped_word: None
        })
    }
    pub fn predict(&mut self) -> anyhow::Result<String>{
        if self.is_current_swipe_empty() {
            bail!("nothing to predict");
        } else {
            let current_swipe = mem::take(&mut self.current_swipe);
            self.reset_swipe();

            let candidates: Vec<SwipeCandidate> = self.swipe_engine.predict(current_swipe, &self.last_swiped_word)?;

            let mut iter = candidates.iter();
            let top_choice = iter.next().ok_or(anyhow!("No swipe candidates generated"))?.word.clone();
            self.swipe_candidates = iter.take(PREDICTION_SUGGESTION_COUNT).map(|c| c.word.clone()).collect();

            Ok(top_choice)
        }
    }
    pub fn reset(&mut self) {
        self.reset_swipe();
        self.last_swiped_word = None;
        self.swipe_candidates = Vec::new();
    }
    fn reset_swipe(&mut self) {
        self.swipe_start_time = None;
        self.current_swipe = Vec::new();
        self.first_swipe_char = char::default();
        self.swipe_left_first_key = false;
        self.current_swipe_device = None;
    }
    fn start_swipe(&mut self, key_label: char, device: usize) -> Instant{
        let now = Instant::now();
        self.swipe_start_time = Some(now);
        self.first_swipe_char = key_label.to_ascii_lowercase();
        self.current_swipe_device = Some(device);
        self.swipe_candidates = Vec::new();
        now
    }
    pub fn did_swipe_leave_first_key(&self) -> bool {
        self.swipe_left_first_key
    }
    pub fn is_current_swipe_empty(&self) -> bool {
        self.current_swipe.is_empty()
    }
    pub fn add_swipe(&mut self, within_key_pos_normalized: &Vec2, key_label: char, device: usize) {
        if let Some(pos) = self.keyboard_gird.key_positions.get(&key_label.to_ascii_lowercase()) {
            if self.first_swipe_char != char::default() && self.first_swipe_char != key_label.to_ascii_lowercase() {
                self.swipe_left_first_key = true;
            }

            let key_pos = Vec2{x: pos.x as f32, y: pos.y as f32 };
            //println!("char : {key_label} at pos: {key_pos}");
            let start_time = match self.swipe_start_time {
                Some(time) => time,
                None => self.start_swipe(key_label, device)
            };

            // only allow the pointer that started the swipe to contribute to it
            if let Some(current_device) = self.current_swipe_device && current_device != device{
                return;
            }

            let within_key_pos_from_center = Vec2 {
                x: within_key_pos_normalized.x - 0.5,
                y: 0.5 - within_key_pos_normalized.y,
            };
            let key_dimensions = Vec2 {
                x: QwertyKeyboardGrid::get_key_width() as f32,
                y: QwertyKeyboardGrid::get_key_width() as f32,
            };

            let point = within_key_pos_from_center*key_dimensions + key_pos;
            let duration = Instant::now().duration_since(start_time);
            self.current_swipe.push(SwipePoint::new(point.x.into(), point.y.into(), duration))
        }

    }



}
