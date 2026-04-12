#![allow(dead_code)]
use std::path::PathBuf;

// TODO: Remove later
use serde::Deserialize;
use wlx_common::async_executor::AsyncExecutor;

use crate::util::networking::{self, WAYVR_SKYMAPS_ROOT, http_client};

pub type SkymapUuid = uuid::Uuid;

#[derive(Copy, Clone)]
pub enum SkymapResolution {
	Res2k,
	Res4k,
	Res8k,
}

impl SkymapResolution {
	pub const fn get_display_str(&self) -> &'static str {
		match self {
			SkymapResolution::Res2k => "2K (6 MiB VRAM)",
			SkymapResolution::Res4k => "4K (24 MiB VRAM)",
			SkymapResolution::Res8k => "8K (96 MiB VRAM)",
		}
	}
}

#[derive(Clone, Debug, Deserialize)]
pub struct SkymapCatalogEntryFiles {
	pub size_8k: Option<String>, // "my_skymap_8k.png"
	pub size_4k: Option<String>, // "my_skymap_4k.png"
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

	// example result: "https://wayvr.org/skymaps/files/my_skymap_8k.png"
	pub fn get_url_from_res(&self, res: SkymapResolution) -> Option<String> {
		let Some(filename) = self.get_filename_from_res(res) else {
			return None;
		};

		Some(format!("{}/files/{}", WAYVR_SKYMAPS_ROOT, filename))
	}
}

#[derive(Clone, Debug, Deserialize)]
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
