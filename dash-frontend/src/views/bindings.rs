use std::{borrow::Cow, collections::HashMap, rc::Rc};

use wgui::{
	assets::AssetPath,
	components::button::{ButtonClickEvent, ComponentButton},
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	log::LogErr,
	parser::{Fetchable, ParseDocumentParams, ParserState},
	task::Tasks,
	widget::label::WidgetLabel,
	windowing::context_menu::{self, TickResult},
};
use wlx_common::{
	config_io,
	openxr_actions::{OneOrMany, OpenXrInputAction, OpenXrInputProfile, load_xr_input_profiles},
};

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	tab::settings::horiz_cell,
	util::{
		openxr_bindings_schema::{ClickType, Component, IdentifierType, ParsedOpenXrInputPath, Profile, Side},
		popup_manager::{MountPopupOnceParams, PopupHolder},
	},
	views::{ViewTrait, ViewUpdateParams},
};

#[derive(Clone)]
enum Task {
	Save,
	Cancel,
	OpenContextMenu(glam::Vec2, Vec<context_menu::Cell>),
}

pub struct Params<'a> {
	pub globals: WguiGlobals,
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub profile_id: Rc<str>,
	pub profile: Rc<Profile>,
	pub close_callback: Box<dyn FnOnce()>,
}

pub struct View {
	parser_state: ParserState,
	tasks: Tasks<Task>,
	list_parent: WidgetID,
	globals: WguiGlobals,
	profiles: Vec<OpenXrInputProfile>,
	cur_profile_idx: usize,
	context_menu: context_menu::ContextMenu,
	schema: Rc<Profile>,
	close_callback: Option<Box<dyn FnOnce()>>,
}

impl ViewTrait for View {
	fn update(&mut self, par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		for task in self.tasks.drain() {
			match task {
				Task::Save => {
					let content = serde_json::to_string_pretty(&self.profiles)
						.map_err(|e| anyhow::anyhow!("Failed to serialize profiles: {e}"))?;
					config_io::save("openxr_actions.json5", &content)
						.map_err(|e| anyhow::anyhow!("Failed to save bindings: {e}"))?;

					if let Some(close_callback) = self.close_callback.take() {
						close_callback();
					}
				}
				Task::Cancel => {
					if let Some(close_callback) = self.close_callback.take() {
						close_callback();
					}
				}
				Task::OpenContextMenu(position, cells) => {
					self.context_menu.open(context_menu::OpenParams {
						on_custom_attribs: None,
						position,
						blueprint: context_menu::Blueprint::Cells(cells),
					});
				}
			}
		}

		// Dropdown handling
		if let TickResult::Action(name) = self.context_menu.tick(&mut par.layout, &mut self.parser_state)?
			&& let (Some(action), Some(_), Some(action_name), Some(side), Some(value)) = {
				let mut s = name.splitn(5, ';');
				(s.next(), s.next(), s.next(), s.next(), s.next())
			} {
			let side = side.to_lowercase();
			let value = value.to_lowercase();

			log::warn!("{action_name}");

			let mut cur_profile = &mut self.profiles[self.cur_profile_idx];
			let action_mut = get_action_mut(&mut cur_profile, action_name);
			let side_mut = if side == "right" {
				&mut action_mut.right
			} else {
				&mut action_mut.left
			};

			match action {
				"clear" => {
					*side_mut = None;
				}
				"subpath" => {
					reconstruct_path(&self.schema, side_mut, &side, Some(value.as_str()), None);
				}
				"comp" => {
					reconstruct_path(&self.schema, side_mut, &side, None, Some(value.as_str()));
				}
				"click" => match value.as_str() {
					"triple" => {
						action_mut.triple_click = Some(true);
						action_mut.double_click = None;
					}
					"double" => {
						action_mut.triple_click = None;
						action_mut.double_click = Some(true);
					}
					_ => {
						action_mut.triple_click = None;
						action_mut.double_click = None;
					}
				},
				_ => log::warn!("Unknown action {action}"),
			}

			self.refresh(par.layout)?;
		}

		Ok(())
	}
}

