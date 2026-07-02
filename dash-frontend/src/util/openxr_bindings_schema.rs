use std::{collections::BTreeMap, io::Read, rc::Rc};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumProperty, EnumString};
use wgui::i18n::Translation;

static BINDINGS_LZ4: &[u8] = include_bytes!("../../assets/bindings.json.lz4");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingsFile {
	#[serde(rename = "$schema")]
	pub schema: Option<String>,

	pub profiles: BTreeMap<String, Rc<Profile>>,
}

impl BindingsFile {
	pub fn load_embedded() -> Self {
		let mut decoder = lz4_flex::frame::FrameDecoder::new(BINDINGS_LZ4);
		let mut json = Vec::new();
		decoder.read_to_end(&mut json).unwrap(); // safe

		serde_json::from_slice(&json).unwrap() // safe
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
	pub title: Rc<str>,

	#[serde(rename = "type")]
	pub kind: ProfileType,

	pub steamvr_controllertype: Option<String>,
	pub monado_device: Option<String>,

	#[serde(default)]
	pub extended_by: Vec<String>,

	#[serde(default)]
	pub subaction_paths: Vec<String>,

	#[serde(default)]
	pub subpaths: BTreeMap<String, Subpath>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileType {
	TrackedController,

	#[serde(other)]
	Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subpath {
	#[serde(rename = "type")]
	pub kind: SubpathType,

	pub localized_name: String,

	#[serde(default)]
	pub components: Vec<Component>,

	pub side: Option<Side>,
}

impl Subpath {
	pub fn get_effective_components(&self) -> Rc<[Component]> {
		let mut v = vec![];
		for c in self.components.iter() {
			match c {
				// position is not an openxr component, it's just a monado thing
				Component::Position => {
					v.push(Component::X);
					v.push(Component::Y);
				}
				Component::Other => {}
				other => v.push(*other),
			}
		}
		v.into()
	}
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SubpathType {
	Button,
	Trigger,
	Joystick,
	Pose,
	Trackpad,
	Vibration,

	#[serde(other)]
	Other,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, EnumString, AsRefStr, EnumProperty)]
#[strum(ascii_case_insensitive)]
pub enum IdentifierType {
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
}

impl BindingsDropdown for IdentifierType {
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
		Some(format!("subpath;{action};{side};-").into())
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

	/// Not an actual component but monado uses this instead of X/Y
	Position,
	Pose,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.COMP.PROXIMITY"))]
	Proximity,
	Haptic,

	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.COMP.X_AXIS"))]
	X,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.COMP.Y_AXIS"))]
	Y,

	#[serde(other)]
	Other,
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
pub enum Side {
	Left,
	Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParsedOpenXrInputPath {
	pub side: Side,
	pub identifier: IdentifierType,
	pub component: Component,
}

impl ParsedOpenXrInputPath {
	pub fn to_subpath(&self) -> String {
		format!("/input/{}", self.identifier.as_ref().to_lowercase())
	}
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

		let identifier = IdentifierType::try_from(identifier).context("bad subpath")?;

		Ok(Self {
			side,
			identifier,
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
