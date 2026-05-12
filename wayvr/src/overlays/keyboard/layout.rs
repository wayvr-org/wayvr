use std::{collections::HashMap, str::FromStr, sync::LazyLock};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{
    config::{ConfigType, load_known_yaml},
    subsystem::hid::{
        KEYS_TO_MODS, KeyType, META, NUM_LOCK, SHIFT, VirtualKey, XkbKeymap, get_key_type,
    },
};

use super::KeyButtonData;

static MACRO_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Za-z0-9_-]+)(?: +(UP|DOWN))?$").unwrap()); // want panic

#[derive(Debug, Deserialize, Serialize)]
#[allow(clippy::struct_field_names)]
pub struct Layout {
    pub(super) row_size: f32,
    pub(super) key_sizes: Vec<Vec<f32>>,
    pub(super) main_layout: Vec<Vec<Option<String>>>,
    pub(super) exec_commands: HashMap<String, Vec<String>>,
    pub(super) macros: HashMap<String, Vec<String>>,
    pub(super) labels: HashMap<String, Vec<String>>,
    pub(super) auto_labels: Option<bool>,
}

impl Layout {
    pub(super) fn load_from_disk() -> Self {
        let mut layout = load_known_yaml::<Self>(ConfigType::Keyboard);
        layout.post_load();
        layout
    }

    fn post_load(&mut self) {
        for i in 0..self.key_sizes.len() {
            let row = &self.key_sizes[i];
            let width: f32 = row.iter().sum();
            assert!(
                (width - self.row_size).abs() < 0.001,
                "Row {} has a width of {}, but the row size is {}",
                i,
                width,
                self.row_size
            );
        }

        for i in 0..self.main_layout.len() {
            let row = &self.main_layout[i];
            let width = row.len();
            assert!(
                (width == self.key_sizes[i].len()),
                "Row {} has {} keys, needs to have {} according to key_sizes",
                i,
                width,
                self.key_sizes[i].len()
            );
        }
    }

    pub(super) fn get_key_data(
        &self,
        keymap: Option<&XkbKeymap>,
        has_altgr: bool,
        col: usize,
        row: usize,
    ) -> Option<KeyData> {
        let key = self.main_layout[row][col].as_ref()?;
        let mut label = Vec::with_capacity(3);
        let mut cap_type = KeyCapType::Letter;
        let button_state: KeyButtonData;

        if let Ok(vk) = VirtualKey::from_str(key) {
            if let Some(keymap) = keymap.as_ref() {
                match get_key_type(vk) {
                    KeyType::Symbol => {
                        let label0 = keymap.label_for_key(vk, 0);
                        let label1 = keymap.label_for_key(vk, SHIFT);

                        if label0.chars().next().is_some_and(char::is_alphabetic) {
                            label.push(label1);
                            if has_altgr {
                                cap_type = KeyCapType::LetterAltGr;
                                label.push(keymap.label_for_key(vk, META));
                            } else {
                                cap_type = KeyCapType::Letter;
                            }
                        } else {
                            label.push(label0);
                            label.push(label1);
                            if has_altgr {
                                label.push(keymap.label_for_key(vk, META));
                                cap_type = KeyCapType::SymbolAltGr;
                            } else {
                                cap_type = KeyCapType::Symbol;
                            }
                        }
                    }
                    KeyType::NumPad => {
                        label.push(keymap.label_for_key(vk, NUM_LOCK));
                    }
                    KeyType::Special => {
                        cap_type = KeyCapType::Special;
                        match vk {
                            VirtualKey::RShift | VirtualKey::LShift => {
                                label.push("shift".into());
                            }
                            VirtualKey::RSuper | VirtualKey::LSuper => {
                                label.push("super".into());
                            }
                            VirtualKey::KP_Enter => {
                                label.push("return".into());
                            }
                            _ => label.push(format!("{vk:?}").to_lowercase()),
                        }
                    }
                    KeyType::Other => {
                        cap_type = KeyCapType::Other;
                    }
                }
            }

            if let Some(mods) = KEYS_TO_MODS.get(vk) {
                button_state = KeyButtonData::Modifier {
                    modifier: *mods,
                    sticky: false.into(),
                };
            } else {
                button_state = KeyButtonData::Key {
                    vk,
                    pressed: false.into(),
                };
            }
        } else if let Some(macro_verbs) = self.macros.get(key) {
            button_state = KeyButtonData::Macro {
                verbs: key_events_for_macro(macro_verbs),
            };
        } else if let Some(exec_args) = self.exec_commands.get(key) {
            let mut iter = exec_args.iter().cloned();
            if let Some(program) = iter.next() {
                button_state = KeyButtonData::Exec {
                    program,
                    args: iter.by_ref().take_while(|arg| arg[..] != *"null").collect(),
                    release_program: iter.next(),
                    release_args: iter.collect(),
                };
            } else {
                log::error!("Keyboard: EXEC args empty for {key}");
                return None;
            }
        } else {
            log::error!("Unknown key: {key}");
            return None;
        }

        if label.is_empty() {
            label = self.label_for_key(key);
        }

        Some(KeyData {
            label,
            button_state,
            cap_type,
        })
    }

    fn label_for_key(&self, key: &str) -> Vec<String> {
        if let Some(label) = self.labels.get(key) {
            return label.clone();
        }
        if key.is_empty() {
            return vec![];
        }
        if key.len() == 1 {
            return vec![key.to_string().to_lowercase()];
        }
        let mut key = key;
        if key.starts_with("KP_") {
            key = &key[3..];
        }
        if key.contains('_') {
            key = key.split('_').next().unwrap_or_else(|| {
                log::error!("keyboard.yaml: Key '{key}' must not start or end with '_'!");
                "???"
            });
        }
        vec![format!(
            "{}{}",
            key.chars().next().unwrap().to_uppercase(), // safe because we checked is_empty
            &key[1..].to_lowercase()
        )]
    }
}

fn key_events_for_macro(macro_verbs: &Vec<String>) -> Vec<(VirtualKey, bool)> {
    let mut key_events = vec![];
    for verb in macro_verbs {
        if let Some(caps) = MACRO_REGEX.captures(verb) {
            if let Ok(virtual_key) = VirtualKey::from_str(&caps[1]) {
                if let Some(state) = caps.get(2) {
                    if state.as_str() == "UP" {
                        key_events.push((virtual_key, false));
                    } else if state.as_str() == "DOWN" {
                        key_events.push((virtual_key, true));
                    } else {
                        log::error!(
                            "Unknown key state in macro: {}, looking for UP or DOWN.",
                            state.as_str()
                        );
                        return vec![];
                    }
                } else {
                    key_events.push((virtual_key, true));
                    key_events.push((virtual_key, false));
                }
            } else {
                log::error!("Unknown virtual key: {}", &caps[1]);
                return vec![];
            }
        }
    }
    key_events
}

pub(super) struct KeyData {
    pub(super) label: Vec<String>,
    pub(super) button_state: KeyButtonData,
    pub(super) cap_type: KeyCapType,
}

#[derive(Debug)]
pub enum KeyCapType {
    /// Label an SVG
    Special,
    /// Label is in center of keycap
    Letter,
    /// Label on the top
    /// AltGr symbol on bottom
    LetterAltGr,
    /// Primary symbol on bottom
    /// Shift symbol on top
    Symbol,
    /// Primary symbol on bottom-left
    /// Shift symbol on top-left
    /// AltGr symbol on bottom-right
    SymbolAltGr,
    /// Label has text in the center, e.g. Home
    Other,
}
