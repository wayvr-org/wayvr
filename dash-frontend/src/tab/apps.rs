use std::{
	cell::RefCell,
	collections::{HashMap, VecDeque},
	marker::PhantomData,
	rc::Rc,
};

use wgui::{
	assets::AssetPath,
	components::button::{ButtonClickCallback, ComponentButton},
	globals::WguiGlobals,
	layout::{WidgetID, WidgetPair},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	task::Tasks,
};
use wlx_common::desktop_finder::DesktopEntry;

use crate::{
	frontend::{Frontend, FrontendTasks},
	tab::{Tab, TabType},
	util::popup_manager::PopupHolder,
	views::{self},
};

#[derive(Clone)]
enum Task {
	CloseLauncher,
}

struct State {
	view_launcher: Option<PopupHolder<views::app_launcher::View>>,
}

pub struct TabApps<T> {
	#[allow(dead_code)]
	parser_state: ParserState,

	state: Rc<RefCell<State>>,
	app_list: AppList,
	tasks: Tasks<Task>,
	marker: PhantomData<T>,
}

impl<T> Tab<T> for TabApps<T> {
	fn get_type(&self) -> TabType {
		TabType::Apps
	}

	fn update(&mut self, frontend: &mut Frontend<T>, _time_ms: u32, data: &mut T) -> anyhow::Result<()> {
		let mut state = self.state.borrow_mut();

		for task in self.tasks.drain() {
			match task {
				Task::CloseLauncher => state.view_launcher = None,
			}
		}

		self
			.app_list
			.tick(frontend, &self.state, &self.tasks, &mut self.parser_state)?;

		if let Some((_, launcher)) = &mut state.view_launcher {
			launcher.update(&mut frontend.interface, data)?;
		}
		Ok(())
	}
}

struct AppList {
	//data: Vec<ParserData>,
	entries_to_mount: VecDeque<DesktopEntry>,
	list_parent: WidgetPair,
	prev_category_name: String,
}

// called after the user clicks any desktop entry
fn on_app_click(
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	entry: DesktopEntry,
	state: Rc<RefCell<State>>,
	tasks: Tasks<Task>,
) -> ButtonClickCallback {
	Rc::new(move |_common, _evt| {
		views::app_launcher::mount_popup(
			frontend_tasks.clone(),
			globals.clone(),
			entry.clone(),
			tasks.make_callback_box(Task::CloseLauncher),
			Box::new({
				let state = state.clone();
				move |popup| state.borrow_mut().view_launcher = Some(popup)
			}),
		);
		Ok(())
	})
}

fn doc_params(globals: WguiGlobals) -> ParseDocumentParams<'static> {
	ParseDocumentParams {
		globals,
		path: AssetPath::BuiltIn("gui/tab/apps.xml"),
		extra: Default::default(),
	}
}

impl<T> TabApps<T> {
	pub fn new(frontend: &mut Frontend<T>, parent_id: WidgetID, data: &mut T) -> anyhow::Result<Self> {
		let globals = frontend.layout.state.globals.clone();
		let tasks = Tasks::new();
		let state = Rc::new(RefCell::new(State { view_launcher: None }));

		let parser_state = wgui::parser::parse_from_assets(&doc_params(globals.clone()), &mut frontend.layout, parent_id)?;
		let app_list_parent = parser_state.fetch_widget(&frontend.layout.state, "app_list_parent")?;

		let mut entries_sorted: Vec<_> = frontend
			.interface
			.desktop_finder(data)
			.find_entries()
			.into_values()
			.collect();

		entries_sorted.sort_by(|a, b| {
			let cat_name_a = get_category_name(a);
			let cat_name_b = get_category_name(b);
			cat_name_a.cmp(cat_name_b)
		});

		let app_list = AppList {
			entries_to_mount: entries_sorted.drain(..).collect(),
			list_parent: app_list_parent,
			prev_category_name: String::new(),
		};

		Ok(Self {
			app_list,
			parser_state,
			state,
			tasks,
			marker: PhantomData,
		})
	}
}

enum Scores {
	Empty,
	Unknown,
	XFooBar, // X-something
	Xfce,
	Gnome,
	Kde,
	Gtk,
	Qt,
	Settings,
	Application,
	System,
	Utility,
	FileTools,
	Filesystem,
	FileManager,
	Graphics,
	Office,
	Game,
	VR, // best score (of course!)
}

