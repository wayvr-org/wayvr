use std::rc::Rc;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumProperty, EnumString};
use wgui::i18n::Translation;

pub struct ControllerProfile {
	pub display_name: &'static str,
	pub profile_id: &'static str,
	pub user_paths: &'static [ControllerUserPath],
}

impl ControllerProfile {
	pub fn find_userpath(&self, side: Side) -> Option<&ControllerUserPath> {
		self.user_paths.iter().find(|x| x.hand == side)
	}
}

pub struct ControllerUserPath {
	pub hand: Side,
	pub paths: &'static [Subpath],
}

impl ControllerUserPath {
	pub fn find_subpath(&self, subpath: SubpathKind) -> Option<&Subpath> {
		self.paths.iter().find(|x| x.kind == subpath)
	}
}

pub struct Subpath {
	pub kind: SubpathKind,
	pub components: &'static [Component],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, EnumString, AsRefStr, EnumProperty)]
#[strum(ascii_case_insensitive)]
pub enum SubpathKind {
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
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.TYPE.MENU"))]
	Menu,

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

	#[strum(props(Hidden = true))]
	Grip,
	#[strum(props(Hidden = true))]
	Aim,
	#[strum(props(Hidden = true))]
	Haptic,
}

impl BindingsDropdown for SubpathKind {
	fn translation(&self) -> Translation {
		self
			.get_str("Translation")
			.map(Translation::from_translation_key)
			.or_else(|| self.get_str("Text").map(Translation::from_raw_text))
			.unwrap_or_else(|| Translation::from_raw_text(self.as_ref()))
	}
	fn action_str(&self, action: &str, side: Side) -> Rc<str> {
		let value = self.as_ref();
		let side = side.as_ref();
		format!("subpath;{action};{side};{value}").into()
	}
	fn clear_str(action: &str, side: Side) -> Option<Rc<str>> {
		let side = side.as_ref();
		Some(format!("clear;{action};{side};-").into())
	}
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, EnumString, AsRefStr, EnumProperty)]
#[serde(rename_all = "snake_case")]
#[strum(ascii_case_insensitive)]
pub enum Component {
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

impl Component {
	pub fn is_analog(&self) -> bool {
		matches!(self, Component::Force | Component::Value | Component::X | Component::Y)
	}
}

impl BindingsDropdown for Component {
	fn translation(&self) -> Translation {
		self
			.get_str("Translation")
			.map(Translation::from_translation_key)
			.or_else(|| self.get_str("Text").map(Translation::from_raw_text))
			.unwrap_or_else(|| Translation::from_raw_text(self.as_ref()))
	}
	fn action_str(&self, action: &str, side: Side) -> Rc<str> {
		let value = self.as_ref();
		let side = side.as_ref();
		format!("comp;{action};{side};{value}").into()
	}
	fn clear_str(_action: &str, _side: Side) -> Option<Rc<str>> {
		None
	}
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, EnumString, AsRefStr)]
#[serde(rename_all = "snake_case")]
#[strum(ascii_case_insensitive)]
pub enum Side {
	Left,
	Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParsedOpenXrInputPath {
	pub side: Side,
	pub subpath: SubpathKind,
	pub component: Component,
}

impl<'a> TryFrom<&'a str> for ParsedOpenXrInputPath {
	type Error = anyhow::Error;

	fn try_from(path: &str) -> anyhow::Result<Self> {
		let (side, rest) = if let Some(rest) = path.strip_prefix("/user/hand/left/") {
			(Side::Left, rest)
		} else if let Some(rest) = path.strip_prefix("/user/hand/right/") {
			(Side::Right, rest)
		} else {
			bail!("missing hand prefix");
		};

		let (input, rest) = rest.split_once('/').context("path too short")?;
		if input != "input" {
			bail!("missing input prefix");
		}

		let (identifier, component) = rest.rsplit_once('/').context("missing identifier or component")?;

		if identifier.is_empty() || component.is_empty() {
			bail!("identifier or component empty");
		}

		let component = Component::try_from(component).context("bad component")?;

		let identifier = SubpathKind::try_from(identifier).context("bad subpath")?;

		Ok(Self {
			side,
			subpath: identifier,
			component,
		})
	}
}

#[derive(Default, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, EnumString, AsRefStr, EnumProperty)]
#[strum(ascii_case_insensitive)]
pub enum ClickType {
	#[default]
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.CLICK.ANY"))]
	Any,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.CLICK.DOUBLE"))]
	Double,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.CLICK.TRIPLE"))]
	Triple,
}

impl BindingsDropdown for ClickType {
	fn translation(&self) -> Translation {
		self
			.get_str("Translation")
			.map(Translation::from_translation_key)
			.or_else(|| self.get_str("Text").map(Translation::from_raw_text))
			.unwrap_or_else(|| Translation::from_raw_text(self.as_ref()))
	}
	fn action_str(&self, action: &str, _side: Side) -> Rc<str> {
		let value = self.as_ref();
		format!("click;{action};-;{value}").into()
	}
	fn clear_str(_action: &str, _side: Side) -> Option<Rc<str>> {
		None
	}
}

pub trait BindingsDropdown {
	fn translation(&self) -> Translation;
	fn action_str(&self, action: &str, side: Side) -> Rc<str>;
	fn clear_str(action: &str, side: Side) -> Option<Rc<str>>;
}
