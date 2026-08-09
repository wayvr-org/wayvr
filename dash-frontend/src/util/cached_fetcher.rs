use std::time::{SystemTime, UNIX_EPOCH};

use crate::util::{networking::http_client, steam_utils::AppID};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr};
use wlx_common::cache_dir;

fn get_unix_timestamp() -> u64 {
	SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, AsRefStr)]
#[serde(rename_all = "snake_case")]
pub enum SupporterTier {
	Platinum,
	Gold,
	Silver,
	Bronze,
}

impl SupporterTier {
	pub fn pretty_str(self) -> String {
		format!("{self:?} Tier")
	}
	
	pub const fn color_str(self) -> &'static str {
		match self {
			Self::Platinum => "#aaffff",
			Self::Gold => "#ffffaa",
			Self::Silver => "#cccccc",
			Self::Bronze => "#ffaa66",
		}
	}

	pub const fn tickets(self) -> u32 {
		match self {
			Self::Platinum => 20,
			Self::Gold => 10,
			Self::Silver => 5,
			Self::Bronze => 1,
		}
	}
}

#[derive(Serialize, Deserialize)]
pub struct Supporter {
	pub username: String,
	pub date: String,
	pub tier: SupporterTier,
	pub contribution_count: u32,
}

impl Supporter {
	pub const fn tickets(&self) -> u32 {
		self.tier.tickets()
	}
}

#[derive(Serialize, Deserialize)]
pub struct Supporters {
	pub time_window_days: u32,
	pub supporters: Vec<Supporter>,
}

#[derive(Serialize, Deserialize)]
struct SupportersFile {
	fetch_timestamp: u64,
	supporters: Option<Supporters>, // empty in case of server/network failure
}

pub async fn request_supporters() -> Option<Supporters> {
	let cache_file_path = "supporters_v1.json";
	const CACHE_DURATION_SECS: u64 = 7200;
	let current_timestamp = get_unix_timestamp();

	if let Some(data) = cache_dir::get_data(cache_file_path).await
		&& let Ok(string) = str::from_utf8(&data)
		&& let Ok(file) = serde_json::from_str::<SupportersFile>(string)
		&& file.fetch_timestamp + CACHE_DURATION_SECS > current_timestamp
	{
		return file.supporters;
	}

	// perform request
	let mut supporters_file = SupportersFile {
		fetch_timestamp: current_timestamp,
		supporters: None,
	};

	let url = "https://wayvr.org/files/supporters_v1.json";
	if let Ok(res) = http_client::get_simple(url).await
		&& let Ok(supporters) = res.into_json::<Supporters>()
	{
		supporters_file.supporters = Some(supporters);
	}

	let json = serde_json::to_string_pretty(&supporters_file).unwrap() /* safe */;

	cache_dir::set_data(cache_file_path, json.as_bytes()).await.ok()?;

	supporters_file.supporters
}

pub struct CoverArt {
	// can be empty in case if data couldn't be fetched (use a fallback image then)
	pub compressed_image_data: Vec<u8>,
}

pub async fn request_cover_art(app_id: AppID) -> anyhow::Result<CoverArt> {
	let cache_file_path = format!("cover_arts/{}.bin", app_id);

	// check if file already exists in cache directory
	if let Some(data) = cache_dir::get_data(&cache_file_path).await {
		return Ok(CoverArt {
			compressed_image_data: data,
		});
	}

	let url = format!(
		"https://shared.steamstatic.com/store_item_assets/steam/apps/{}/library_600x900.jpg",
		app_id
	);

	match http_client::get_simple(&url).await {
		Ok(response) => {
			log::info!("Success");
			cache_dir::set_data(&cache_file_path, &response.data).await?;
			Ok(CoverArt {
				compressed_image_data: response.data,
			})
		}
		Err(e) => {
			// fetch failed, write an empty file
			log::error!("CoverArtFetcher: failed fetch for AppID {}: {}", app_id, e);
			cache_dir::set_data(&cache_file_path, &[]).await?;
			Ok(CoverArt {
				compressed_image_data: Vec::new(),
			})
		}
	}
}

#[derive(Deserialize, Clone)]
pub struct AppDetailsJSONData {
	#[allow(dead_code)]
	pub r#type: String, // "game"
	#[allow(dead_code)]
	pub name: String, // "Half-Life 3"
	#[allow(dead_code)]
	pub is_free: Option<bool>, // "false"
	pub detailed_description: Option<String>, //
	pub short_description: Option<String>,    //
	pub developers: Vec<String>,              // ["Valve"]
}

async fn get_app_details_json_internal(cache_file_path: &str, app_id: AppID) -> anyhow::Result<AppDetailsJSONData> {
	// check if file already exists in cache directory
	if let Some(data) = cache_dir::get_data(cache_file_path).await {
		return Ok(serde_json::from_value(serde_json::from_slice(&data)?)?);
	}

	// Fetch from Steam API
	log::info!("Fetching app detail ID {}", app_id);
	let url = format!("https://store.steampowered.com/api/appdetails?appids={}", app_id);
	let response = http_client::get_simple(&url).await?;
	let res_utf8 = String::from_utf8(response.data)?;
	let root = serde_json::from_str::<serde_json::Value>(&res_utf8)?;
	let body = root.get(&app_id).context("invalid body")?;

	if !body.get("success").is_some_and(|v| v.as_bool().unwrap_or(false)) {
		anyhow::bail!("Failed");
	}

	let data = body.get("data").context("data null")?;

	let data_bytes = serde_json::to_vec(&data)?;
	let app_details: AppDetailsJSONData = serde_json::from_value(data.clone())?;

	// cache for future use
	cache_dir::set_data(cache_file_path, &data_bytes).await?;

	Ok(app_details)
}

pub async fn get_app_details_json(app_id: AppID) -> Option<AppDetailsJSONData> {
	let cache_file_path = format!("app_details/{}.json", app_id);

	match get_app_details_json_internal(&cache_file_path, app_id).await {
		Ok(r) => Some(r),
		Err(e) => {
			log::error!("Failed to get app details: {:?}", e);
			let _ = cache_dir::set_data(&cache_file_path, &[]).await; // write empty data
			None
		}
	}
}
