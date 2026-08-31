use std::{fs, io, path::PathBuf};

use crate::util::downloadable_file::DownloadableFile;
use wlx_common::data_dir;

pub const SWIPE_TYPE_MODEL: DownloadableFile = DownloadableFile {
	file_name: "en.tar",
	display_name: "English Qwerty (15 MiB)",
	url: "https://github.com/oneshinyboi/super-swipe-type/raw/refs/tags/v0.4.2/crates/super-swipe-type/assets/en.tar",
};

pub fn swipe_type_model_folder() -> PathBuf {
	data_dir::get_path("swipe_type")
}

pub fn swipe_type_model_path(file_name: &str) -> PathBuf {
	swipe_type_model_folder().join(file_name)
}

pub fn swwipe_type_model_downloaded() -> io::Result<bool> {
	let path = swipe_type_model_folder();
	if !path.is_dir() {
		return Ok(false);
	}
	if !path.join(SWIPE_TYPE_MODEL.file_name).exists() {
		return Ok(false);
	}
	Ok(true)
}

pub fn swipe_type_delete_all_models() -> io::Result<()> {
	let path = swipe_type_model_folder();
	if !path.is_dir() {
		return Ok(());
	}

	for entry in fs::read_dir(path)? {
		let entry = entry?;
		let file_type = entry.file_type()?;

		if file_type.is_file() || file_type.is_symlink() {
			fs::remove_file(entry.path())?;
		}
	}

	Ok(())
}
