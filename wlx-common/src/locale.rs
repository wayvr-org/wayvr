use std::rc::Rc;

use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumProperty, EnumString, VariantArray};

use crate::config::GeneralConfig;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, AsRefStr, EnumString, EnumProperty, VariantArray)]
pub enum Language {
	#[strum(props(CountryCode = "en", Text = "English"))]
	English,
	#[strum(props(CountryCode = "en_uk", Text = "English (UK)"))]
	British,
	#[strum(props(CountryCode = "pl", Text = "Polski"))]
	Polish,
	#[strum(props(CountryCode = "ja", Text = "日本語"))]
	Japanese,
	#[strum(props(CountryCode = "de", Text = "German"))]
	German,
	#[strum(props(CountryCode = "it", Text = "Italiano"))]
	Italian,
	#[strum(props(CountryCode = "cn", Text = "简体中文"))]
	ChineseSimplified,
	#[strum(props(CountryCode = "es", Text = "Español"))]
	Spanish,
	#[strum(props(CountryCode = "se", Text = "Svenska"))]
	Swedish,
}

impl Language {
	pub const fn code(&self) -> &'static str {
		match self {
			Language::English => "en",
			Language::British => "en_GB",
			Language::Polish => "pl_PL",
			Language::Japanese => "ja_JP",
			Language::German => "de_DE",
			Language::Italian => "it_IT",
			Language::ChineseSimplified => "zh_CN",
			Language::Spanish => "es_ES",
			Language::Swedish => "sv_SE",
		}
	}

	pub const fn get_default() -> Self {
		Self::English
	}

	pub const fn all_codes() -> &'static [&'static str] {
		&["en", "pl", "ja", "de", "it", "zh_CN", "es", "sv"]
	}
}

pub struct WayVRLangsList {}

impl wgui::i18n::LangsList for WayVRLangsList {
	fn all_locale(&self) -> &'static [&'static str] {
		Language::all_codes()
	}

	fn default_lang(&self) -> &'static str {
		Language::get_default().code()
	}
}

// static
const G_LANGS_LIST: WayVRLangsList = WayVRLangsList {};

#[derive(Default)]
pub struct WayVRLangProvider {
	forced_lang: Option<Rc<str>>,
}

impl wgui::assets::LangProvider for WayVRLangProvider {
	fn langs_list(&self) -> &dyn wgui::i18n::LangsList {
		&G_LANGS_LIST
	}

	fn forced_lang(&self) -> Option<&str> {
		self.forced_lang.as_ref().map(|lang| lang.as_ref())
	}
}

impl WayVRLangProvider {
	pub fn from_config(config: &GeneralConfig) -> Self {
		if let Some(lang) = &config.language {
			return Self {
				forced_lang: Some(lang.code().into()),
			};
		}

		Self::default()
	}
}

pub fn capitalize_string(s: &str) -> String {
	let mut chars = s.chars();
	match chars.next() {
		None => String::new(),
		Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
	}
}
