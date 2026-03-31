use std::{
	collections::{HashMap, HashSet},
	ffi::OsStr,
	fmt::Debug,
	fs::exists,
	path::Path,
	rc::Rc,
	sync::Arc,
	thread::JoinHandle,
	time::Instant,
};

use crate::cache_dir;
use anyhow::Context;
use ini::Ini;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

struct DesktopEntryOwned {
	exec_path: String,
	exec_args: String,
	app_name: String,
	icon_path: Option<String>,
	categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopEntry {
	pub exec_path: Rc<str>,
	pub exec_args: Rc<str>,
	pub app_name: Rc<str>,
	pub icon_path: Option<Rc<str>>,
	pub categories: Vec<Rc<str>>,
}

impl From<DesktopEntryOwned> for DesktopEntry {
	fn from(value: DesktopEntryOwned) -> Self {
		Self {
			exec_path: value.exec_path.into(),
			exec_args: value.exec_args.into(),
			app_name: value.app_name.into(),
			icon_path: value.icon_path.map(|x| x.into()),
			categories: value.categories.into_iter().map(|x| x.into()).collect(),
		}
	}
}

const CMD_BLOCKLIST: [&str; 3] = [
	"lsp-plugins", // LSP Plugins collection. They clutter the application list a lot
	"vrmonitor",
	"vrurlhandler",
];

const CATEGORY_TYPE_BLOCKLIST: [&str; 1] = ["ConsoleOnly"];

struct DesktopFinderParams {
	size_preferences: Vec<&'static OsStr>,
	icon_folders: Vec<String>,
	app_folders: Vec<String>,
}

pub struct DesktopFinder {
	params: Arc<DesktopFinderParams>,
	entry_cache: HashMap<String, DesktopEntry>,
	bg_task: Option<JoinHandle<HashMap<String, DesktopEntryOwned>>>,
}

impl DesktopFinder {
	#[allow(clippy::new_without_default)]
	pub fn new() -> Self {
		let xdg = xdg::BaseDirectories::new();

		let mut app_folders = vec![];
		let mut icon_folders = vec![];

		if let Some(data_home) = xdg.get_data_home() {
			app_folders.push(data_home.join("applications").to_string_lossy().to_string());
			app_folders.push(
				data_home
					.join("flatpak/exports/share/applications")
					.to_string_lossy()
					.to_string(),
			);
			icon_folders.push(data_home.join("icons").to_string_lossy().to_string());
			icon_folders.push(
				data_home
					.join("flatpak/exports/share/icons")
					.to_string_lossy()
					.to_string(),
			);
		}

		app_folders.push("/var/lib/flatpak/exports/share/applications".into());
		icon_folders.push("/var/lib/flatpak/exports/share/icons".into());

		// /usr/share and such
		for data_dir in xdg.get_data_dirs() {
			app_folders.push(data_dir.join("applications").to_string_lossy().to_string());
			icon_folders.push(data_dir.join("icons").to_string_lossy().to_string());
		}

		let size_preferences: Vec<&'static OsStr> = ["scalable", "128x128", "96x96", "72x72", "64x64", "48x48", "32x32"]
			.into_iter()
			.map(OsStr::new)
			.collect();

		Self {
			params: Arc::new(DesktopFinderParams {
				app_folders,
				icon_folders,
				size_preferences,
			}),
			entry_cache: HashMap::new(),
			bg_task: None,
		}
	}

