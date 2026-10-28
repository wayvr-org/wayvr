use std::{
	collections::{HashMap, HashSet},
	ffi::OsStr,
	io,
	path::{Path, PathBuf},
};

use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};

#[derive(Default)]
pub(crate) struct DesktopChanges {
	pub files: HashSet<PathBuf>,
	pub removed_directories: HashSet<PathBuf>,
	pub overflowed: bool,
}

pub(crate) struct DesktopWatcher {
	inotify: Inotify,
	roots: Vec<PathBuf>,
	by_wd: HashMap<WatchDescriptor, HashSet<PathBuf>>,
	by_path: HashMap<PathBuf, WatchDescriptor>,
}

impl DesktopWatcher {
	pub fn new(roots: &[PathBuf]) -> io::Result<Self> {
		let mut watcher = Self {
			inotify: Inotify::init()?,
			roots: roots.to_vec(),
			by_wd: HashMap::new(),
			by_path: HashMap::new(),
		};

		for root in roots {
			for ancestor in root.ancestors().skip(1) {
				if ancestor.is_dir() {
					watcher.watch_directory(ancestor);
				}
			}
			watcher.watch_tree(root, None);
		}

		Ok(watcher)
	}

	fn is_related_directory(&self, path: &Path) -> bool {
		self
			.roots
			.iter()
			.any(|root| path.starts_with(root) || root.starts_with(path))
	}

	fn is_application_file(&self, path: &Path) -> bool {
		path.extension() == Some(OsStr::new("desktop")) && self.roots.iter().any(|root| path.starts_with(root))
	}

	fn watch_directory(&mut self, path: &Path) {
		if self.by_path.contains_key(path) || !path.is_dir() {
			return;
		}

		let mask = WatchMask::CREATE
			| WatchMask::DELETE
			| WatchMask::MOVED_FROM
			| WatchMask::MOVED_TO
			| WatchMask::MODIFY
			| WatchMask::CLOSE_WRITE
			| WatchMask::ATTRIB
			| WatchMask::DELETE_SELF
			| WatchMask::MOVE_SELF
			| WatchMask::ONLYDIR;

		let wd = match self.inotify.watches().add(path, mask) {
			Ok(wd) => wd,
			Err(err) => {
				log::warn!("Unable to watch desktop directory {}: {err}", path.display());
				return;
			}
		};

		self.by_wd.entry(wd.clone()).or_default().insert(path.to_path_buf());
		self.by_path.insert(path.to_path_buf(), wd);
	}

	fn watch_tree(&mut self, directory: &Path, mut existing_files: Option<&mut HashSet<PathBuf>>) {
		let mut pending = vec![directory.to_path_buf()];

		while let Some(dir) = pending.pop() {
			if !self.is_related_directory(&dir) || !dir.is_dir() {
				continue;
			}

			self.watch_directory(&dir);

			let children = match std::fs::read_dir(&dir) {
				Ok(children) => children,
				Err(err) => {
					log::debug!("Cannot read desktop directory {}: {err}", dir.display());
					continue;
				}
			};

			for child in children.flatten() {
				let path = child.path();

				match child.file_type() {
					Ok(kind) if kind.is_dir() => {
						if self.is_related_directory(&path) {
							pending.push(path);
						}
					}
					Ok(_) => {
						if self.is_application_file(&path)
							&& let Some(files) = existing_files.as_deref_mut()
						{
							files.insert(path);
						}
					}
					Err(_) => {}
				}
			}
		}
	}

	fn unwatch_tree(&mut self, directory: &Path) {
		let descendants: Vec<_> = self
			.by_path
			.iter()
			.filter(|(path, _)| path.starts_with(directory))
			.map(|(path, wd)| (path.clone(), wd.clone()))
			.collect();

		for (path, wd) in descendants {
			self.by_path.remove(&path);

			if let Some(paths) = self.by_wd.get_mut(&wd) {
				paths.remove(&path);

				if paths.is_empty() {
					self.by_wd.remove(&wd);
					let _ = self.inotify.watches().remove(wd);
				}
			}
		}
	}

	pub fn collect_changes(&mut self) -> DesktopChanges {
		let mut changes = DesktopChanges::default();
		let mut buffer = [0u8; 64 * 1024];

		loop {
			let events = match self.inotify.read_events(&mut buffer) {
				Ok(events) => events,
				Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
				Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
				Err(err) => {
					log::warn!("Unable to read desktop directory inotify events: {err}");
					changes.overflowed = true;
					break;
				}
			};

			for event in events {
				if event.mask.contains(EventMask::Q_OVERFLOW) {
					changes.overflowed = true;
					continue;
				}

				if event.mask.contains(EventMask::IGNORED) {
					if let Some(paths) = self.by_wd.remove(&event.wd) {
						for path in paths {
							self.by_path.remove(&path);
						}
					}
					continue;
				}

				let Some(directories) = self.by_wd.get(&event.wd).cloned() else {
					continue;
				};

				for directory in directories {
					let path = match event.name {
						Some(name) => directory.join(name),
						None => directory,
					};

					if event.mask.intersects(EventMask::DELETE_SELF | EventMask::MOVE_SELF) {
						changes.removed_directories.insert(path.clone());
						self.unwatch_tree(&path);
					} else if event.mask.contains(EventMask::ISDIR) {
						if event.mask.intersects(EventMask::DELETE | EventMask::MOVED_FROM) {
							changes.removed_directories.insert(path.clone());
							self.unwatch_tree(&path);
						}

						if event.mask.intersects(EventMask::CREATE | EventMask::MOVED_TO) {
							self.watch_tree(&path, Some(&mut changes.files));
						}
					} else if self.is_application_file(&path) {
						changes.files.insert(path);
					}
				}
			}
		}

		changes
	}
}
