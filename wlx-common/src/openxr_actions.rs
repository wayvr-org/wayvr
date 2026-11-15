use serde::{Deserialize, Serialize};

use crate::config_io;

const DEFAULT_XR_INPUT_PROFILES: &str = include_str!("../assets/openxr_actions.json5");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
	One(T),
	Many(Vec<T>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OpenXrInputChordMember {
	Path(String),
	Detailed {
		path: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		threshold: Option<[f32; 2]>,
	},
}

impl OpenXrInputChordMember {
	pub fn path(&self) -> &str {
		match self {
			Self::Path(path) | Self::Detailed { path, .. } => path,
		}
	}

	pub fn threshold(&self) -> Option<[f32; 2]> {
		match self {
			Self::Path(_) => None,
			Self::Detailed { threshold, .. } => *threshold,
		}
	}
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
	pub head: Option<OneOrMany<String>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub left_chord: Option<Vec<OpenXrInputChordMember>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub right_chord: Option<Vec<OpenXrInputChordMember>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub threshold_left: Option<[f32; 2]>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub threshold_right: Option<[f32; 2]>,
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
	#[serde(skip_serializing_if = "Option::is_none")]
	pub volume_up: Option<OpenXrInputAction>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub volume_down: Option<OpenXrInputAction>,
}

fn strip_axis_component(path: &str) -> String {
	if let Some((prefix, suffix)) = path.rsplit_once('/')
		&& (suffix == "x" || suffix == "y")
		&& (prefix.ends_with("/thumbstick") || prefix.ends_with("/trackpad") || prefix.ends_with("/joystick"))
	{
		return prefix.to_owned();
	}
	path.to_owned()
}

// TODO: remove this after thumbstick issue is not a thing anymore
fn normalize_profile(profile: &mut OpenXrInputProfile) {
	if let Some(action) = profile.pose.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.click.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.grab.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.alt_click.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.show_hide.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.toggle_dashboard.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.space_drag.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.space_rotate.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.space_reset.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.click_modifier_right.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.click_modifier_middle.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.move_mouse.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.scroll.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.haptic.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.volume_up.as_mut() {
		normalize_action(action);
	}
	if let Some(action) = profile.volume_down.as_mut() {
		normalize_action(action);
	}
}

fn normalize_action(action: &mut OpenXrInputAction) {
	let process_one_or_many = |opt: &mut Option<OneOrMany<String>>| {
		if let Some(paths) = opt {
			match paths {
				OneOrMany::One(p) => {
					let normalized = strip_axis_component(p);
					if normalized != *p {
						*p = normalized;
					}
				}
				OneOrMany::Many(ps) => {
					for p in ps.iter_mut() {
						let normalized = strip_axis_component(p);
						if normalized != *p {
							*p = normalized;
						}
					}
				}
			}
		}
	};

	process_one_or_many(&mut action.left);
	process_one_or_many(&mut action.right);
	process_one_or_many(&mut action.handsfree);
	process_one_or_many(&mut action.head);

	let process_chord = |members: &mut Option<Vec<OpenXrInputChordMember>>| {
		if let Some(m) = members {
			for member in m.iter_mut() {
				let path = member.path().to_owned();
				let normalized = strip_axis_component(&path);
				if normalized != path {
					let threshold = member.threshold();
					*member = if threshold.is_some() {
						OpenXrInputChordMember::Detailed {
							path: normalized,
							threshold,
						}
					} else {
						OpenXrInputChordMember::Path(normalized)
					};
				}
			}
		}
	};

	process_chord(&mut action.left_chord);
	process_chord(&mut action.right_chord);
}

pub fn load_xr_input_profiles() -> Vec<OpenXrInputProfile> {
	let mut profiles: Vec<OpenXrInputProfile> = serde_json5::from_str(DEFAULT_XR_INPUT_PROFILES).unwrap(); // want panic

	let Some(conf) = config_io::load("openxr_actions.json5") else {
		return profiles;
	};

	match serde_json5::from_str::<Vec<OpenXrInputProfile>>(&conf) {
		Ok(mut override_profiles) => {
			for new in override_profiles.iter_mut() {
				normalize_profile(new);
			}
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
