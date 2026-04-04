use std::{fmt::Display, rc::Rc, str};

use anyhow::Context;

use crate::assets::{AssetProvider, LangProvider};

// a string which optionally has translation key in it
// it will hopefully support dynamic language changing soon
// for now it's just a simple string container
#[derive(Debug, Default, Clone)]
pub struct Translation {
	pub text: Rc<str>,
	pub translated: bool, // if true, `text` is a translation key
}

impl PartialEq for Translation {
	fn eq(&self, other: &Self) -> bool {
		*self.text == *other.text && self.translated == other.translated
	}
}

impl Translation {
	pub fn generate(&self, i18n: &mut I18n) -> Rc<str> {
		if self.translated {
			i18n.translate(&self.text)
		} else {
			self.text.clone()
		}
	}

	pub const fn from_raw_text_rc(text: Rc<str>) -> Self {
		Self {
			text,
			translated: false,
		}
	}

	pub fn from_raw_text_string(text: String) -> Self {
		Self {
			text: text.into(),
			translated: false,
		}
	}

	pub fn from_raw_text(text: &str) -> Self {
		Self {
			text: Rc::from(text),
			translated: false,
		}
	}

	pub fn from_translation_key(translated: &str) -> Self {
		Self {
			text: Rc::from(translated),
			translated: true,
		}
	}
}

pub struct Locale {
	lang: String,
	region: Option<String>,
	matched: String,
}

pub trait LangsList {
	fn all_locale(&self) -> &'static [&'static str]; // "en", "de", "es" (...)
	fn default_lang(&self) -> &'static str; // "en"
}

impl Locale {
	fn match_locale<'o>(
		default_lang: &'static str,
		lang: &str,
		region: Option<&str>,
		all_locales: &[&'o str],
	) -> &'o str {
		if let Some(region) = region {
			let locale_str = format!("{lang}_{region}");
			if let Some(locale) = all_locales.iter().find(|&&l| l == locale_str) {
				return locale;
			}
			log::warn!("Unsupported locale \"{locale_str}\", trying \"{lang}\".");
		}

		if let Some(locale) = all_locales.iter().find(|&&l| l == lang) {
			return locale;
		}

		let prefix = format!("{lang}_");
		if let Some(locale) = all_locales.iter().find(|&&l| l.starts_with(&prefix)) {
			return locale;
		}

		log::warn!("Unsupported language \"{lang}\", defaulting to \"{default_lang}\".");
		default_lang
	}

	pub fn new(langs_list: &dyn LangsList, lang: String, region: Option<String>) -> Self {
		let matched = Self::match_locale(
			langs_list.default_lang(),
			&lang,
			region.as_deref(),
			langs_list.all_locale(),
		)
		.to_string();
		Self { lang, region, matched }
	}

	pub fn parse_str(langs_list: &dyn LangsList, locale: &str) -> Self {
		let base = locale.split(['.', '@']).next().unwrap_or(locale);
		let parts: Vec<&str> = base.split(['_', '-']).collect();
		// Ensures the format is lang_REGION
		match parts.as_slice() {
			[lang, region, ..] => Self::new(langs_list, lang.to_lowercase(), Some(region.to_uppercase())),
			[lang] if !lang.is_empty() => Self::new(langs_list, lang.to_lowercase(), None),
			_ => Self::new(langs_list, langs_list.default_lang().to_string(), None),
		}
	}

	pub fn from_env(lang_provider: &dyn LangProvider) -> Self {
		let default_lang = lang_provider.langs_list().default_lang();

		// check if forced language is set
		if let Some(forced_lang) = lang_provider.forced_lang() {
			return Self::parse_str(lang_provider.langs_list(), forced_lang);
		}

		// fallback to environment variables
		use std::env;
		let vars = ["LC_ALL", "LC_MESSAGES", "LANG"];
		let full_locale = vars
			.iter()
			.find_map(|&v| env::var(v).ok())
			.filter(|v| !v.is_empty() && v != "C" && v != "POSIX")
			.unwrap_or_else(|| {
				log::warn!("LC_ALL/LC_MESSAGES/LANG is not set, defaulting to \"{default_lang}\"");
				default_lang.to_string()
			});
		Self::parse_str(lang_provider.langs_list(), &full_locale)
	}
	pub fn get_matched(&self) -> &str {
		&self.matched
	}
}

impl Display for Locale {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.lang)?;
		if let Some(ref region) = self.region {
			write!(f, "_{region}")
		} else {
			Ok(())
		}
	}
}

pub struct I18n {
	locale: Locale,
	json_root_translated: serde_json::Value, // any language
	json_root_fallback: serde_json::Value,   // english
}

fn find_translation<'a>(translation: &str, mut val: &'a serde_json::Value) -> Option<&'a str> {
	for part in translation.split('.') {
		if let Some(sub) = val.get(part) {
			val = sub;
		}
	}

	val.as_str()
}

impl I18n {
	pub fn new(asset_provider: &mut dyn AssetProvider, lang_provider: &dyn LangProvider) -> anyhow::Result<Self> {
		let locale = Locale::from_env(lang_provider);
		log::info!("Guessed system language: {locale} (matched: {})", locale.get_matched());

		let data_english = asset_provider.load_from_path("lang/en.json")?;
		let path = format!("lang/{}.json", locale.get_matched());
		let data_translated = asset_provider
			.load_from_path(&path)
			.with_context(|| path.clone())
			.context("Could not load translation file")?;

		let json_root_fallback = serde_json::from_str(
			str::from_utf8(&data_english)
				.with_context(|| path.clone())
				.context("Translation file not valid UTF-8")?,
		)
		.with_context(|| path.clone())
		.context("Translation file not valid JSON")?;

		let json_root_translated = serde_json::from_str(
			str::from_utf8(&data_translated)
				.with_context(|| path.clone())
				.context("Translation file not valid UTF-8")?,
		)
		.with_context(|| path.clone())
		.context("Translation file not valid JSON")?;

		Ok(Self {
			locale,
			json_root_translated,
			json_root_fallback,
		})
	}

	pub const fn get_locale(&self) -> &Locale {
		&self.locale
	}

	pub fn translate(&mut self, translation_key_full: &str) -> Rc<str> {
		let mut sections = translation_key_full.split(';');
		let translation_key = sections.next().map_or(translation_key_full, |a| a);

		if let Some(translated) = find_translation(translation_key, &self.json_root_translated) {
			return Rc::from(format_translated(translated, sections));
		}

		if let Some(translated_fallback) = find_translation(translation_key, &self.json_root_fallback) {
			log::warn!("missing translation for key \"{translation_key}\", using fallback instead");
			return Rc::from(format_translated(translated_fallback, sections));
		}

		log::error!("missing translation for key \"{translation_key}\"");
		Rc::from(translation_key) // show translation key as a fallback
	}

	pub fn translate_and_replace(&mut self, translation_key: &str, to_replace: (&str, &str)) -> String {
		let translated = self.translate(translation_key);
		translated.replace(to_replace.0, to_replace.1)
	}
}

fn format_translated<'a, I>(format: &str, args: I) -> String
where
	I: IntoIterator<Item = &'a str>,
{
	let mut result = String::new();
	let mut args = args.into_iter();

	let mut chars = format.chars().peekable();
	while let Some(c) = chars.next() {
		if c == '{' && chars.peek() == Some(&'}') {
			chars.next(); //consume }
			if let Some(arg) = args.next() {
				result.push_str(arg);
			} else {
				// no more args → keep literal {}
				result.push_str("{}");
			}
		} else {
			result.push(c);
		}
	}

	result
}
