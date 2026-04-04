use std::rc::Rc;

use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumProperty, EnumString, VariantArray};

use crate::config::GeneralConfig;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, AsRefStr, EnumString, EnumProperty, VariantArray)]
pub enum Language {
	#[strum(props(Text = "English"))]
	English,
	#[strum(props(Text = "English (UK)"))]
	British,
	#[strum(props(Text = "Polski"))]
	Polish,
	#[strum(props(Text = "日本語"))]
	Japanese,
	#[strum(props(Text = "German"))]
	German,
	#[strum(props(Text = "Italiano"))]
	Italian,
	#[strum(props(Text = "简体中文"))]
	ChineseSimplified,
	#[strum(props(Text = "Español"))]
	Spanish,
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
		}
	}

	pub const fn get_default() -> Self {
		Self::English
	}

	pub const fn all_codes() -> &'static [&'static str] {
		&["en", "pl", "ja", "de", "it", "zh_CN", "es"]
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
