use std::{collections::HashMap, marker::PhantomData, rc::Rc};

use wgui::{
	assets::AssetPath,
	components::{
		bar_graph::{ComponentBarGraph, ValueCell},
		checkbox::ComponentCheckbox,
		slider::ComponentSlider,
		tabs::ComponentTabs,
	},
	drawing,
	globals::WguiGlobals,
	layout::{Layout, WidgetID},
	parser::{self, Fetchable, ParseDocumentParams, ParserState},
	task::Tasks,
};
use wlx_common::dash_interface;

use crate::{
	frontend::Frontend,
	tab::{Tab, TabType},
};

#[derive(Clone)]
enum TabNameEnum {
	GeneralSettings,
	ProcessList,
	DebugTimings,
}

impl TabNameEnum {
	fn from_string(s: &str) -> Option<Self> {
		match s {
			"general_settings" => Some(TabNameEnum::GeneralSettings),
			"process_list" => Some(TabNameEnum::ProcessList),
			"debug_timings" => Some(TabNameEnum::DebugTimings),
			_ => None,
		}
	}
}

enum Task {
	SetBrightness(f32),
	SetTab(TabNameEnum),

	// `ProcessList` tab
	ProcessListRefresh,
	ProcessListFocusClient(String),
}

struct SubtabProcessList {
	id_list_parent: WidgetID,
	state: ParserState,
	cells: Vec<parser::ParserData>,
}

struct SubtabGeneralSettings {
	#[allow(dead_code)]
	state: ParserState,
}

struct SubtabDebugTimings {
	#[allow(dead_code)]
	state: ParserState,

	graph_first: Rc<ComponentBarGraph>,
	graph_second: Rc<ComponentBarGraph>,
}

#[allow(dead_code)]
enum Subtab {
	Empty,
	GeneralSettings(SubtabGeneralSettings),
	ProcessList(SubtabProcessList),
	DebugTimings(SubtabDebugTimings),
}

pub struct TabMonado<T> {
	#[allow(dead_code)]
	state: ParserState,
	tasks: Tasks<Task>,

	marker: PhantomData<T>,

	id_content: WidgetID,
	subtab: Subtab,

	ticks: u32,
}

impl<T> Tab<T> for TabMonado<T> {
	fn get_type(&self) -> TabType {
		TabType::Games
	}

	fn update(&mut self, frontend: &mut Frontend<T>, _time_ms: u32, data: &mut T) -> anyhow::Result<()> {
		for task in self.tasks.drain() {
			match task {
				Task::ProcessListRefresh => {
					if let Subtab::ProcessList(process_list) = &mut self.subtab {
						process_list.refresh(frontend, data, &self.tasks)?;
					}
				}
				Task::ProcessListFocusClient(client_name) => {
					if let Subtab::ProcessList(process_list) = &mut self.subtab {
						process_list.focus_client(frontend, data, client_name, &self.tasks)?;
					}
				}
				Task::SetBrightness(brightness) => self.set_brightness(frontend, data, brightness),
				Task::SetTab(tab) => {
					frontend.layout.remove_children(self.id_content);
					match tab {
						TabNameEnum::GeneralSettings => {
							self.subtab = Subtab::GeneralSettings(SubtabGeneralSettings::new(
								self.id_content,
								frontend,
								data,
								&self.tasks,
							)?)
						}
						TabNameEnum::ProcessList => {
							self.tasks.push(Task::ProcessListRefresh);
							self.subtab = Subtab::ProcessList(SubtabProcessList::new(self.id_content, frontend)?)
						}
						TabNameEnum::DebugTimings => {
							self.subtab = Subtab::DebugTimings(SubtabDebugTimings::new(self.id_content, frontend)?)
						}
					}
				}
			}
		}

		match &mut self.subtab {
			Subtab::Empty => {}
			Subtab::GeneralSettings(_) => {}
			Subtab::ProcessList(_) => {
				// every few seconds
				if let Subtab::ProcessList(_) = &self.subtab
					&& self.ticks.is_multiple_of(500)
				{
					self.tasks.push(Task::ProcessListRefresh);
				}
			}
			Subtab::DebugTimings(timings) => {
				timings.update(&mut frontend.layout);
			}
		}

		self.ticks += 1;

		Ok(())
	}
}

