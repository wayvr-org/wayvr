use crate::state::AppState;
use crate::subsystem::hid::{KeyModifier, VirtualKey, CTRL};
use anyhow::{bail};
use arboard::Clipboard;
use glam::Vec2;
use std::mem;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, channel, Sender};
use std::thread::{self, JoinHandle};
use std::time::Instant;
use super_swipe_type::keyboard_manager::QwertyKeyboardGrid;
use super_swipe_type::swipe_orchestrator::SwipeOrchestrator;
use super_swipe_type::{SwipePoint};
use crate::subsystem::input::KeyboardFocus;

const PREDICTION_SUGGESTION_COUNT: usize = 5;

enum PredictionTask {
    Predict {
        swipe: Vec<SwipePoint>,
        last_word: Option<String>,
    },
    Shutdown,
}

pub struct SwipeTypingManager {
    keyboard_gird: QwertyKeyboardGrid,
    current_swipe: Vec<SwipePoint>,
    swipe_candidate_sender: SyncSender<Option<Vec<String>>>,
    prediction_task_sender: Sender<PredictionTask>,
    worker_thread: Option<JoinHandle<()>>,
    swipe_start_time: Option<Instant>,
    clipboard: Clipboard,
    swipe_left_first_key: bool,
    first_swipe_char: char,
    current_swipe_device: Option<usize>,
    last_swiped_word: Option<String>,
}

impl SwipeTypingManager {
    pub fn select_alternate_prediction(&mut self, word: &String, app: &mut AppState, original_keyboard_mods: KeyModifier) {
        Self::undo(app, original_keyboard_mods);
        self.select_word(word, app, original_keyboard_mods);
    }

    pub fn select_word(&mut self, word: &String, app: &mut AppState, original_keyboard_mods: KeyModifier) {
        self.last_swiped_word = Some(word.clone());
        let text_to_paste = format!("{word} ");

        match app.hid_provider.keyboard_focus {
            KeyboardFocus::PhysicalScreen => {
                if let Ok(_) = self.clipboard.set_text(text_to_paste) {
                    Self::paste(app, original_keyboard_mods);
                }
            },
            KeyboardFocus::WayVR => {
                if let Some(wvr_server) = app.wvr_server.as_mut() {
                    wvr_server.set_clipboard_text(text_to_paste);
                    Self::paste(app, original_keyboard_mods);
                }
            },
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
    pub fn new() -> anyhow::Result<(SwipeTypingManager, Receiver<Option<Vec<String>>>)> {
        let (candidate_sender, candidate_receiver) = sync_channel(1);
        let (task_sender, task_receiver) = channel::<PredictionTask>();

        // Spawn persistent worker thread
        let worker_candidate_sender = candidate_sender.clone();
        let worker_thread = thread::spawn(move || {
            let mut swipe_engine = match SwipeOrchestrator::new() {
                Ok(engine) => engine,
                Err(e) => {
                    log::error!("Failed to initialize SwipeOrchestrator: {}", e);
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

                                let _ = worker_candidate_sender.send(Some(words));
                            }
                            Err(e) => {
                                log::error!("Prediction failed: {}", e);
                            }
                        }
                    }
                    PredictionTask::Shutdown => break,
                }
            }
        });

        Ok((
            Self {
                keyboard_gird: QwertyKeyboardGrid::new(),
                current_swipe: Vec::new(),
                swipe_candidate_sender: candidate_sender,
                prediction_task_sender: task_sender,
                worker_thread: Some(worker_thread),
                swipe_start_time: None,
                clipboard: Clipboard::new()?,
                swipe_left_first_key: false,
                first_swipe_char: char::default(),
                current_swipe_device: None,
                last_swiped_word: None,
            },
            candidate_receiver,
        ))
    }

    pub fn predict(&mut self) -> anyhow::Result<()> {
        if self.is_current_swipe_empty() {
            bail!("nothing to predict");
        }

        let current_swipe = mem::take(&mut self.current_swipe);
        let last_word = self.last_swiped_word.clone();
        self.reset_swipe();

        self.prediction_task_sender
            .send(PredictionTask::Predict {
                swipe: current_swipe,
                last_word,
            })?;

        Ok(())
    }

    pub fn reset(&mut self) {
        self.reset_swipe();
        let _ = self.swipe_candidate_sender.send(None);
        self.last_swiped_word = None;
    }

    fn reset_swipe(&mut self) {
        self.swipe_start_time = None;
        self.current_swipe = Vec::new();
        self.first_swipe_char = char::default();
        self.swipe_left_first_key = false;
        self.current_swipe_device = None;
    }

    fn start_swipe(&mut self, key_label: char, device: usize) -> Instant {
        let now = Instant::now();
        self.swipe_start_time = Some(now);
        self.first_swipe_char = key_label.to_ascii_lowercase();
        self.current_swipe_device = Some(device);
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
            if let Some(current_device) = self.current_swipe_device {
                if current_device != device {
                    return;
                }
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
                None => self.start_swipe(key_label, device),
            };

            let within_key_pos_from_center = Vec2 {
                x: within_key_pos_normalized.x - 0.5,
                y: 0.5 - within_key_pos_normalized.y,
            };
            let key_dimensions = Vec2 {
                x: QwertyKeyboardGrid::get_key_width() as f32,
                y: QwertyKeyboardGrid::get_key_height() as f32,
            };

            let point = within_key_pos_from_center * key_dimensions + key_pos;
            let duration = Instant::now().duration_since(start_time).mul_f32(0.8); // multiply by .8 because library is trained on mobile swipes which happen on a smaller keyboard and are faster
            self.current_swipe
                .push(SwipePoint::new(point.x.into(), point.y.into(), duration))
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