impl View {
	pub fn new(params: Params) -> anyhow::Result<Self> {
		let doc_params = &ParseDocumentParams {
			globals: params.globals.clone(),
			path: AssetPath::BuiltIn("gui/view/bindings.xml"),
			extra: Default::default(),
		};

		let mut profiles = load_xr_input_profiles();

		let cur_profile_idx = profiles
			.iter()
			.position(|i| i.profile.as_str() == &*params.profile_id)
			.unwrap_or_else(|| {
				let idx = profiles.len();
				profiles.push(OpenXrInputProfile {
					profile: params.profile_id.to_string(),
					..Default::default()
				});
				idx
			});

		let parser_state = wgui::parser::parse_from_assets(doc_params, params.layout, params.parent_id)?;
		let list_parent = parser_state.fetch_widget(&params.layout.state, "list_parent")?.id;
		let tasks = Tasks::new();

		{
			let mut title_label = parser_state.fetch_widget_as::<WidgetLabel>(&params.layout.state, "controller_type")?;
			title_label.set_text_simple(
				&mut params.globals.get(),
				Translation::from_raw_text_rc(params.profile.title.clone()),
			);
		}

		tasks.handle_button(
			&parser_state.fetch_component_as::<ComponentButton>("btn_save")?,
			Task::Save,
		);

		tasks.handle_button(
			&parser_state.fetch_component_as::<ComponentButton>("btn_cancel")?,
			Task::Cancel,
		);

		let mut me = Self {
			parser_state,
			tasks,
			list_parent,
			globals: params.globals.clone(),
			profiles,
			cur_profile_idx,
			context_menu: context_menu::ContextMenu::default(),
			schema: params.profile,
			close_callback: Some(params.close_callback),
		};

		me.refresh(params.layout)?;

		Ok(me)
	}

	fn refresh(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		let action_names = [
			"click",
			"grab",
			"alt_click",
			"show_hide",
			"toggle_dashboard",
			"space_drag",
			"space_rotate",
			"space_reset",
			"click_modifier_right",
			"click_modifier_middle",
			"move_mouse",
			"scroll",
		];

		let mut mp = MacroParams {
			parser_state: &mut self.parser_state,
			doc_params: &ParseDocumentParams {
				globals: self.globals.clone(),
				path: AssetPath::BuiltIn("gui/view/bindings.xml"),
				extra: Default::default(),
			},
			layout,
			tasks: self.tasks.clone(),
			idx: 0,
		};

		mp.layout.remove_children(self.list_parent);

		for action in action_names {
			let current = get_action_mut(&mut self.profiles[self.cur_profile_idx], action);
			input_controls_for_action(&mut mp, self.list_parent, action.into(), &self.schema, current)?;
		}

		Ok(())
	}
}

fn get_action_mut<'a>(profile: &'a mut OpenXrInputProfile, action_name: &str) -> &'a mut OpenXrInputAction {
	let action = match action_name {
		"click" => &mut profile.click,
		"grab" => &mut profile.grab,
		"alt_click" => &mut profile.alt_click,
		"show_hide" => &mut profile.show_hide,
		"toggle_dashboard" => &mut profile.toggle_dashboard,
		"space_drag" => &mut profile.space_drag,
		"space_rotate" => &mut profile.space_rotate,
		"space_reset" => &mut profile.space_reset,
		"click_modifier_right" => &mut profile.click_modifier_right,
		"click_modifier_middle" => &mut profile.click_modifier_middle,
		"move_mouse" => &mut profile.move_mouse,
		"scroll" => &mut profile.scroll,
		_ => panic!("unknown action_name: {action_name}"),
	};

	action.get_or_insert_with(OpenXrInputAction::default)
}

pub fn mount_popup(
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	popup: PopupHolder<View>,
	profile_id: Rc<str>,
	profile: Rc<Profile>,
) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_translation_key("APP_SETTINGS.INPUT_PROFILES"),
			Box::new(move |data| {
				let close_callback = popup.get_close_callback(data.layout);
				let view = View::new(Params {
					globals: globals.clone(),
					layout: data.layout,
					parent_id: data.id_content,
					profile_id,
					profile,
					close_callback,
				})?;

				popup.set_view(data.handle, view, None);
				Ok(popup.get_close_callback(data.layout))
			}),
		)));
}

struct MacroParams<'a> {
	pub layout: &'a mut Layout,
	pub parser_state: &'a mut ParserState,
	pub doc_params: &'a ParseDocumentParams<'a>,
	pub tasks: Tasks<Task>,
	pub idx: usize,
}

fn input_controls_for_action(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	profile: &Profile,
	current: &mut OpenXrInputAction,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("id"), Rc::from(id.as_ref()));
	params.insert(
		Rc::from("translation"),
		Rc::from(format!(
			"APP_SETTINGS.BINDINGS.ACTION.{}",
			action.as_ref().to_uppercase()
		)),
	);

	mp.parser_state
		.instantiate_template(mp.doc_params, "ActionRow", mp.layout, parent, params)?;

	let parent = mp.parser_state.get_widget_id(&id)?;

	let click_type = if current.triple_click.unwrap_or_default() {
		ClickType::Triple
	} else if current.double_click.unwrap_or_default() {
		ClickType::Double
	} else {
		ClickType::Any
	};

	let current_left = current.left.as_ref().map(|x| match x {
		OneOrMany::One(s) => s.as_str(),
		OneOrMany::Many(s) => s.first().unwrap().as_str(), // safe
	});

	input_controls_for_hand(
		mp,
		parent,
		current_left,
		Side::Left,
		action.clone(),
		click_type,
		profile,
	)?;

	let current_right = current.right.as_ref().map(|x| match x {
		OneOrMany::One(s) => s.as_str(),
		OneOrMany::Many(s) => s.first().unwrap().as_str(), // safe
	});

	input_controls_for_hand(mp, parent, current_right, Side::Right, action, click_type, profile)
}

