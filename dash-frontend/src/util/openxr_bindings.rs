use std::rc::Rc;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumProperty, EnumString};
use wgui::i18n::Translation;
use wlx_common::openxr_bindings_schema::{XrInputComponent, XrInputSide, XrInputSubpathKind};

#[derive(Default, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, EnumString, AsRefStr, EnumProperty)]
#[strum(ascii_case_insensitive, serialize_all = "snake_case")]
pub enum ClickType {
	#[default]
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.CLICK.ANY"))]
	Any,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.CLICK.DOUBLE"))]
	Double,
	#[strum(props(Translation = "APP_SETTINGS.BINDINGS.CLICK.TRIPLE"))]
	Triple,
}

pub trait BindingsDropdown {
	fn translation(&self) -> Translation;
	fn action_str(&self, action: &str, side: XrInputSide) -> Rc<str>;
	fn clear_str(action: &str, side: XrInputSide) -> Option<Rc<str>>;
}

impl BindingsDropdown for ClickType {
	fn translation(&self) -> Translation {
		self
			.get_str("Translation")
			.map(Translation::from_translation_key)
			.or_else(|| self.get_str("Text").map(Translation::from_raw_text))
			.unwrap_or_else(|| Translation::from_raw_text(self.as_ref()))
	}
	fn action_str(&self, action: &str, _side: XrInputSide) -> Rc<str> {
		let value = self.as_ref();
		format!("click;{action};-;{value}").into()
	}
	fn clear_str(_action: &str, _side: XrInputSide) -> Option<Rc<str>> {
		None
	}
}

impl BindingsDropdown for XrInputSubpathKind {
	fn translation(&self) -> Translation {
		self
			.get_str("Translation")
			.map(Translation::from_translation_key)
			.unwrap_or_else(|| {
				let mut chars = self.as_ref().chars();
				let capitalized = match chars.next() {
					None => String::new(),
					Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
				};
				Translation::from_raw_text(&capitalized)
			})
	}
	fn action_str(&self, action: &str, side: XrInputSide) -> Rc<str> {
		let value = self.as_ref();
		let side = side.as_ref();
		format!("subpath;{action};{side};{value}").into()
	}
	fn clear_str(action: &str, side: XrInputSide) -> Option<Rc<str>> {
		let side = side.as_ref();
		Some(format!("clear;{action};{side};-").into())
	}
}

impl BindingsDropdown for XrInputComponent {
	fn translation(&self) -> Translation {
		self
			.get_str("Translation")
			.map(Translation::from_translation_key)
			.or_else(|| self.get_str("Text").map(Translation::from_raw_text))
			.unwrap_or_else(|| Translation::from_raw_text(self.as_ref()))
	}
	fn action_str(&self, action: &str, side: XrInputSide) -> Rc<str> {
		let value = self.as_ref();
		let side = side.as_ref();
		format!("comp;{action};{side};{value}").into()
	}
	fn clear_str(_action: &str, _side: XrInputSide) -> Option<Rc<str>> {
		None
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParsedOpenXrInputPath {
	pub side: XrInputSide,
	pub subpath: XrInputSubpathKind,
	pub component: XrInputComponent,
}

impl<'a> TryFrom<&'a str> for ParsedOpenXrInputPath {
	type Error = anyhow::Error;

	fn try_from(path: &str) -> anyhow::Result<Self> {
		let (side, rest) = if let Some(rest) = path.strip_prefix("/user/hand/left/") {
			(XrInputSide::Left, rest)
		} else if let Some(rest) = path.strip_prefix("/user/hand/right/") {
			(XrInputSide::Right, rest)
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

		let component = XrInputComponent::try_from(component).context("bad component")?;
		let subpath = XrInputSubpathKind::try_from(identifier).context("bad subpath")?;

		Ok(Self {
			side,
			subpath,
			component,
		})
	}
}