fn doc_params_monado(globals: &'_ WguiGlobals) -> ParseDocumentParams<'_> {
	ParseDocumentParams {
		globals: globals.clone(),
		path: AssetPath::BuiltIn("gui/tab/monado.xml"),
		extra: Default::default(),
	}
}

fn doc_params_tab_process_list(globals: &'_ WguiGlobals) -> ParseDocumentParams<'_> {
	ParseDocumentParams {
		globals: globals.clone(),
		path: AssetPath::BuiltIn("gui/tab/monado_tab_process_list.xml"),
		extra: Default::default(),
	}
}

fn doc_params_tab_general_settings(globals: &'_ WguiGlobals) -> ParseDocumentParams<'_> {
	ParseDocumentParams {
		globals: globals.clone(),
		path: AssetPath::BuiltIn("gui/tab/monado_tab_general_settings.xml"),
		extra: Default::default(),
	}
}

fn doc_params_tab_debug_timings(globals: &'_ WguiGlobals) -> ParseDocumentParams<'_> {
	ParseDocumentParams {
		globals: globals.clone(),
		path: AssetPath::BuiltIn("gui/tab/monado_tab_debug_timings.xml"),
		extra: Default::default(),
	}
}

fn yesno(n: bool) -> &'static str {
	match n {
		true => "yes",
		false => "no",
	}
}

impl SubtabGeneralSettings {
	fn new<T>(
		parent_id: WidgetID,
		frontend: &mut Frontend<T>,
		data: &mut T,
		tasks: &Tasks<Task>,
	) -> anyhow::Result<Self> {
		let state = wgui::parser::parse_from_assets(
			&doc_params_tab_general_settings(&frontend.globals),
			&mut frontend.layout,
			parent_id,
		)?;

		// get brightness
		let slider_brightness = state.fetch_component_as::<ComponentSlider>("slider_brightness")?;
		if let Some(brightness) = frontend.interface.monado_brightness_get(data) {
			let mut c = frontend.layout.start_common();
			slider_brightness.set_value(&mut c.common(), brightness * 100.0);
			c.finish()?;

			slider_brightness.on_value_changed({
				let tasks = tasks.clone();
				Box::new(move |_common, e| {
					tasks.push(Task::SetBrightness(e.value / 100.0));
					Ok(())
				})
			});
		}

		Ok(Self { state })
	}
}

impl SubtabDebugTimings {
	fn new<T>(parent_id: WidgetID, frontend: &mut Frontend<T>) -> anyhow::Result<Self> {
		let state = wgui::parser::parse_from_assets(
			&doc_params_tab_debug_timings(&frontend.globals),
			&mut frontend.layout,
			parent_id,
		)?;

		let graph_first = state.fetch_component_as::<ComponentBarGraph>("graph_first")?;
		let graph_second = state.fetch_component_as::<ComponentBarGraph>("graph_second")?;

		Ok(Self {
			state,
			graph_first,
			graph_second,
		})
	}

	fn update(&mut self, layout: &mut Layout) {
		self.graph_first.push_value(ValueCell {
			value: rand::random_range(0.0..50.0),
			color: drawing::Color::new(rand::random_range(0.0..1.0), rand::random_range(0.0..1.0), 0.0, 1.0),
		});

		self.graph_second.push_value(ValueCell {
			value: rand::random_range(0.0..30.0),
			color: drawing::Color::new(0.0, rand::random_range(0.0..1.0), rand::random_range(0.0..1.0), 1.0),
		});

		layout.mark_redraw();
	}
}

impl SubtabProcessList {
	fn new<T>(parent_id: WidgetID, frontend: &mut Frontend<T>) -> anyhow::Result<Self> {
		let state = wgui::parser::parse_from_assets(
			&doc_params_tab_process_list(&frontend.globals),
			&mut frontend.layout,
			parent_id,
		)?;
		let id_list_parent = state.get_widget_id("list_parent")?;

		Ok(Self {
			state,
			id_list_parent,
			cells: Vec::new(),
		})
	}

	fn mount_client(
		&mut self,
		layout: &mut Layout,
		client: &dash_interface::MonadoClient,
		tasks: &Tasks<Task>,
	) -> anyhow::Result<()> {
		let mut par = HashMap::<Rc<str>, Rc<str>>::new();
		par.insert(
			"checked".into(),
			if client.is_primary {
				Rc::from("1")
			} else {
				Rc::from("0")
			},
		);
		par.insert("name".into(), client.name.clone().into());
		par.insert("flag_active".into(), yesno(client.is_active).into());
		par.insert("flag_focused".into(), yesno(client.is_focused).into());
		par.insert("flag_io_active".into(), yesno(client.is_io_active).into());
		par.insert("flag_overlay".into(), yesno(client.is_overlay).into());
		par.insert("flag_primary".into(), yesno(client.is_primary).into());
		par.insert("flag_visible".into(), yesno(client.is_visible).into());

		let globals = layout.state.globals.clone();

		let state_cell = self.state.parse_template(
			&doc_params_tab_process_list(&globals),
			"Cell",
			layout,
			self.id_list_parent,
			par,
		)?;

		let checkbox = state_cell.fetch_component_as::<ComponentCheckbox>("checkbox")?;
		checkbox.on_toggle({
			let tasks = tasks.clone();
			let client_name = client.name.clone();
			Box::new(move |_common, e| {
				if e.checked {
					tasks.push(Task::ProcessListFocusClient(client_name.clone()));
				}
				Ok(())
			})
		});

		self.cells.push(state_cell);

		Ok(())
	}

	fn focus_client<T>(
		&mut self,
		frontend: &mut Frontend<T>,
		data: &mut T,
		name: String,
		tasks: &Tasks<Task>,
	) -> anyhow::Result<()> {
		frontend.interface.monado_client_focus(data, &name)?;
		tasks.push(Task::ProcessListRefresh);
		Ok(())
	}

	fn refresh<T>(&mut self, frontend: &mut Frontend<T>, data: &mut T, tasks: &Tasks<Task>) -> anyhow::Result<()> {
		log::debug!("refreshing monado client list");

		let clients = frontend.interface.monado_client_list(data)?;

		frontend.layout.remove_children(self.id_list_parent);
		self.cells.clear();

		for client in clients {
			self.mount_client(&mut frontend.layout, &client, tasks)?;
		}

		Ok(())
	}
}

impl<T> TabMonado<T> {
	pub fn new(frontend: &mut Frontend<T>, parent_id: WidgetID) -> anyhow::Result<Self> {
		let globals = frontend.layout.state.globals.clone();
		let state = wgui::parser::parse_from_assets(&doc_params_monado(&globals), &mut frontend.layout, parent_id)?;
		let id_content = state.get_widget_id("content")?;
		let tabs = state.fetch_component_as::<ComponentTabs>("tabs")?;

		let tasks = Tasks::<Task>::new();

		tabs.on_select({
			let tasks = tasks.clone();
			Rc::new(move |_common, evt| {
				if let Some(tab) = TabNameEnum::from_string(&evt.name) {
					tasks.push(Task::SetTab(tab));
				}
				Ok(())
			})
		});

		tasks.push(Task::SetTab(TabNameEnum::ProcessList));

		Ok(Self {
			state,
			marker: PhantomData,
			tasks,
			id_content,
			ticks: 0,
			subtab: Subtab::Empty,
		})
	}

	fn set_brightness(&mut self, frontend: &mut Frontend<T>, data: &mut T, brightness: f32) {
		frontend.interface.monado_brightness_set(data, brightness);
	}
}
