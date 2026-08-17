use crate::i18n::LangsList;
use flate2::read::GzDecoder;
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetPathSource {
	Internal,
	BuiltIn,
	Filesystem,
}

#[derive(Debug, Clone, Copy)]
pub enum AssetPathRef<'a> {
	WguiInternal(&'a str),  // tied to internal wgui AssetProvider. Used internally
	BuiltIn(&'a str),       // tied to user AssetProvider
	FileOrBuiltIn(&'a str), // attempts to load from a path relative to asset_folder, falls back to BuiltIn
	File(&'a str),          // load from filesystem
}

// see AssetPath above for documentation
#[derive(Debug, Clone)]
pub enum AssetPathRc {
	WguiInternal(Rc<Path>),
	BuiltIn(Rc<Path>),
	FileOrBuiltIn(Rc<Path>),
	File(Rc<Path>),
}

impl AssetPathRef<'_> {
	pub const fn get_str(&self) -> &str {
		match &self {
			AssetPathRef::WguiInternal(path) => path,
			AssetPathRef::BuiltIn(path) => path,
			AssetPathRef::FileOrBuiltIn(path) => path,
			AssetPathRef::File(path) => path,
		}
	}

	pub fn to_rc(&self) -> AssetPathRc {
		match self {
			AssetPathRef::WguiInternal(path) => AssetPathRc::WguiInternal(Rc::from(Path::new(path))),
			AssetPathRef::BuiltIn(path) => AssetPathRc::BuiltIn(Rc::from(Path::new(path))),
			AssetPathRef::FileOrBuiltIn(path) => AssetPathRc::FileOrBuiltIn(Rc::from(Path::new(path))),
			AssetPathRef::File(path) => AssetPathRc::File(Rc::from(Path::new(path))),
		}
	}
}

impl AssetPathRc {
	pub fn as_ref(&'_ self) -> AssetPathRef<'_> {
		match self {
			AssetPathRc::WguiInternal(buf) => AssetPathRef::WguiInternal(buf.to_str().unwrap()),
			AssetPathRc::BuiltIn(buf) => AssetPathRef::BuiltIn(buf.to_str().unwrap()),
			AssetPathRc::FileOrBuiltIn(buf) => AssetPathRef::FileOrBuiltIn(buf.to_str().unwrap()),
			AssetPathRc::File(buf) => AssetPathRef::File(buf.to_str().unwrap()),
		}
	}

	#[must_use]
	pub const fn replace_path(&self, new_path: Rc<Path>) -> AssetPathRc {
		match self {
			AssetPathRc::WguiInternal(_) => AssetPathRc::WguiInternal(new_path),
			AssetPathRc::BuiltIn(_) => AssetPathRc::BuiltIn(new_path),
			AssetPathRc::FileOrBuiltIn(_) => AssetPathRc::FileOrBuiltIn(new_path),
			AssetPathRc::File(_) => AssetPathRc::File(new_path),
		}
	}

	pub fn get_path(&self) -> &Path {
		match self {
			AssetPathRc::WguiInternal(buf) => buf.as_ref(),
			AssetPathRc::BuiltIn(buf) => buf.as_ref(),
			AssetPathRc::FileOrBuiltIn(buf) => buf.as_ref(),
			AssetPathRc::File(buf) => buf.as_ref(),
		}
	}

	#[must_use]
	pub fn strip_filename(&self) -> AssetPathRc {
		let res = strip_filename_from_path(self.get_path());
		match self {
			AssetPathRc::WguiInternal(_) => AssetPathRc::WguiInternal(res.into()),
			AssetPathRc::BuiltIn(_) => AssetPathRc::BuiltIn(res.into()),
			AssetPathRc::FileOrBuiltIn(_) => AssetPathRc::FileOrBuiltIn(res.into()),
			AssetPathRc::File(_) => AssetPathRc::File(res.into()),
		}
	}
}

fn strip_filename_from_path(path: &Path) -> PathBuf {
	path.parent().unwrap_or_else(|| Path::new("/")).to_path_buf()
}

pub trait LangProvider {
	fn langs_list(&self) -> &dyn LangsList;
	fn forced_lang(&self) -> Option<&str>;
}

pub trait AssetProvider {
	fn load_from_path(&mut self, path: &str) -> anyhow::Result<Vec<u8>>;
	fn load_from_path_gzip(&mut self, path: &str) -> anyhow::Result<Vec<u8>> {
		let compressed = self.load_from_path(path)?;
		let mut gz = GzDecoder::new(&compressed[..]);
		let mut out = Vec::new();
		gz.read_to_end(&mut out)?;
		Ok(out)
	}
}

// replace "./foo/bar/../file.txt" with "foo/file.txt"
pub fn normalize_path(path: &Path, remove_root_slash: bool) -> PathBuf {
	let mut stack = Vec::new();

	for component in path.components() {
		match component {
			Component::ParentDir => {
				match stack.last() {
					// ../foo, ../../foo, ./../foo → push ".."
					None | Some(Component::ParentDir | Component::CurDir) => stack.push(Component::ParentDir),
					// "foo/../bar" → pop "foo" and don't push ".."
					Some(Component::Normal(_)) => {
						stack.pop();
					}
					// other weird cases, e.g. "/../foo" → "/foo"
					_ => {}
				}
			}
			// ./foo → foo
			Component::CurDir => {}

			// keep as-is
			Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
				stack.push(component);
			}
		}
	}

	stack
		.into_iter()
		.map(|comp| match comp {
			Component::RootDir => {
				if remove_root_slash {
					OsStr::new("")
				} else {
					OsStr::new("/")
				}
			}
			Component::Prefix(p) => p.as_os_str(), // should not occur on Unix
			Component::ParentDir => OsStr::new(".."),
			Component::Normal(s) => s,
			Component::CurDir => unreachable!(), // stripped in all cases
		})
		.collect()
}
