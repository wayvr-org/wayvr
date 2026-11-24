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

#[derive(Clone)]
pub enum BindingsDropdownAction {
	Click {
		action: Rc<str>,
		click_type: ClickType,
	},
	Subpath {
		action: Rc<str>,
		side: XrInputSide,
		subpath: XrInputSubpathKind,
	},
	Comp {
		action: Rc<str>,
		side: XrInputSide,
		comp: XrInputComponent,
	},
	Clear {
		action: Rc<str>,
		side: XrInputSide,
	},
}

pub trait BindingsDropdown {
	fn translation(&self) -> Translation;
	fn get_action(&self, action: &str, side: XrInputSide) -> BindingsDropdownAction;
	fn get_clear(action: &str, side: XrInputSide) -> Option<BindingsDropdownAction>;
}

impl BindingsDropdown for ClickType {
	fn translation(&self) -> Translation {
		self
			.get_str("Translation")
			.map(Translation::from_translation_key)
			.or_else(|| self.get_str("Text").map(Translation::from_raw_text))
			.unwrap_or_else(|| Translation::from_raw_text(self.as_ref()))
	}
	fn get_action(&self, action: &str, _side: XrInputSide) -> BindingsDropdownAction {
		BindingsDropdownAction::Click {
			action: action.into(),
			click_type: *self,
		}
	}
	fn get_clear(_action: &str, _side: XrInputSide) -> Option<BindingsDropdownAction> {
		None
	}
}

impl BindingsDropdown for XrInputSubpathKind {
	fn translation(&self) -> Translation {
		self
			.get_str("Translation")
			.map(Translation::from_translation_key)
			.unwrap_or_else(|| Translation::from_raw_text(wlx_common::locale::capitalize_string(self.as_ref())))
	}
	fn get_action(&self, action: &str, side: XrInputSide) -> BindingsDropdownAction {
		BindingsDropdownAction::Subpath {
			action: action.into(),
			side,
			subpath: *self,
		}
	}
	fn get_clear(action: &str, side: XrInputSide) -> Option<BindingsDropdownAction> {
		Some(BindingsDropdownAction::Clear {
			action: action.into(),
			side,
		})
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
	fn get_action(&self, action: &str, side: XrInputSide) -> BindingsDropdownAction {
		BindingsDropdownAction::Comp {
			action: action.into(),
			side,
			comp: *self,
		}
	}
	fn get_clear(_action: &str, _side: XrInputSide) -> Option<BindingsDropdownAction> {
		None
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParsedOpenXrInputPath {
	pub side: XrInputSide,
	pub subpath: XrInputSubpathKind,
	pub component: Option<XrInputComponent>,
}

impl TryFrom<&str> for ParsedOpenXrInputPath {
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

		let (identifier, component) = match rest.rsplit_once('/') {
			Some((ident, comp)) if !comp.is_empty() => {
				let comp = XrInputComponent::try_from(comp).context("bad component")?;
				(ident, Some(comp))
			}
			_ => (rest, None),
		};

		if identifier.is_empty() {
			bail!("identifier empty");
		}

		let subpath = XrInputSubpathKind::try_from(identifier).context("bad subpath")?;

		Ok(Self {
			side,
			subpath,
			component,
		})
	}
}