	fn build_cache(params: Arc<DesktopFinderParams>) -> HashMap<String, DesktopEntryOwned> {
		let start = Instant::now();

		let mut known_files = HashSet::new();
		let mut entries = HashMap::<String, DesktopEntryOwned>::new();

		let icons_folder = cache_dir::get_path("icons");
		if !std::fs::exists(&icons_folder).unwrap_or(false) {
			let _ = std::fs::create_dir(&icons_folder);
		}

		for path in &params.app_folders {
			log::debug!("Searching desktop entries in path {}", path);

			'entries: for entry in WalkDir::new(path)
				.into_iter()
				.filter_map(|e| e.ok())
				.filter(|e| !e.file_type().is_dir())
			{
				let Some(extension) = Path::new(entry.file_name()).extension() else {
					continue;
				};

				if extension != "desktop" {
					continue; // ignore, go on
				}

				let file_name = entry.file_name().to_string_lossy();
				let Some(app_id) = Path::new(entry.file_name())
					.file_stem()
					.map(|x| x.to_string_lossy().to_string())
				else {
					continue;
				};

				if known_files.contains(file_name.as_ref()) {
					// as per xdg spec, user entries of the same filename will override system entries
					continue;
				}

				let file_path = format!("{}/{}", path, file_name);

				let ini = match Ini::load_from_file(&file_path) {
					Ok(ini) => ini,
					Err(e) => {
						log::debug!("Failed to read INI for .desktop file {}: {:?}, skipping", file_path, e);
						continue;
					}
				};

				let Some(section) = ini.section(Some("Desktop Entry")) else {
					log::debug!("Failed to get [Desktop Entry] section for file {}, skipping", file_path);
					continue;
				};

				if section.contains_key("OnlyShowIn") {
					continue; // probably XFCE, KDE, GNOME or other DE-specific stuff
				}

				if let Some(x) = section.get("Terminal")
					&& x == "true"
				{
					continue;
				}

				if let Some(x) = section.get("NoDisplay")
					&& x.eq_ignore_ascii_case("true")
				{
					continue; // This application is hidden
				}

				if let Some(x) = section.get("Hidden")
					&& x.eq_ignore_ascii_case("true")
				{
					continue; // This application is hidden
				}

				let Some(exec) = section.get("Exec") else {
					log::debug!("Failed to get desktop entry Exec for file {}, skipping", file_path);
					continue;
				};

				for entry in &CMD_BLOCKLIST {
					if exec.contains(entry) {
						continue 'entries;
					}
				}

				let (exec_path, exec_args) = match exec.split_once(" ") {
					Some((left, right)) => (
						left,
						right
							.split(" ")
							.filter(|arg| !arg.starts_with('%')) // exclude arguments like "%f"
							.map(String::from)
							.collect(),
					),
					None => (exec, Vec::new()),
				};

				let Some(app_name) = section.get("Name") else {
					log::debug!(
						"Failed to get desktop entry application name for file {}, skipping",
						file_path
					);
					continue;
				};

				let icon_path = section
					.get("Icon")
					.and_then(|icon_name| Self::find_icon(&params, icon_name))
					.or_else(|| Self::create_icon(&file_name).ok());

				let mut vec_categories = Vec::<String>::new();

				if let Some(categories) = section.get("Categories") {
					for cat in categories.split(";") {
						if CATEGORY_TYPE_BLOCKLIST.contains(&cat) {
							continue 'entries;
						}
						vec_categories.push(String::from(cat));
					}
				}

				known_files.insert(file_name.to_string());

				entries.insert(
					app_id,
					DesktopEntryOwned {
						app_name: String::from(app_name),
						exec_path: String::from(exec_path),
						exec_args: exec_args.join(" "),
						icon_path,
						categories: vec_categories,
					},
				);
			}
		}

		log::debug!("Desktop entry & icon scan took {:?}", start.elapsed());

		entries
	}

	fn find_icon(params: &DesktopFinderParams, icon_name: &str) -> Option<String> {
		if icon_name.starts_with("/") && exists(icon_name).unwrap_or(false) {
			return Some(icon_name.to_string());
		}

		for folder in &params.icon_folders {
			let pattern = format!("{}/hicolor/*/apps/{}.*", folder, icon_name);

			let mut entries: Vec<_> = glob::glob(&pattern)
				.expect("Bad glob pattern!")
				.filter_map(Result::ok)
				.collect();

			// sort by SIZE_PREFERENCES
			entries.sort_by_key(|path| {
				path
					.components()
					.rev()
					.nth(2) // ← hicolor/<*SIZE*>/apps/filename.ext
					.map(|c| c.as_os_str())
					.and_then(|size| params.size_preferences.iter().position(|&p| p == size))
					.unwrap_or(usize::MAX)
			});

			if let Some(first) = entries.into_iter().next() {
				return Some(first.to_string_lossy().into());
			}
		}
		None
	}

	pub fn create_icon(desktop_entry_name: &str) -> anyhow::Result<String> {
		let relative_path = format!("icons/{}.svg", desktop_entry_name);
		let file_path = cache_dir::get_path(&relative_path);

		let mut dir_path = file_path.clone();
		dir_path.pop();
		std::fs::create_dir_all(dir_path).context("Could not create ~/.cache/wayvr")?;

		let file_path = file_path.to_string_lossy().to_string();

		if std::fs::exists(&file_path).unwrap_or(false) {
			return Ok(file_path);
		}

		let svg = identicons_svg::generate(identicons_svg::IdenticonOptions {
			background: identicons_svg::Background {
				r: 64,
				color: "rgba(0.9,0.9,0.9,0.5)".into(),
			},
			..Default::default()
		});

		std::fs::write(&file_path, svg).context("Could not save file to ~/.cache/wayvr")?;
		Ok(file_path)
	}

	fn wait_for_entries(&mut self) {
		let Some(bg_task) = self.bg_task.take() else {
			return;
		};

		let Ok(entries) = bg_task.join() else {
			return;
		};

		self.entry_cache.clear();
		for (app_id, entry) in entries {
			self.entry_cache.insert(app_id, entry.into());
		}
	}

	pub fn get_cached_entry(&self, app_id: &str) -> Option<&DesktopEntry> {
		self.entry_cache.get(app_id)
	}

	pub fn find_entries(&mut self) -> HashMap<String, DesktopEntry> {
		self.wait_for_entries();
		self.entry_cache.clone()
	}

	pub fn refresh(&mut self) {
		let bg_task = std::thread::spawn({
			let params = self.params.clone();
			move || Self::build_cache(params)
		});
		self.bg_task = Some(bg_task);
	}
}
