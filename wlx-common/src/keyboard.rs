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

// display name, layout code, language code
pub const KEYBOARD_LAYOUTS: &[(&str, &str, &str)] = &[
	("US English", "us", "us"),
	("US Colemak", "us(colemak)", "us"),
	("US Colemak-DH", "us(colemak_dh)", "us"),
	("US Dvorak", "us(dvorak)", "us"),
	("US International", "us(intl)", "us"),
	("US Intl. AltGr", "us(altgr-intl)", "us"),
	("Arabic", "ara", "sa"),
	("Australian English", "au", "au"),
	("Belgian", "be", "be"),
	("Brazilian Portuguese", "br", "br"),
	("Bulgarian", "bg", "bg"),
	("Canadian French", "ca", "ca"),
	("Canadian Multilingual", "ca(multix)", "ca"),
	("Croatian", "hr", "hr"),
	("Czech", "cz", "cz"),
	("Czech QWERTY", "cz(qwerty)", "cz"),
	("Danish", "dk", "dk"),
	("Dutch", "nl", "nl"),
	("Estonian", "ee", "ee"),
	("Finnish", "fi", "fi"),
	("French", "fr", "fr"),
	("French AFNOR", "fr(afnor)", "fr"),
	("French BÉPO", "fr(bepo)", "fr"),
	("German", "de", "de"),
	("German No-Dead-Keys", "de(nodeadkeys)", "de"),
	("Greek", "gr", "gr"),
	("Hebrew", "il", "il"),
	("Hindi", "in", "in"),
	("Hungarian", "hu", "hu"),
	("Icelandic", "is", "is"),
	("Irish", "ie", "ie"),
	("Italian", "it", "it"),
	("Japanese", "jp", "ja"),
	("Korean", "kr", "kr"),
	("Latin Amer. Spanish", "latam", "es"),
	("Latvian", "lv", "lv"),
	("Lithuanian", "lt", "lt"),
	("Norwegian", "no", "no"),
	("Polish", "pl", "pl"),
	("Portuguese", "pt", "pt"),
	("Romanian", "ro", "ro"),
	("Russian", "ru", "ru"),
	("Russian Phonetic", "ru(phonetic)", "ru"),
	("Serbian", "rs", "rs"),
	("Slovak", "sk", "sk"),
	("Slovenian", "si", "si"),
	("Spanish", "es", "es"),
	("Swedish", "se", "se"),
	("Swiss French", "ch(fr)", "ch"),
	("Swiss German", "ch", "ch"),
	("Thai", "th", "th"),
	("Turkish F", "tr(f)", "tr"),
	("Turkish Q", "tr", "tr"),
	("UK English", "gb", "en_uk"),
	("Ukrainian", "ua", "ua"),
	("Vietnamese", "vn", "vn"),
];
