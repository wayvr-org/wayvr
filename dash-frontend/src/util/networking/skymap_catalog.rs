#![allow(dead_code)]
use std::path::PathBuf;

// TODO: Remove later
use serde::{Deserialize, Serialize};
use wlx_common::{async_executor::AsyncExecutor, config_io};

use crate::util::networking::{self, WAYVR_SKYMAPS_ROOT, http_client};

pub type SkymapUuid = uuid::Uuid;

#[derive(Copy, Clone, Serialize, Deserialize, Debug)]
pub enum SkymapResolution {
	Res2k,
	Res4k,
	Res8k,
}

impl SkymapResolution {
	pub const fn get_display_str(&self) -> &'static str {
		match self {
			SkymapResolution::Res2k => "2K (2 MiB VRAM)",
			SkymapResolution::Res4k => "4K (8 MiB VRAM)",
			SkymapResolution::Res8k => "8K (33 MiB VRAM)",
		}
	}

	pub const fn get_display_str_simple(&self) -> &'static str {
		match self {
			SkymapResolution::Res2k => "2K",
			SkymapResolution::Res4k => "4K",
			SkymapResolution::Res8k => "8K",
		}
	}

	pub fn from_display_str_simple(text: &str) -> Option<SkymapResolution> {
		match text {
			"2K" => Some(SkymapResolution::Res2k),
			"4K" => Some(SkymapResolution::Res4k),
			"8K" => Some(SkymapResolution::Res8k),
			_ => None,
		}
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkymapCatalogEntryFiles {
	pub size_8k: Option<String>, // "my_skymap_8k.dds"
	pub size_4k: Option<String>, // "my_skymap_4k.dds"
	pub size_2k: String,         // we should have *at least* this
	pub preview: String,
}

impl SkymapCatalogEntryFiles {
	pub fn get_url_preview(&self) -> String {
		format!("{}/files/{}", WAYVR_SKYMAPS_ROOT, self.preview)
	}

	pub fn get_filename_from_res(&self, res: SkymapResolution) -> Option<String> {
		match res {
			SkymapResolution::Res2k => Some(&self.size_2k),
			SkymapResolution::Res4k => self.size_4k.as_ref(),
			SkymapResolution::Res8k => self.size_8k.as_ref(),
		}
		.map(|raw_filename| {
			// sanitize filename, do not allow "../" just in case
			PathBuf::from(raw_filename)
				.file_name()
				.map(|s| String::from(s.to_string_lossy()))
		})?
	}

	// example result: "https://wayvr.org/skymaps/files/my_skymap_8k.dds"
	pub fn get_url_from_res(&self, res: SkymapResolution) -> Option<String> {
		let Some(filename) = self.get_filename_from_res(res) else {
			return None;
		};

		Some(format!("{}/files/{}", WAYVR_SKYMAPS_ROOT, filename))
	}

	pub fn get_preview_path(&self) -> PathBuf {
		config_io::get_skymaps_root().join(&self.preview)
	}

	pub fn save_preview_to_file(&self, data: &[u8]) -> anyhow::Result<()> {
		std::fs::write(self.get_preview_path(), data)?;
		Ok(())
	}

	pub fn remove_preview_file(&self) {
		let _dont_care = std::fs::remove_file(self.get_preview_path());
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkymapCatalogEntry {
	pub uuid: SkymapUuid,
	pub created_at: String,
	pub modified_at: String,
	pub version: u32,
	pub name: String,
	pub description: String,
	pub author: String,
	pub files: SkymapCatalogEntryFiles,
}

impl SkymapCatalogEntry {
	pub fn get_destination_path(&self, resolution: SkymapResolution) -> Option<PathBuf> {
		let Some(filename) = self.files.get_filename_from_res(resolution) else {
			return None;
		};

		Some(config_io::get_skymaps_root().join(filename))
	}

	pub fn get_destination_metadata_path(&self) -> PathBuf {
		config_io::get_skymaps_root().join(format!("{}.json", self.uuid))
	}

	pub fn is_downloaded(&self, resolution: SkymapResolution) -> anyhow::Result<bool> {
		let Some(full_path) = self.get_destination_path(resolution) else {
			return Ok(false);
		};

		Ok(std::fs::exists(full_path)?)
	}

	pub fn has_any_downloaded(&self) -> bool {
		self.is_downloaded(SkymapResolution::Res2k).unwrap_or(false)
			|| self.is_downloaded(SkymapResolution::Res4k).unwrap_or(false)
			|| self.is_downloaded(SkymapResolution::Res8k).unwrap_or(false)
	}

	pub fn remove_file(&self, resolution: SkymapResolution) {
		let Some(full_path) = self.get_destination_path(resolution) else {
			return;
		};

		let _dont_care = std::fs::remove_file(full_path);
	}

	pub fn save_metadata(&self) -> anyhow::Result<()> {
		let json = serde_json::to_string_pretty(self)?;
		std::fs::write(self.get_destination_metadata_path(), json)?;
		Ok(())
	}

	pub fn remove_metadata(&self) {
		let _dont_care = std::fs::remove_file(self.get_destination_metadata_path());
	}
}

#[derive(Clone, Debug, Deserialize)]
pub struct SkymapCatalog {
	pub version: u32,
	pub r#type: String,
	pub entries: Vec<SkymapCatalogEntry>,
}

impl SkymapCatalog {
	fn validate(&self) -> anyhow::Result<()> {
		if self.version != 1 {
			anyhow::bail!("Unsupported version");
		}

		if self.r#type != "wayvr_skymaps" {
			anyhow::bail!("Unsupported type");
		}
		Ok(())
	}
}

pub async fn request_catalog(executor: &AsyncExecutor) -> anyhow::Result<SkymapCatalog> {
	log::info!("Fetching skymap list");

	let res = http_client::get_simple(executor, &format!("{}/catalog.json", networking::WAYVR_SKYMAPS_ROOT)).await?;
	let catalog = res.as_json::<SkymapCatalog>()?;
	catalog.validate()?;

	Ok(catalog)
}

pub fn get_entries_from_disk() -> anyhow::Result<Vec<SkymapCatalogEntry>> {
	let mut entries = Vec::<SkymapCatalogEntry>::new();

	let skymaps_root = config_io::get_skymaps_root();

	for uuid in config_io::get_skymaps_uuids().unwrap_or_default() {
		let metadata_path = skymaps_root.join(format!("{}.json", uuid));
		let Ok(data) = std::fs::read_to_string(metadata_path) else {
			continue;
		};

		let entry = serde_json::from_str::<SkymapCatalogEntry>(&data)?;
		entries.push(entry);
	}

	Ok(entries)
}
