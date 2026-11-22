use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumProperty, EnumString, VariantArray};

use crate::config_io;

#[derive(
	Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize, AsRefStr, EnumString, EnumProperty, VariantArray,
)]
pub enum KeyboardModel {
	#[strum(props(Text = "ANSI"))]
	Ansi,
	#[default]
	#[strum(props(Text = "ISO"))]
	Iso,
	#[strum(props(Text = "JIS"))]
	Jis,
	#[strum(props(Translation = "APP_SETTINGS.OPTION.CUSTOM"))]
	Custom,
}

impl KeyboardModel {
	pub fn layout_yaml(&self) -> anyhow::Result<Cow<'static, str>> {
		match self {
			KeyboardModel::Ansi => Ok(Cow::Borrowed(include_str!("res/keyboard-pc104.yaml"))),
			KeyboardModel::Iso => Ok(Cow::Borrowed(include_str!("res/keyboard-pc105.yaml"))),
			KeyboardModel::Jis => Ok(Cow::Borrowed(include_str!("res/keyboard-jp106.yaml"))),
			KeyboardModel::Custom => {
				let path = config_io::get_config_file_path("keyboard.yaml");
				let yaml =
					std::fs::read_to_string(&path).map_err(|e| anyhow::anyhow!("Could not read {}: {e}", path.display()))?;
				Ok(Cow::Owned(yaml))
			}
		}
	}
}

pub const KEYBOARD_LAYOUTS: &[(&str, &str)] = &[
	("US English", "us"),
	("US Colemak", "us(colemak)"),
	("US Colemak-DH", "us(colemak_dh)"),
	("US Dvorak", "us(dvorak)"),
	("US International", "us(intl)"),
	("US International (AltGr)", "us(altgr-intl)"),
	("Arabic", "ara"),
	("Australian English", "au"),
	("Belgian", "be"),
	("Brazilian Portuguese", "br"),
	("Bulgarian", "bg"),
	("Canadian French", "ca"),
	("Canadian Multilingual", "ca(multix)"),
	("Croatian", "hr"),
	("Czech", "cz"),
	("Czech (QWERTY)", "cz(qwerty)"),
	("Danish", "dk"),
	("Dutch", "nl"),
	("Estonian", "ee"),
	("Finnish", "fi"),
	("French", "fr"),
	("French (AFNOR)", "fr(afnor)"),
	("French (BÉPO)", "fr(bepo)"),
	("German", "de"),
	("German (No Dead Keys)", "de(nodeadkeys)"),
	("Greek", "gr"),
	("Hebrew", "il"),
	("Hindi", "in"),
	("Hungarian", "hu"),
	("Icelandic", "is"),
	("Irish", "ie"),
	("Italian", "it"),
	("Japanese", "jp"),
	("Korean", "kr"),
	("Latvian", "lv"),
	("Lithuanian", "lt"),
	("Norwegian", "no"),
	("Polish", "pl"),
	("Portuguese", "pt"),
	("Romanian", "ro"),
	("Russian", "ru"),
	("Russian (Phonetic)", "ru(phonetic)"),
	("Serbian", "rs"),
	("Slovak", "sk"),
	("Slovenian", "si"),
	("Spanish", "es"),
	("Spanish (Latin American)", "latam"),
	("Swedish", "se"),
	("Swiss French", "ch(fr)"),
	("Swiss German", "ch"),
	("Thai", "th"),
	("Turkish F", "tr(f)"),
	("Turkish Q", "tr"),
	("UK English", "gb"),
	("Ukrainian", "ua"),
	("Vietnamese", "vn"),
];