fn input_controls_for_hand(
	mp: &mut MacroParams,
	parent: WidgetID,
	current: Option<&str>,
	side: Side,
	action: Rc<str>,
	click_type: ClickType,
	profile: &Profile,
) -> anyhow::Result<()> {
	let subaction_path = match side {
		Side::Left => "/user/hand/left",
		Side::Right => "/user/hand/right",
	};

	if !profile.subaction_paths.iter().any(|p| p == subaction_path) {
		return Ok(()); // skip
	}

	let current = current
		.map(|cur| ParsedOpenXrInputPath::try_from(cur).log_warn(cur).ok())
		.flatten();

	let parent = horiz_cell(mp.layout, parent)?;

	let available_components = current
		.as_ref()
		.map(|par| profile.subpaths.get(&par.to_subpath()))
		.flatten()
		.map(|subp| subp.get_effective_components())
		.unwrap_or_default();

	let available_subpaths = profile
		.subpaths
		.iter()
		.filter(|(_, path)| path.side.is_none_or(|s| s == side))
		.filter_map(|(key, _)| {
			key
				.strip_prefix("/input/")
				.map(|ident| IdentifierType::try_from(ident).ok())
				.flatten()
		})
		.collect::<Rc<[IdentifierType]>>();

	subpath_dropdown(
		mp,
		parent,
		action.clone(),
		side,
		available_subpaths,
		current.as_ref().map(|x| x.identifier),
	)?;

	if !component_dropdown(
		mp,
		parent,
		action.clone(),
		side,
		available_components,
		current.as_ref().map(|x| x.component),
	)? {
		return Ok(());
	}

	clicks_dropdown(mp, parent, action, click_type)?;

	Ok(())
}

fn subpath_dropdown(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	side: Side,
	available: Rc<[IdentifierType]>,
	current: Option<IdentifierType>,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("id"), Rc::from(id.as_ref()));
	params.insert(
		Rc::from("translation"),
		Rc::from(format!("APP_SETTINGS.BINDINGS.{}", side.as_ref().to_uppercase())),
	);
	params.insert(Rc::from("tooltip"), Rc::from("APP_SETTINGS.BINDINGS.SUBPATH"));

	mp.parser_state
		.instantiate_template(mp.doc_params, "DropdownButton", mp.layout, parent, params)?;

	let title = current
		.map(|c| c.translation())
		.unwrap_or_else(|| Translation::from_translation_key("APP_SETTINGS.OPTION.NONE"));

	{
		let mut label = mp
			.parser_state
			.fetch_widget_as::<WidgetLabel>(&mp.layout.state, &format!("{id}_value"))?;
		label.set_text_simple(&mut mp.layout.state.globals.get(), title);
	}

	let btn = mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
	btn.on_click(Rc::new({
		let available = available.clone();
		let tasks = mp.tasks.clone();
		move |_common, e: ButtonClickEvent| {
			let side = side.as_ref();
			let mut cells = available
				.iter()
				.map(|item| {
					let value = item.as_ref();
					let title = item.translation();

					context_menu::Cell {
						action_name: Some(format!("subpath;{id};{action};{side};{value}").into()),
						title,
						tooltip: None,
						attribs: vec![],
					}
				})
				.collect::<Vec<_>>();

			cells.insert(
				0,
				context_menu::Cell {
					action_name: Some(format!("clear;{id};{action};{side};-").into()),
					title: Translation::from_translation_key("APP_SETTINGS.OPTION.NONE"),
					tooltip: None,
					attribs: vec![],
				},
			);

			tasks.push(Task::OpenContextMenu(e.mouse_pos_absolute.unwrap_or_default(), cells));
			Ok(())
		}
	}));

	Ok(())
}

