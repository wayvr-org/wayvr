use serde::{Deserialize, Serialize};

use crate::config_io;

const DEFAULT_XR_INPUT_PROFILES: &str = include_str!("../assets/openxr_actions.json5");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
	One(T),
	Many(Vec<T>),
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct OpenXrInputAction {
	pub left: Option<OneOrMany<String>>,
	pub right: Option<OneOrMany<String>>,
	pub handsfree: Option<OneOrMany<String>>,
	pub threshold: Option<[f32; 2]>,
	pub double_click: Option<bool>,
	pub triple_click: Option<bool>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct OpenXrInputProfile {
	pub profile: String,
	pub pose: Option<OpenXrInputAction>,
	pub click: Option<OpenXrInputAction>,
	pub grab: Option<OpenXrInputAction>,
	pub alt_click: Option<OpenXrInputAction>,
	pub show_hide: Option<OpenXrInputAction>,
	pub toggle_dashboard: Option<OpenXrInputAction>,
	pub space_drag: Option<OpenXrInputAction>,
	pub space_rotate: Option<OpenXrInputAction>,
	pub space_reset: Option<OpenXrInputAction>,
	pub click_modifier_right: Option<OpenXrInputAction>,
	pub click_modifier_middle: Option<OpenXrInputAction>,
	pub move_mouse: Option<OpenXrInputAction>,
	pub scroll: Option<OpenXrInputAction>,
	pub haptic: Option<OpenXrInputAction>,
}

pub fn load_xr_input_profiles() -> Vec<OpenXrInputProfile> {
	let mut profiles: Vec<OpenXrInputProfile> = serde_json5::from_str(DEFAULT_XR_INPUT_PROFILES).unwrap(); // want panic

	let Some(conf) = config_io::load("openxr_actions.json5") else {
		return profiles;
	};

	match serde_json5::from_str::<Vec<OpenXrInputProfile>>(&conf) {
		Ok(override_profiles) => {
			for new in override_profiles {
				if let Some(i) = profiles.iter().position(|old| old.profile == new.profile) {
					profiles[i] = new;
				} else {
					profiles.push(new);
				}
			}
		}
		Err(e) => {
			log::error!("Failed to load openxr_actions.json5: {e}");
		}
	}

	profiles
}
