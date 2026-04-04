use log::error;
use std::{path::PathBuf, sync::LazyLock};

pub enum ConfigRoot {
	Generic,
}

const FALLBACK_CONFIG_PATH: &str = "/tmp/wayvr";

static CONFIG_ROOT_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
	if let Some(mut dir) = xdg::BaseDirectories::new().get_config_home() {
		dir.push("wayvr");
		return dir;
	}
	//Return fallback config path
	error!("Err: Failed to find config path, using {FALLBACK_CONFIG_PATH}");
	PathBuf::from(FALLBACK_CONFIG_PATH)
});

pub fn get_config_root() -> PathBuf {
	CONFIG_ROOT_PATH.clone()
}

pub fn get_skymaps_root() -> PathBuf {
	get_config_root().join("skymaps")
}

pub fn get_skymaps_uuids() -> anyhow::Result<Vec<String>> {
	let data = std::fs::read_to_string(get_skymaps_root().join("skymaps.txt"))?;
	Ok(data.lines().filter(|line| !line.is_empty()).map(String::from).collect())
}

pub fn set_skymaps_uuids(uuids: &[String]) -> anyhow::Result<()> {
	let skymaps_root = get_skymaps_root();
	let _ = std::fs::create_dir_all(&skymaps_root);
	let data = String::from_iter(uuids.iter().map(|uuid| format!("{}\n", uuid)));
	std::fs::write(skymaps_root.join("skymaps.txt"), data)?;
	Ok(())
}

impl ConfigRoot {
	pub fn get_conf_d_path(&self) -> PathBuf {
		get_config_root().join(match self {
			Self::Generic => "conf.d",
		})
	}

	// Make sure config directory is present and return root config path
	pub fn ensure_dir(&self) -> PathBuf {
		let path = get_config_root();
		let _ = std::fs::create_dir(&path);

		let path_conf_d = self.get_conf_d_path();
		let _ = std::fs::create_dir(path_conf_d);
		path
	}
}

pub fn get_config_file_path(filename: &str) -> PathBuf {
	get_config_root().join(filename)
}

pub fn load(filename: &str) -> Option<String> {
	let path = get_config_file_path(filename);
	log::info!("Loading config: {}", path.to_string_lossy());

	std::fs::read_to_string(path).ok()
}
