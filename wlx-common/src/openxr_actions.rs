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
	#[serde(skip_serializing_if = "Option::is_none")]
	pub left: Option<OneOrMany<String>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub right: Option<OneOrMany<String>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub handsfree: Option<OneOrMany<String>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub threshold: Option<[f32; 2]>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub double_click: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub triple_click: Option<bool>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct OpenXrInputProfile {
	pub profile: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub pose: Option<OpenXrInputAction>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub click: Option<OpenXrInputAction>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub grab: Option<OpenXrInputAction>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub alt_click: Option<OpenXrInputAction>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub show_hide: Option<OpenXrInputAction>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub toggle_dashboard: Option<OpenXrInputAction>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub space_drag: Option<OpenXrInputAction>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub space_rotate: Option<OpenXrInputAction>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub space_reset: Option<OpenXrInputAction>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub click_modifier_right: Option<OpenXrInputAction>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub click_modifier_middle: Option<OpenXrInputAction>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub move_mouse: Option<OpenXrInputAction>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub scroll: Option<OpenXrInputAction>,
	#[serde(skip_serializing_if = "Option::is_none")]
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
