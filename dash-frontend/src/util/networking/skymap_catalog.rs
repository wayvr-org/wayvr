#![allow(dead_code)] // TODO: Remove later
use serde::Deserialize;
use wlx_common::async_executor::AsyncExecutor;

use crate::util::{http_client, networking};

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
	pub uuid: String,
	pub created_at: String,
	pub modified_at: String,
	pub entry_version: u32,
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

pub async fn request_catalog(executor: &AsyncExecutor) -> anyhow::Result<SkymapCatalog> {
	log::info!("Fetching skymap list");

	let res = http_client::get(executor, &format!("{}/catalog.json", networking::WAYVR_SKYMAPS_ROOT)).await?;
	let catalog = res.as_json::<SkymapCatalog>()?;

	Ok(catalog)
}
