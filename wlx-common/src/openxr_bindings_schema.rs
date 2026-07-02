use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumProperty, EnumString};

pub const DEFAULT_BUTTON_THRESHOLDS: [f32; 2] = [0.5, 0.7];

pub struct XrControllerProfile {
	pub display_name: &'static str,
	pub profile_id: &'static str,
	pub extension: Option<&'static str>,
	pub user_paths: &'static [XrControllerUserPath],
}

impl XrControllerProfile {
	pub fn find_userpath(&self, side: XrInputSide) -> Option<&XrControllerUserPath> {
		self.user_paths.iter().find(|x| x.hand == side)
	}
}

pub struct XrControllerUserPath {
	pub hand: XrInputSide,
	pub paths: &'static [XrInputSubpath],
}

impl XrControllerUserPath {
	pub fn find_subpath(&self, subpath: XrInputSubpathKind) -> Option<&XrInputSubpath> {
		self.paths.iter().find(|x| x.kind == subpath)
	}
}

pub struct XrInputSubpath {
	pub kind: XrInputSubpathKind,
	pub components: &'static [XrInputComponent],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, EnumString, AsRefStr, EnumProperty)]
#[strum(ascii_case_insensitive, serialize_all = "snake_case")]
pub enum XrInputSubpathKind {
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.TYPE.TRIGGER"))]
	Trigger,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.TYPE.TRACKPAD"))]
	Trackpad,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.TYPE.THUMBSTICK"))]
	Thumbstick,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.TYPE.JOYSTICK"))]
	Joystick,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.TYPE.SYSTEM"))]
	System,
	Menu,
	View,

	Primary,
	Secondary,

	A,
	B,
	X,
	Y,
	Start,
	Home,
	End,
	Select,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.TYPE.THUMBREST"))]
	Thumbrest,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.TYPE.SHOULDER"))]
	Shoulder,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.TYPE.SQUEEZE"))]
	Squeeze,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.TYPE.BUMPER"))]
	Bumper,

	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.TYPE.DPAD_UP"))]
	DpadUp,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.TYPE.DPAD_DOWN"))]
	DpadDown,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.TYPE.DPAD_LEFT"))]
	DpadLeft,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.TYPE.DPAD_RIGHT"))]
	DpadRight,

	#[strum(props(Hidden = true))]
	Grip,
	#[strum(props(Hidden = true))]
	Aim,
	#[strum(props(Hidden = true))]
	Haptic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, EnumString, AsRefStr, EnumProperty)]
#[serde(rename_all = "snake_case")]
#[strum(ascii_case_insensitive, serialize_all = "snake_case")]
pub enum XrInputComponent {
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.COMP.CLICK"))]
	Click,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.COMP.FORCE"))]
	Force,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.COMP.TOUCH"))]
	Touch,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.COMP.VALUE"))]
	Value,

	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.COMP.PROXIMITY"))]
	Proximity,

	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.COMP.X_AXIS"))]
	X,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.COMP.Y_AXIS"))]
	Y,

	// below are hidden
	Pose,
}

impl XrInputComponent {
	pub fn is_analog(&self) -> bool {
		matches!(
			self,
			XrInputComponent::Force | XrInputComponent::Value | XrInputComponent::X | XrInputComponent::Y
		)
	}
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, EnumString, AsRefStr)]
#[serde(rename_all = "snake_case")]
#[strum(ascii_case_insensitive, serialize_all = "snake_case")]
pub enum XrInputSide {
	Left,
	Right,
}
