use std::{cell::RefCell, collections::VecDeque, marker::PhantomData, rc::Rc};
use wgui::{
	assets::AssetPath,
	components::button::{ButtonClickCallback, ComponentButton},
	globals::WguiGlobals,
	i18n::Translation,
	layout::{WidgetID, WidgetPair},
	parser::{Fetchable, ParseDocumentParams, ParserState, TemplateParams},
	task::Tasks,
};
use wlx_common::{config::PinnedApp, desktop_finder::DesktopEntry};

use crate::{
	frontend::{Frontend, FrontendTasks},
	tab::{Tab, TabType},
	util::{popup_manager::PopupHolder, wgui_simple},
	views::{self},
};

struct State {
	#[allow(dead_code)]
	parser_state: ParserState,
	view_launcher: PopupHolder<views::app_launcher::View>,
}

#[derive(Clone)]
enum Task {
	RefreshPinnedApps,
}

pub struct TabApps<T> {
	state: Rc<RefCell<State>>,
	app_list: AppList,
	marker: PhantomData<T>,

	pinned_apps_parent: WidgetID,

	tasks: Tasks<Task>,
}

impl<T> Tab<T> for TabApps<T> {
	fn get_type(&self) -> TabType {
		TabType::Apps
	}

	fn update(&mut self, frontend: &mut Frontend<T>, _time_ms: u32, data: &mut T) -> anyhow::Result<()> {
		let mut state = self.state.borrow_mut();

		self.app_list.tick(frontend, &self.tasks, &mut state, &self.state)?;

		for task in self.tasks.drain() {
			match task {
				Task::RefreshPinnedApps => self.refresh_pinned_apps(&mut state, frontend, data)?,
			}
		}

		state
			.view_launcher
			.with_view_res(|view| view.update(&mut frontend.interface, data))?;
		Ok(())
	}
}

fn find_entry_from_app_name<'a>(app_id: &str, entries: &'a [DesktopEntry]) -> Option<&'a DesktopEntry> {
	entries.iter().find(|&entry| *entry.app_id == *app_id)
}

impl<T> TabApps<T> {
	fn refresh_pinned_apps(&self, state: &mut State, frontend: &mut Frontend<T>, data: &mut T) -> anyhow::Result<()> {
		frontend.layout.remove_children(self.pinned_apps_parent);
		let globals = frontend.globals.clone();

		let mut stale_entries = Vec::<Rc<str>>::new();

		let mut pinned_desktop_entries = Vec::<(PinnedApp, &DesktopEntry)>::new();

		// collect pinned desktop entries
		{
			let config = frontend.interface.general_config(data);
			for pinned_app in &config.pinned_apps {
				let Some(desktop_entry) = find_entry_from_app_name(&pinned_app.app_id, &self.app_list.all_entries) else {
					stale_entries.push(pinned_app.app_id.clone());
					continue;
				};

				pinned_desktop_entries.push((pinned_app.clone(), desktop_entry));
			}
			// cleanup:
			// remove non-existent app ids from pinned apps
			config.pinned_apps.retain(|pinned_app| {
				for app_id in &stale_entries {
					if *app_id == pinned_app.app_id {
						return false;
					}
				}
				true
			});
		}

		if pinned_desktop_entries.is_empty() {
			wgui_simple::create_label(
				&mut frontend.layout,
				self.pinned_apps_parent,
				Translation::from_translation_key("EMPTY"),
			)?;
		}

		// mount pinned desktop entries
		for (pinned_app, desktop_entry) in pinned_desktop_entries {
			let tooltip_string = format!(
				"{}\n{}\n{}",
				pinned_app.compositor_mode.as_ref(),
				pinned_app.orientation_mode.as_ref(),
				pinned_app.res_mode.as_ref()
			);

			let button = mount_entry(
				frontend,
				&mut state.parser_state,
				&doc_params(frontend.globals.clone()),
				self.pinned_apps_parent,
				desktop_entry,
				Some(tooltip_string),
			)?;

			button.on_click(on_app_click(
				frontend.tasks.clone(),
				self.tasks.clone(),
				globals.clone(),
				desktop_entry.clone(),
				self.state.clone(),
				Some(pinned_app.clone()),
			));
		}

		Ok(())
	}
}

struct AppList {
	//data: Vec<ParserData>,
	all_entries: Vec<DesktopEntry>,
	entries_to_mount: VecDeque<DesktopEntry>,
	list_parent: WidgetPair,
	prev_category_name: String,
}

