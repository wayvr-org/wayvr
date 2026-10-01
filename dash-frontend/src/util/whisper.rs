use std::{fs, io, path::PathBuf};

use crate::util::downloadable_file::DownloadableFile;
use wlx_common::data_dir;

pub const WHISPER_MODELS: &[DownloadableFile] = &[
	DownloadableFile {
		file_name: "ggml-base-q8_0.bin",
		display_name: "Base Q8 (78MiB)",
		url: "https://wayvr.org/files/whisper/ggml-base-q8_0.bin",
	},
	DownloadableFile {
		file_name: "ggml-small-q8_0.bin",
		display_name: "Small Q8 (252MiB)",
		url: "https://wayvr.org/files/whisper/ggml-small-q8_0.bin",
	},
	DownloadableFile {
		file_name: "ggml-large-v3-turbo-q5_0.bin",
		display_name: "Turbo Q5 (574MiB)",
		url: "https://wayvr.org/files/whisper/ggml-large-v3-turbo-q5_0.bin",
	},
	DownloadableFile {
		file_name: "ggml-large-v3-turbo-q8_0.bin",
		display_name: "Turbo Q8 (874MiB)",
		url: "https://wayvr.org/files/whisper/ggml-large-v3-turbo-q8_0.bin",
	},
	DownloadableFile {
		file_name: "ggml-large-v3-turbo.bin",
		display_name: "Turbo (1.5GiB)",
		url: "https://wayvr.org/files/whisper/ggml-large-v3-turbo.bin",
	},
];

pub fn whisper_model_from_name(file_name: &str) -> Option<&'static DownloadableFile> {
	WHISPER_MODELS.iter().find(|x| x.file_name == file_name)
}

pub fn whisper_model_folder() -> PathBuf {
	data_dir::get_path("whisper")
}

pub fn whisper_model_path(file_name: &str) -> PathBuf {
	whisper_model_folder().join(file_name)
}

pub fn whisper_any_models_downloaded() -> io::Result<bool> {
	let path = whisper_model_folder();
	if !path.is_dir() {
		return Ok(false);
	}
	Ok(fs::read_dir(path)?.count() > 0)
}

pub fn whisper_delete_all_models() -> io::Result<()> {
	let path = whisper_model_folder();
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
