#![allow(dead_code)] // TODO: Remove later
use serde::Deserialize;
use wlx_common::async_executor::AsyncExecutor;

use crate::util::{http_client, networking};

pub type SkymapUuid = String;

#[derive(Clone, Debug, Deserialize)]
pub struct SkymapCatalogEntryFiles {
	pub size_16k: Option<String>, // "my_skymap_16k.png"
	pub size_8k: Option<String>,  // "my_skymap_8k.png"
	pub size_4k: Option<String>,  // "my_skymap_4k.png"
	pub size_2k: String,          // we should have *at least* this
	pub preview: String,
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

	let res = http_client::get(executor, &format!("{}/catalog.json", networking::WAYVR_SKYMAPS_ROOT)).await?;
	let catalog = res.as_json::<SkymapCatalog>()?;
	catalog.validate()?;

	Ok(catalog)
}
