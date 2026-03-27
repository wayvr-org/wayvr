use std::{collections::HashMap, marker::PhantomData, rc::Rc};

use wgui::{
	assets::AssetPath,
	components::{
		bar_graph::{ComponentBarGraph, ValueCell},
		button::ComponentButton,
		checkbox::ComponentCheckbox,
		slider::ComponentSlider,
		tabs::ComponentTabs,
	},
	drawing::Color,
	globals::WguiGlobals,
	layout::{Layout, WidgetID},
	parser::{self, Fetchable, ParseDocumentParams, ParserState},
	task::Tasks,
};
use wlx_common::dash_interface::{self, MonadoDumpSessionFrame};

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

	// `DebugTimings` tab
	DebugTimingsRefreshSessionList,
	DebugTimingsSetSessionId(i64),
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

struct DebugGraph {
	graph: Rc<ComponentBarGraph>,
}

struct DebugSessionList {
	#[allow(dead_code)]
	buttons: Vec<Rc<ComponentButton>>,
}

struct TimingsSession {
	resolved_name: Option<String>,
	last_frame: MonadoDumpSessionFrame,
}

struct Graphs {
	predicted_display_time: DebugGraph,
	predicted_frame_time: DebugGraph,
	predicted_wake_up_time: DebugGraph,
	predicted_gpu_done_time: DebugGraph,
	predicted_display_period: DebugGraph,
	display_time: DebugGraph,
	when_predicted: DebugGraph,
	when_wait_woke: DebugGraph,
	when_begin: DebugGraph,
	when_delivered: DebugGraph,
	when_gpu_done: DebugGraph,
}

type SessionsMap = HashMap<i64 /* session id */, TimingsSession>;

struct SubtabDebugTimings {
	#[allow(dead_code)]
	state: ParserState,

	graphs: Option<Graphs>,
	session_list: DebugSessionList,
	selected_session_id: Option<i64>,

	id_sessions_list_parent: WidgetID,
	id_timings_parent: WidgetID,

	sessions: SessionsMap,
}

