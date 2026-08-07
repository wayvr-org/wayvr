use std::{fs::File, io::BufReader};

use serde::{Deserialize, Deserializer, de::Error as _};
use wgui::{
	drawing,
	log::LogErr,
	palette::{PALETTES, WguiColorPalette},
};

use crate::config_io;

pub fn load_palette(want_palette: &str) -> WguiColorPalette {
	if is_custom_palette(want_palette) {
		if let Ok(palette) = load_custom_palette(want_palette).log_warn("Could not load custom color palette") {
			return palette;
		}
	} else {
		for (name, palette) in PALETTES {
			if *name == want_palette {
				return (*palette).clone();
			}
		}
		log::warn!("No color palette found with name: {want_palette}")
	}
	PALETTES[0].1.clone()
}

fn is_custom_palette(name: &str) -> bool {
	name
		.as_bytes()
		.get(name.len().saturating_sub(5)..)
		.is_some_and(|suffix| suffix.eq_ignore_ascii_case(b".json"))
}

pub fn list_palette_files() -> Vec<String> {
	let path = config_io::get_palettes_root();
	let Ok(entries) = std::fs::read_dir(path) else {
		return Vec::new();
	};

	let mut files: Vec<_> = entries
		.filter_map(Result::ok)
		.filter_map(|entry| {
			let path = entry.path();

			if !path.is_file() || !path.extension()?.to_str()?.eq_ignore_ascii_case("json") {
				return None;
			}

			Some(path.file_name()?.to_str()?.to_owned())
		})
		.collect();

	files.sort_unstable();
	files
}

pub fn load_custom_palette(name: &str) -> anyhow::Result<WguiColorPalette> {
	let path = config_io::get_palettes_root().join(name);
	let file = File::open(path)?;
	let reader = BufReader::new(file);
	let scp = serde_json::from_reader::<_, SerializedWguiColorPalette>(reader)?;
	Ok(scp.into_palette())
}

fn deserialize_color<'de, D>(deserializer: D) -> Result<drawing::Color, D::Error>
where
	D: Deserializer<'de>,
{
	let value = String::deserialize(deserializer)?;

	drawing::Color::from_hex(&value).ok_or_else(|| D::Error::custom(format!("invalid hexadecimal color: {value:?}")))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SerializedWguiColorPalette {
	#[serde(deserialize_with = "deserialize_color")]
	primary: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	on_primary: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	secondary: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	on_secondary: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	tertiary: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	on_tertiary: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	danger: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	on_danger: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	background: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	on_background: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	background_variant: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	on_background_variant: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	background_contrast: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	on_background_contrast: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	outline: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	shadow: drawing::Color,

	#[serde(deserialize_with = "deserialize_color")]
	highlight: drawing::Color,
}

impl SerializedWguiColorPalette {
	pub fn into_palette(self) -> WguiColorPalette {
		// order must stay the same as per WguiColorName
		WguiColorPalette::from_inner([
			self.primary,
			self.on_primary,
			self.secondary,
			self.on_secondary,
			self.tertiary,
			self.on_tertiary,
			self.danger,
			self.on_danger,
			self.background,
			self.on_background,
			self.background_variant,
			self.on_background_variant,
			self.background_contrast,
			self.on_background_contrast,
			self.outline,
			self.shadow,
			self.highlight,
		])
	}
}