// called after the user clicks any desktop entry
fn on_app_click(
	frontend_tasks: FrontendTasks,
	tasks: Tasks<Task>,
	globals: WguiGlobals,
	entry: DesktopEntry,
	state: Rc<RefCell<State>>,
	pinned_app: Option<PinnedApp>,
) -> ButtonClickCallback {
	Rc::new(move |_common, _evt| {
		views::app_launcher::mount_popup(
			frontend_tasks.clone(),
			globals.clone(),
			entry.clone(),
			state.borrow_mut().view_launcher.clone(),
			tasks.make_callback_box(Task::RefreshPinnedApps),
			pinned_app.clone(),
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
		let parser_state = wgui::parser::parse_from_assets(&doc_params(globals.clone()), &mut frontend.layout, parent_id)?;

		let app_list_parent = parser_state.fetch_widget(&frontend.layout.state, "app_list_parent")?;
		let pinned_apps_parent = parser_state.fetch_widget(&frontend.layout.state, "pinned_apps_parent")?;

		let state = Rc::new(RefCell::new(State {
			view_launcher: Default::default(),
			parser_state,
		}));

		let entries: Vec<_> = frontend
			.interface
			.desktop_finder(data)
			.find_entries()
			.into_values()
			.collect();

		let mut entries_sorted = entries.clone();
		entries_sorted.sort_by(|a, b| {
			let cat_name_a = get_category_name(a);
			let cat_name_b = get_category_name(b);
			cat_name_a.cmp(cat_name_b)
		});

		let app_list = AppList {
			all_entries: entries,
			entries_to_mount: entries_sorted.drain(..).collect(),
			list_parent: app_list_parent,
			prev_category_name: String::new(),
		};

		let tasks = Tasks::<Task>::new();
		tasks.push(Task::RefreshPinnedApps);

		Ok(Self {
			app_list,
			state,
			tasks,
			marker: PhantomData,
			pinned_apps_parent: pinned_apps_parent.id,
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

fn mount_entry<T>(
	frontend: &mut Frontend<T>,
	parser_state: &mut ParserState,
	doc_params: &ParseDocumentParams,
	id_parent: WidgetID,
	entry: &DesktopEntry,
	tooltip: Option<String>,
) -> anyhow::Result<Rc<ComponentButton>> {
	{
		let mut params = TemplateParams::new();

		if let Some(tooltip) = tooltip {
			params.insert_str("tooltip", tooltip);
		};

		// entry icon
		params.insert_rc(
			"src_ext",
			entry
				.icon_path
				.as_ref()
				.map_or_else(|| "".into(), |icon_path| icon_path.clone()),
		);

		// entry fallback (question mark) icon
		params.insert(
			"src",
			if entry.icon_path.is_none() {
				"dashboard/terminal.svg"
			} else {
				""
			},
		);
		params.insert("name", &entry.app_name);

		let data = parser_state.realize_template(doc_params, "AppEntry", &mut frontend.layout, id_parent, params)?;

		data.fetch_component_as::<ComponentButton>("button")
	}
}

impl AppList {
	fn tick<T>(
		&mut self,
		frontend: &mut Frontend<T>,
		tasks: &Tasks<Task>,
		state: &mut State,
		rc_state: &Rc<RefCell<State>>,
	) -> anyhow::Result<()> {
		let parser_state = &mut state.parser_state;

		// load 30 entries for a single frame at most
		for _ in 0..30 {
			if let Some(entry) = self.entries_to_mount.pop_front() {
				let globals = frontend.layout.state.globals.clone();

				let category_name = get_category_name(&entry);
				if category_name != self.prev_category_name {
					self.prev_category_name = String::from(category_name);
					let mut params = TemplateParams::new();
					params.insert("text", category_name);

					parser_state.realize_template(
						&doc_params(globals.clone()),
						"CategoryText",
						&mut frontend.layout,
						self.list_parent.id,
						params,
					)?;
				}

				let button = mount_entry(
					frontend,
					parser_state,
					&doc_params(globals.clone()),
					self.list_parent.id,
					&entry,
					None,
				)?;

				button.on_click(on_app_click(
					frontend.tasks.clone(),
					tasks.clone(),
					globals.clone(),
					entry.clone(),
					rc_state.clone(),
					None,
				));
			} else {
				break;
			}
		}

		Ok(())
	}
}