#[allow(dead_code)]
#[allow(clippy::large_enum_variant)]
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
				Task::DebugTimingsRefreshSessionList => {
					if let Subtab::DebugTimings(tab) = &mut self.subtab {
						tab.refresh_session_list(&mut frontend.layout, &self.tasks)?;
					}
				}
				Task::DebugTimingsSetSessionId(session_id) => {
					if let Subtab::DebugTimings(tab) = &mut self.subtab {
						tab.set_session_id(&mut frontend.layout, session_id)?;
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
							self.subtab = Subtab::DebugTimings(SubtabDebugTimings::new(self.id_content, frontend, &self.tasks)?)
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
				timings.update(&self.tasks, data, frontend)?;
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

fn mount_sessions_list(
	state: &mut ParserState,
	layout: &mut Layout,
	tasks: &Tasks<Task>,
	id_parent: WidgetID,
	sessions: &SessionsMap,
) -> anyhow::Result<DebugSessionList> {
	let mut buttons = Vec::new();
	let globals = layout.state.globals.clone();
	layout.remove_children(id_parent);

	for (session_id, session) in sessions {
		let mut params = HashMap::new();

		params.insert(
			Rc::from("text"),
			Rc::from(format!(
				"{} (ID {})",
				session.resolved_name.as_ref().map_or("Unknown", |s| s.as_str()),
				session_id,
			)),
		);

		let data = state.realize_template(
			&doc_params_tab_debug_timings(&globals),
			"SessionButton",
			layout,
			id_parent,
			params,
		)?;

		let button = data.fetch_component_as::<ComponentButton>("button")?;

		button.on_click({
			let tasks = tasks.clone();
			let session_id = *session_id;
			Rc::new(move |_, _| {
				tasks.push(Task::DebugTimingsSetSessionId(session_id));
				Ok(())
			})
		});

		buttons.push(button);
	}

	Ok(DebugSessionList { buttons })
}

fn mount_graph(
	state: &mut ParserState,
	layout: &mut Layout,
	id_parent: WidgetID,
	name: &'static str,
	limits: (f32, f32),
) -> anyhow::Result<DebugGraph> {
	let globals = layout.state.globals.clone();
	let mut params = HashMap::new();
	params.insert(Rc::from("name"), Rc::from(name));
	params.insert(Rc::from("limit_min"), Rc::from(limits.0.to_string()));
	params.insert(Rc::from("limit_max"), Rc::from(limits.1.to_string()));

	let data = state.realize_template(
		&doc_params_tab_debug_timings(&globals),
		"DebugGraph",
		layout,
		id_parent,
		params,
	)?;

	let graph = data.fetch_component_as::<ComponentBarGraph>("graph")?;
	Ok(DebugGraph { graph })
}

fn ns_to_ms(ns: i64) -> f32 {
	(ns / 1000) as f32 / 1000.0
}

impl SubtabDebugTimings {
	fn new<T>(parent_id: WidgetID, frontend: &mut Frontend<T>, tasks: &Tasks<Task>) -> anyhow::Result<Self> {
		let mut state = wgui::parser::parse_from_assets(
			&doc_params_tab_debug_timings(&frontend.globals),
			&mut frontend.layout,
			parent_id,
		)?;

		let id_timings_parent = state.get_widget_id("timings_parent")?;
		let id_sessions_list_parent = state.get_widget_id("session_list_parent")?;

		let sessions = Default::default();

		let session_list = mount_sessions_list(
			&mut state,
			&mut frontend.layout,
			tasks,
			id_sessions_list_parent,
			&sessions,
		)?;

		Ok(Self {
			state,
			graphs: None,
			session_list,
			id_sessions_list_parent,
			id_timings_parent,
			sessions,
			selected_session_id: None,
		})
	}

	fn set_session_id(&mut self, layout: &mut Layout, session_id: i64) -> anyhow::Result<()> {
		layout.remove_children(self.id_timings_parent);

		let mut graph = |name: &'static str, limits: (f32, f32)| -> anyhow::Result<DebugGraph> {
			mount_graph(&mut self.state, layout, self.id_timings_parent, name, limits)
		};

		// populate graphs
		self.graphs = Some(Graphs {
			predicted_display_time: graph("Predicted display time", (0.0, 30.0))?,
			predicted_frame_time: graph("Predicted frame time", (0.0, 30.0))?,
			predicted_wake_up_time: graph("Predicted wake-up time", (0.0, 30.0))?,
			predicted_gpu_done_time: graph("Predicted GPU done time", (0.0, 30.0))?,
			predicted_display_period: graph("Predicted display period", (0.0, 30.0))?,
			display_time: graph("Display time", (0.0, 30.0))?,
			when_predicted: graph("When predicted", (0.0, 30.0))?,
			when_wait_woke: graph("When wait woke", (0.0, 30.0))?,
			when_begin: graph("When begin", (0.0, 30.0))?,
			when_delivered: graph("When delivered", (0.0, 30.0))?,
			when_gpu_done: graph("When GPU done", (0.0, 30.0))?,
		});

		self.selected_session_id = Some(session_id);

		Ok(())
	}

	fn refresh_session_list(&mut self, layout: &mut Layout, tasks: &Tasks<Task>) -> anyhow::Result<()> {
		self.session_list = mount_sessions_list(
			&mut self.state,
			layout,
			tasks,
			self.id_sessions_list_parent,
			&self.sessions,
		)?;
		Ok(())
	}

	fn update<T>(&mut self, tasks: &Tasks<Task>, data: &mut T, frontend: &mut Frontend<T>) -> anyhow::Result<()> {
		if !frontend.interface.monado_metrics_set_enabled(data, true) {
			return Ok(());
		}

		let frames = frontend.interface.monado_metrics_dump_session_frames(data);
		if frames.is_empty() {
			return Ok(());
		}

		let col_green = Color::new(0.0, 1.0, 0.0, 1.0);

		for frame in frames {
			//log::info!("{:?}", frame);

			match self.sessions.get_mut(&frame.session_id) {
				Some(session) => {
					if let Some(graphs) = &mut self.graphs
						&& let Some(selected_session_id) = self.selected_session_id
						&& selected_session_id == frame.session_id
					{
						let predicted_display_time = ns_to_ms(session.last_frame.predicted_display_time_ns as i64);
						let predicted_frame_time = ns_to_ms(frame.predicted_frame_time_ns as i64);
						let predicted_wake_up_time =
							ns_to_ms(frame.predicted_wake_up_time_ns as i64 - session.last_frame.predicted_wake_up_time_ns as i64);
						let predicted_gpu_done_time =
							ns_to_ms(frame.predicted_gpu_done_time_ns as i64 - session.last_frame.predicted_gpu_done_time_ns as i64);
						let predicted_display_period = ns_to_ms(session.last_frame.predicted_display_period_ns as i64); // 6.944 ms for 144Hz
						let display_time = ns_to_ms(frame.display_time_ns as i64 - session.last_frame.display_time_ns as i64);
						let when_predicted = ns_to_ms(frame.when_predicted_ns as i64 - session.last_frame.when_predicted_ns as i64);
						let when_wait_woke = ns_to_ms(frame.when_wait_woke_ns as i64 - session.last_frame.when_wait_woke_ns as i64);
						let when_begin = ns_to_ms(frame.when_begin_ns as i64 - session.last_frame.when_begin_ns as i64);
						let when_delivered = ns_to_ms(frame.when_delivered_ns as i64 - session.last_frame.when_delivered_ns as i64);
						let when_gpu_done = ns_to_ms(frame.when_gpu_done_ns as i64 - session.last_frame.when_gpu_done_ns as i64);

						graphs.predicted_display_time.graph.push_value(ValueCell {
							value: predicted_display_time,
							color: col_green,
						});

						graphs.predicted_frame_time.graph.push_value(ValueCell {
							value: predicted_frame_time,
							color: col_green,
						});

						graphs.predicted_wake_up_time.graph.push_value(ValueCell {
							value: predicted_wake_up_time,
							color: col_green,
						});

						graphs.predicted_gpu_done_time.graph.push_value(ValueCell {
							value: predicted_gpu_done_time,
							color: col_green,
						});

						graphs.predicted_display_period.graph.push_value(ValueCell {
							value: predicted_display_period,
							color: col_green,
						});

						graphs.display_time.graph.push_value(ValueCell {
							value: display_time,
							color: col_green,
						});

						graphs.when_predicted.graph.push_value(ValueCell {
							value: when_predicted,
							color: col_green,
						});

						graphs.when_wait_woke.graph.push_value(ValueCell {
							value: when_wait_woke,
							color: col_green,
						});

						graphs.when_begin.graph.push_value(ValueCell {
							value: when_begin,
							color: col_green,
						});

						graphs.when_delivered.graph.push_value(ValueCell {
							value: when_delivered,
							color: col_green,
						});

						graphs.when_gpu_done.graph.push_value(ValueCell {
							value: when_gpu_done,
							color: col_green,
						});
					}

					session.last_frame = frame;
				}
				None => {
					self.sessions.insert(
						frame.session_id,
						TimingsSession {
							last_frame: frame,
							resolved_name: None, /* TODO! find client ID from session ID */
						},
					);
					tasks.push(Task::DebugTimingsRefreshSessionList);
				}
			}
		}

		frontend.layout.mark_redraw();

		Ok(())
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
		par.insert(
			"name".into(),
			format!("{} (Client ID: {})", client.name, client.id).into(),
		);
		par.insert("flag_active".into(), yesno(client.is_active).into());
		par.insert("flag_focused".into(), yesno(client.is_focused).into());
		par.insert("flag_io_active".into(), yesno(client.is_io_active).into());
		par.insert("flag_overlay".into(), yesno(client.is_overlay).into());
		par.insert("flag_primary".into(), yesno(client.is_primary).into());
		par.insert("flag_visible".into(), yesno(client.is_visible).into());

		let globals = layout.state.globals.clone();

		let state_cell = self.state.realize_template(
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

		let clients = frontend.interface.monado_client_list(data, true)?;

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