fn component_dropdown(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	side: Side,
	available: Rc<[Component]>,
	current: Option<Component>,
) -> anyhow::Result<bool> {
	if available.is_empty() {
		return Ok(false);
	}

	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("id"), Rc::from(id.as_ref()));
	params.insert(Rc::from("text"), Rc::from("・"));
	params.insert(Rc::from("tooltip"), Rc::from("APP_SETTINGS.BINDINGS.COMPONENT"));

	mp.parser_state
		.instantiate_template(mp.doc_params, "DropdownButton", mp.layout, parent, params)?;

	let title = current
		.map(|c| c.translation())
		.unwrap_or_else(|| Translation::from_raw_text_rc(Default::default()));

	{
		let mut label = mp
			.parser_state
			.fetch_widget_as::<WidgetLabel>(&mp.layout.state, &format!("{id}_value"))?;
		label.set_text_simple(&mut mp.layout.state.globals.get(), title);
	}

	let btn = mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
	btn.on_click(Rc::new({
		let tasks = mp.tasks.clone();
		let available = available.clone();
		move |_common, e: ButtonClickEvent| {
			tasks.push(Task::OpenContextMenu(
				e.mouse_pos_absolute.unwrap_or_default(),
				available
					.iter()
					.filter_map(|item| {
						let value = item.as_ref();
						let title = item.translation();
						let side = side.as_ref();

						Some(context_menu::Cell {
							action_name: Some(format!("comp;{id};{action};{side};{value}").into()),
							title,
							tooltip: None,
							attribs: vec![],
						})
					})
					.collect(),
			));
			Ok(())
		}
	}));

	Ok(true)
}

fn clicks_dropdown(mp: &mut MacroParams, parent: WidgetID, action: Rc<str>, current: ClickType) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("id"), Rc::from(id.as_ref()));
	params.insert(Rc::from("text"), Rc::from("・"));
	params.insert(Rc::from("tooltip"), Rc::from("APP_SETTINGS.BINDINGS.CLICK.TYPE"));

	mp.parser_state
		.instantiate_template(mp.doc_params, "DropdownButton", mp.layout, parent, params)?;

	let title = current.translation();

	{
		let mut label = mp
			.parser_state
			.fetch_widget_as::<WidgetLabel>(&mp.layout.state, &format!("{id}_value"))?;
		label.set_text_simple(&mut mp.layout.state.globals.get(), title);
	}

	let btn = mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
	btn.on_click(Rc::new({
		let tasks = mp.tasks.clone();
		move |_common, e: ButtonClickEvent| {
			tasks.push(Task::OpenContextMenu(
				e.mouse_pos_absolute.unwrap_or_default(),
				[ClickType::Any, ClickType::Double, ClickType::Triple]
					.iter()
					.filter_map(|item| {
						let value = item.as_ref();
						let title = item.translation();

						Some(context_menu::Cell {
							action_name: Some(format!("click;{id};{action};-;{value}").into()),
							title,
							tooltip: None,
							attribs: vec![],
						})
					})
					.collect(),
			));
			Ok(())
		}
	}));

	Ok(())
}

fn reconstruct_path(
	schema: &Profile,
	side_mut: &mut Option<OneOrMany<String>>,
	side: &str,
	subpath: Option<&str>,
	comp: Option<&str>,
) {
	if side_mut.is_none() {
		let Some(subpath) = subpath else {
			return;
		};

		if let Some(comp) = comp {
			*side_mut = Some(OneOrMany::One(format!("/user/hand/{side}/input/{subpath}/{comp}")));
		} else {
			let key = format!("/input/{subpath}");
			let Some(schema_subpath) = schema.subpaths.get(&key) else {
				return;
			};
			let comps = schema_subpath.get_effective_components();
			let comp = comps.first().unwrap().as_ref(); // safe
			*side_mut = Some(OneOrMany::One(format!("/user/hand/{side}/input/{subpath}/{comp}")));
		}
	} else {
		let first = match side_mut.as_ref().unwrap() {
			OneOrMany::One(x) => x.as_str(),
			OneOrMany::Many(x) => x.first().unwrap().as_str(),
		};

		let Ok(parsed) = ParsedOpenXrInputPath::try_from(first) else {
			return;
		};

		let new_subpath = subpath.map_or_else(|| Cow::Owned(parsed.identifier.as_ref().to_lowercase()), Cow::Borrowed);

		let mut parsed_compo = parsed.component;
		let key = format!("/input/{new_subpath}");
		let Some(schema_subpath) = schema.subpaths.get(&key) else {
			return;
		};
		let effective_compo = schema_subpath.get_effective_components();
		if !effective_compo.contains(&parsed_compo) {
			parsed_compo = *effective_compo.first().unwrap();
		}

		let new_comp = comp.map_or_else(|| Cow::Owned(parsed_compo.as_ref().to_lowercase()), Cow::Borrowed);

		*side_mut = Some(OneOrMany::One(format!(
			"/user/hand/{side}/input/{new_subpath}/{new_comp}"
		)));
	}
}