fn get_category_name_score(name: &str) -> u8 {
	if name.starts_with("X-") {
		return Scores::XFooBar as u8;
	}

	match name {
		"" => {
			return Scores::Empty as u8;
		}
		"VR" => {
			return Scores::VR as u8;
		}
		"Game" => {
			return Scores::Game as u8;
		}
		"FileManager" => {
			return Scores::FileManager as u8;
		}
		"Utility" => {
			return Scores::Utility as u8;
		}
		"FileTools" => {
			return Scores::FileTools as u8;
		}
		"Filesystem" => {
			return Scores::Filesystem as u8;
		}
		"System" => {
			return Scores::System as u8;
		}
		"Office" => {
			return Scores::Office as u8;
		}
		"Settings" => {
			return Scores::Settings as u8;
		}
		"Application" => {
			return Scores::Application as u8;
		}
		"GTK" => {
			return Scores::Gtk as u8;
		}
		"Qt" => {
			return Scores::Qt as u8;
		}
		"XFCE" => {
			return Scores::Xfce as u8;
		}
		"GNOME" => {
			return Scores::Gnome as u8;
		}
		"KDE" => {
			return Scores::Kde as u8;
		}
		"Graphics" => {
			return Scores::Graphics as u8;
		}
		_ => {}
	}

	Scores::Unknown as u8
}

fn get_best_category_name(categories: &[Rc<str>]) -> Option<&Rc<str>> {
	let mut best_score: u8 = 0;
	let mut best_category: Option<&Rc<str>> = None;

	for cat in categories {
		let score = get_category_name_score(cat);
		if score > best_score {
			best_category = Some(cat);
			best_score = score;
		}
	}

	best_category
}

fn get_category_name(entry: &DesktopEntry) -> &str {
	//log::info!("{:?}", entry.categories);

	match get_best_category_name(&entry.categories) {
		Some(cat) => cat,
		None => "Other",
	}
}

impl AppList {
	fn mount_entry<T>(
		&mut self,
		frontend: &mut Frontend<T>,
		parser_state: &mut ParserState,
		doc_params: &ParseDocumentParams,
		entry: &DesktopEntry,
	) -> anyhow::Result<Rc<ComponentButton>> {
		let category_name = get_category_name(entry);
		if category_name != self.prev_category_name {
			self.prev_category_name = String::from(category_name);
			let mut params = HashMap::<Rc<str>, Rc<str>>::new();
			params.insert("text".into(), category_name.into());

			parser_state.realize_template(
				doc_params,
				"CategoryText",
				&mut frontend.layout,
				self.list_parent.id,
				params,
			)?;
		}

		{
			let mut params = HashMap::new();

			// entry icon
			params.insert(
				"src_ext".into(),
				entry
					.icon_path
					.as_ref()
					.map_or_else(|| "".into(), |icon_path| icon_path.clone()),
			);

			// entry fallback (question mark) icon
			params.insert(
				"src".into(),
				if entry.icon_path.is_none() {
					"dashboard/terminal.svg".into()
				} else {
					"".into()
				},
			);
			params.insert("name".into(), entry.app_name.clone());

			let data = parser_state.realize_template(
				doc_params,
				"AppEntry",
				&mut frontend.layout,
				self.list_parent.id,
				params,
			)?;

			data.fetch_component_as::<ComponentButton>("button")
		}
	}

	fn tick<T>(
		&mut self,
		frontend: &mut Frontend<T>,
		state: &Rc<RefCell<State>>,
		tasks: &Tasks<Task>,
		parser_state: &mut ParserState,
	) -> anyhow::Result<()> {
		// load 4 entries for a single frame at most
		for _ in 0..4 {
			if let Some(entry) = self.entries_to_mount.pop_front() {
				let globals = frontend.layout.state.globals.clone();
				let button = self.mount_entry(frontend, parser_state, &doc_params(globals.clone()), &entry)?;

				button.on_click(on_app_click(
					frontend.tasks.clone(),
					globals.clone(),
					entry.clone(),
					state.clone(),
					tasks.clone(),
				));
			} else {
				break;
			}
		}

		Ok(())
	}
}
