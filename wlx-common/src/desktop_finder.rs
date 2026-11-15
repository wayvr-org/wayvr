use std::{
	collections::{HashMap, HashSet},
	ffi::OsStr,
	fmt::Debug,
	fs::exists,
	path::{Path, PathBuf},
	rc::Rc,
	sync::Arc,
	thread::JoinHandle,
	time::Instant,
};

use crate::{cache_dir, desktop_watcher::DesktopWatcher, locale::Language};
use anyhow::Context;
use freedesktop_desktop_entry::DesktopEntry as FreedesktopDesktopEntry;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;
use wgui::i18n::Locale;

struct DesktopEntryOwned {
	app_id: Box<str>,
	exec_argv: Box<[Box<str>]>,
	app_name: Box<str>,
	icon_path: Option<Box<str>>,
	categories: Box<[Box<str>]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopEntry {
	pub exec_argv: Rc<[Rc<str>]>,
	pub app_id: Rc<str>,
	pub app_name: Rc<str>,
	pub icon_path: Option<Rc<str>>,
	pub categories: Rc<[Rc<str>]>,
}

impl From<DesktopEntryOwned> for DesktopEntry {
	fn from(value: DesktopEntryOwned) -> Self {
		Self {
			app_id: value.app_id.into(),
			exec_argv: value.exec_argv.into_iter().map(|x| x.into()).collect(),
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
	app_folders: Vec<PathBuf>,
	langs: Vec<String>,
}

struct CachedFileOwned {
	root_index: usize,
	entry: DesktopEntryOwned,
}

struct CachedFile {
	root_index: usize,
	entry: Rc<DesktopEntry>,
}

struct CacheBuild {
	files: HashMap<PathBuf, CachedFileOwned>,
	watcher: Option<DesktopWatcher>,
}

pub struct DesktopFinder {
	params: Arc<DesktopFinderParams>,
	// include all desktop files, so that removing a higher-prio one will reveal the lower-prio underneath
	files: HashMap<PathBuf, CachedFile>,
	entry_cache: HashMap<String, Rc<DesktopEntry>>,
	watcher: Option<DesktopWatcher>,
	bg_task: Option<JoinHandle<CacheBuild>>,
}

impl DesktopFinder {
	#[allow(clippy::new_without_default)]
	pub fn new(locale: &Locale) -> Self {
		let xdg = xdg::BaseDirectories::new();

		let mut app_folders = vec![];
		let mut icon_folders = vec![];

		if let Some(data_home) = xdg.get_data_home() {
			app_folders.push(data_home.join("applications"));
			app_folders.push(data_home.join("flatpak/exports/share/applications"));
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
			app_folders.push(data_dir.join("applications"));
			icon_folders.push(data_dir.join("icons").to_string_lossy().to_string());
		}

		let size_preferences: Vec<&'static OsStr> = ["scalable", "128x128", "96x96", "72x72", "64x64", "48x48", "32x32"]
			.into_iter()
			.map(OsStr::new)
			.collect();

		let langs = {
			let main_lang = locale.without_region().to_string();
			if main_lang == Language::get_default().code() {
				vec![main_lang]
			} else {
				vec![main_lang, Language::get_default().code().to_string()]
			}
		};

		Self {
			params: Arc::new(DesktopFinderParams {
				app_folders,
				icon_folders,
				size_preferences,
				langs,
			}),
			files: HashMap::new(),
			entry_cache: HashMap::new(),
			watcher: None,
			bg_task: None,
		}
	}

	fn read_single(params: &DesktopFinderParams, path: &Path) -> Option<DesktopEntryOwned> {
		let file_path = path.display();
		let file_name = path.file_name()?.to_string_lossy();
		let desktop = match FreedesktopDesktopEntry::from_path(path, Some(&params.langs)) {
			Ok(desktop) => desktop,
			Err(e) => {
				log::debug!("Failed to read .desktop file {}: {:?}, skipping", file_path, e);
				return None;
			}
		};

		if desktop.groups.desktop_entry().is_none() {
			log::debug!("Failed to get [Desktop Entry] section for file {}, skipping", file_path);
			return None;
		}
		if desktop.only_show_in().is_some() || desktop.terminal() || desktop.no_display() || desktop.hidden() {
			return None;
		}

		let exec = desktop.exec()?;
		if CMD_BLOCKLIST.iter().any(|blocked| exec.contains(blocked)) {
			return None;
		}
		let exec_argv = desktop
			.parse_exec_with_uris(&[], &params.langs)
			.into_iter()
			.flatten()
			.map(|x| x.into_boxed_str())
			.collect::<Box<[_]>>();
		if exec_argv.is_empty() {
			log::debug!("Invalid Exec in desktop file {}, skipping", file_path);
			return None;
		}
		let app_name = desktop.name(&params.langs)?;
		let mut vec_categories = Vec::<Box<str>>::new();
		for cat in desktop.categories().unwrap_or_default() {
			if CATEGORY_TYPE_BLOCKLIST.contains(&cat) {
				return None;
			}
			if !cat.is_empty() {
				vec_categories.push(cat.into());
			}
		}

		let icon_path = desktop
			.icon()
			.and_then(|icon_name| Self::find_icon(params, icon_name))
			.or_else(|| Self::create_icon(&file_name).ok());

		Some(DesktopEntryOwned {
			app_id: desktop.id().into(),
			app_name: app_name.into(),
			exec_argv,
			icon_path: icon_path.map(|i| i.into_boxed_str()),
			categories: vec_categories.into_boxed_slice(),
		})
	}

	fn build_cache_full(params: Arc<DesktopFinderParams>) -> CacheBuild {
		let start = Instant::now();
		// install watch before scan so edits during scan remain in inotify event queue
		let watcher = match DesktopWatcher::new(&params.app_folders) {
			Ok(watcher) => Some(watcher),
			Err(err) => {
				log::warn!("Could not watch .desktop files: {err}");
				None
			}
		};
		let mut files = HashMap::new();

		let icons_folder = cache_dir::get_path("icons");
		if !std::fs::exists(&icons_folder).unwrap_or(false) {
			let _ = std::fs::create_dir(&icons_folder);
		}

		for (root_index, root) in params.app_folders.iter().enumerate() {
			log::debug!("Searching desktop entries in path {}", root.display());
			for entry in WalkDir::new(root)
				.into_iter()
				.filter_map(Result::ok)
				.filter(|entry| !entry.file_type().is_dir())
			{
				let path = entry.path();
				if path.extension() != Some(OsStr::new("desktop")) || files.contains_key(path) {
					continue;
				}
				if let Some(parsed) = Self::read_single(&params, path) {
					files.insert(
						path.to_path_buf(),
						CachedFileOwned {
							root_index,
							entry: parsed,
						},
					);
				}
			}
		}

		log::debug!("Desktop entry & icon scan took {:?}", start.elapsed());
		CacheBuild { files, watcher }
	}

	pub fn resolve_icon(&self, icon_name: &str) -> Option<String> {
		Self::find_icon(&self.params, icon_name)
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

		let Ok(CacheBuild { files, watcher }) = bg_task.join() else {
			log::warn!("Desktop entry scan thread panicked");
			return;
		};

		self.files = files
			.into_iter()
			.map(|(path, file)| {
				(
					path,
					CachedFile {
						root_index: file.root_index,
						entry: Rc::new(file.entry.into()),
					},
				)
			})
			.collect();
		self.watcher = watcher;

		let mut files: Vec<_> = self.files.iter().collect();
		files.sort_by(|(path_a, a), (path_b, b)| a.root_index.cmp(&b.root_index).then_with(|| path_a.cmp(path_b)));
		self.entry_cache.clear();
		for (_, file) in files {
			self
				.entry_cache
				.entry(file.entry.app_id.to_string())
				.or_insert_with(|| Rc::clone(&file.entry));
		}
	}

	fn update_app_id(&mut self, app_id: &str) {
		let best = self
			.files
			.iter()
			.filter(|(_, file)| file.entry.app_id.as_ref() == app_id)
			.min_by(|(path_a, a), (path_b, b)| a.root_index.cmp(&b.root_index).then_with(|| path_a.cmp(path_b)))
			.map(|(_, file)| Rc::clone(&file.entry));
		if let Some(best) = best {
			self.entry_cache.insert(app_id.to_owned(), best);
		} else {
			self.entry_cache.remove(app_id);
		}
	}

	fn sync_file_changes(&mut self) {
		self.wait_for_entries();
		let Some(watcher) = self.watcher.as_mut() else {
			return;
		};
		let changes = watcher.collect_changes();
		if changes.overflowed {
			log::warn!("Desktop file inotify event queue overflowed; rebuilding cache");
			self.refresh();
			self.wait_for_entries();
			return;
		}

		let mut affected_ids = HashSet::new();
		if !changes.removed_directories.is_empty() {
			self.files.retain(|path, file| {
				let removed = changes
					.removed_directories
					.iter()
					.any(|directory| path.starts_with(directory));
				if removed {
					affected_ids.insert(file.entry.app_id.to_string());
				}
				!removed
			});
		}
		for path in changes.files {
			if let Some(previous) = self.files.remove(&path) {
				affected_ids.insert(previous.entry.app_id.to_string());
			}
			let root_index = self.params.app_folders.iter().position(|root| path.starts_with(root));
			if let Some(root_index) = root_index
				&& let Some(entry) = Self::read_single(&self.params, &path)
			{
				affected_ids.insert(entry.app_id.to_string());
				self.files.insert(
					path,
					CachedFile {
						root_index,
						entry: Rc::new(entry.into()),
					},
				);
			}
		}
		for app_id in affected_ids {
			self.update_app_id(&app_id);
		}
	}

	pub fn get_cached_entry(&mut self, app_id: &str) -> Option<&Rc<DesktopEntry>> {
		self.sync_file_changes();
		self.entry_cache.get(app_id)
	}

	pub fn find_entries(&mut self) -> HashMap<String, Rc<DesktopEntry>> {
		self.sync_file_changes();
		self.entry_cache.clone()
	}

	pub fn refresh(&mut self) {
		let bg_task = std::thread::spawn({
			let params = self.params.clone();
			move || Self::build_cache_full(params)
		});
		self.bg_task = Some(bg_task);
	}
}
