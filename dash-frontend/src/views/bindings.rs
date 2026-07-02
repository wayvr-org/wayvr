use std::{collections::HashMap, rc::Rc};

use glam::Vec2;
use strum::EnumProperty;
use wgui::{
	assets::AssetPath,
	components::{
		button::{ButtonClickEvent, ComponentButton},
		slider::ComponentSlider,
	},
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
		openxr_bindings_schema::{
			BindingsDropdown, ClickType, Component, ControllerProfile, ParsedOpenXrInputPath, Side, SubpathKind, 
		},
		popup_manager::{MountPopupOnceParams, MountPopupOnceParamsExtra, PopupHolder, PopupPadding},
		wgui_simple,
	},
	views::{ViewTrait, ViewUpdateParams},
};

#[derive(Clone)]
enum Task {
	Save,
	Cancel,
	OpenContextMenu(glam::Vec2, Vec<context_menu::Cell>),
	UpdateThreshold(Rc<str>, f32, f32),
}

pub struct Params<'a> {
	pub globals: WguiGlobals,
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub controller_profile: &'static ControllerProfile,
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
	controller_profile: &'static ControllerProfile,
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
				Task::UpdateThreshold(action_name, lo, hi) => {
					let cur_profile = &mut self.profiles[self.cur_profile_idx];
					let action_mut = get_action_mut(cur_profile, &*action_name);

					action_mut.threshold = Some([lo, hi]);
				}
			}
		}

		// Dropdown handling
		if let TickResult::Action(name) = self.context_menu.tick(par.layout, &mut self.parser_state)?
			&& let (Some(action), Some(action_name), Some(side), Some(value)) = {
				let mut s = name.splitn(4, ';');
				(s.next(), s.next(), s.next(), s.next())
			} {
			let side = side.to_lowercase();
			let value = value.to_lowercase();

			log::warn!("{action_name}");

			let cur_profile = &mut self.profiles[self.cur_profile_idx];
			let action_mut = get_action_mut(cur_profile, action_name);
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
					apply_subpath(side_mut, &side, &value, self.controller_profile);
				}
				"comp" => {
					apply_comp(side_mut, &side, &value);
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
			.position(|i| i.profile.as_str() == &*params.controller_profile.profile_id)
			.unwrap_or_else(|| {
				let idx = profiles.len();
				profiles.push(OpenXrInputProfile {
					profile: params.controller_profile.profile_id.to_string(),
					..Default::default()
				});
				idx
			});

		let parser_state = wgui::parser::parse_from_assets(doc_params, params.layout, params.parent_id)?;
		let list_parent = parser_state.fetch_widget(&params.layout.state, "list_parent")?.id;
		let tasks = Tasks::new();

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
			controller_profile: params.controller_profile,
			close_callback: Some(params.close_callback),
		};

		me.ensure_pose_and_haptics();

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
			input_controls_for_action(&mut mp, self.list_parent, action.into(), &self.controller_profile, current)?;
		}

		Ok(())
	}

	fn ensure_pose_and_haptics(&mut self) {
		let cur_profile = &mut self.profiles[self.cur_profile_idx];

		let profile_left = self.controller_profile.find_userpath(Side::Left);
		let profile_right = self.controller_profile.find_userpath(Side::Right);

		let action = cur_profile.pose.get_or_insert_default();

		if action.left.is_none() && profile_left.is_some() {
				let path = "/user/hand/left/input/aim/pose";
				action.left = Some(OneOrMany::One(path.into()));
		}

		if action.right.is_none() && profile_right.is_some() {
				let path = "/user/hand/right/input/aim/pose";
				action.right = Some(OneOrMany::One(path.into()));
		}

		let action = cur_profile.haptic.get_or_insert_default();

		let has_haptic = profile_left.map(|x| x.find_subpath(SubpathKind::Haptic).is_some()).unwrap_or_default();
		if action.left.is_none() && has_haptic {
				let path = "/user/hand/left/output/haptic";
				action.left = Some(OneOrMany::One(path.into()));
		}

		let has_haptic = profile_right.map(|x| x.find_subpath(SubpathKind::Haptic).is_some()).unwrap_or_default();
		if action.right.is_none() && has_haptic {
					let path = "/user/hand/right/output/haptic";
					action.right = Some(OneOrMany::One(path.into()));
		}
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
	controller_profile: &'static ControllerProfile,
) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_raw_text(controller_profile.display_name),
			Box::new(move |data| {
				let close_callback = popup.get_close_callback(data.layout);
				let view = View::new(Params {
					globals: globals.clone(),
					layout: data.layout,
					parent_id: data.id_content,
					controller_profile,
					close_callback,
				})?;

				popup.set_view(data.handle, view, None);
				Ok(popup.get_close_callback(data.layout))
			}),
			MountPopupOnceParamsExtra {
				padding: PopupPadding::None,
			},
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
	profile: &ControllerProfile,
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
		current.threshold,
	)?;

	let current_right = current.right.as_ref().map(|x| match x {
		OneOrMany::One(s) => s.as_str(),
		OneOrMany::Many(s) => s.first().unwrap().as_str(), // safe
	});

	input_controls_for_hand(
		mp,
		parent,
		current_right,
		Side::Right,
		action,
		click_type,
		profile,
		current.threshold,
	)
}

fn input_controls_for_hand(
	mp: &mut MacroParams,
	parent: WidgetID,
	current: Option<&str>,
	side: Side,
	action: Rc<str>,
	click_type: ClickType,
	profile: &ControllerProfile,
	threshold: Option<[f32; 2]>,
) -> anyhow::Result<()> {
	let Some(user_path) = profile.find_userpath(side) else {
		return Ok(()); // this hand is not available
	};

	let current = current.and_then(|cur| ParsedOpenXrInputPath::try_from(cur).log_warn(cur).ok());

	let parent = horiz_cell(mp.layout, parent)?;

		let available_components : Rc<[Component]> = current
		.as_ref()
		.and_then(|par| user_path.find_subpath(par.subpath))
		.map(|subp| subp.components)
		.unwrap_or_default().into();

	let available_subpaths : Rc<[SubpathKind]> = user_path.paths.iter().filter(|x| !x.kind.get_bool("Hidden").unwrap_or_default()).map(|x| x.kind).collect();

	subpath_dropdown(
		mp,
		parent,
		action.clone(),
		side,
		available_subpaths,
		current.as_ref().map(|x| x.subpath),
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

	clicks_dropdown(mp, parent, action.clone(), click_type)?;

	if let Some(component) = current.as_ref().map(|x| x.component)
		&& component.is_analog()
	{
		threshold_slider(mp, parent, action, threshold)?;
	}

	Ok(())
}

fn subpath_dropdown(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	side: Side,
	available: Rc<[SubpathKind]>,
	current: Option<SubpathKind>,
) -> anyhow::Result<()> {
	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("tooltip"), Rc::from("APP_SETTINGS.BINDINGS.SUBPATH"));
	params.insert(Rc::from("min_width"), Rc::from("100"));

	// left/right hand icon
	wgui_simple::create_icon(
		mp.layout,
		parent,
		Vec2::new(32.0, 32.0),
		AssetPath::BuiltIn(&format!("dashboard/hand_{}.svg", side.as_ref().to_lowercase())),
	)?;

	let current_text = current
		.map(|c| c.translation())
		.unwrap_or_else(|| Translation::from_translation_key("APP_SETTINGS.OPTION.NONE"));

	create_dropdown(mp, parent, params, action, side, current_text, available)?;

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

	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("text"), Rc::from("・"));
	params.insert(Rc::from("tooltip"), Rc::from("APP_SETTINGS.BINDINGS.COMPONENT"));
	params.insert(Rc::from("min_width"), Rc::from("100"));

	let current_text = current
		.map(|c| c.translation())
		.unwrap_or_else(|| Translation::from_raw_text_rc(Default::default()));

	create_dropdown(mp, parent, params, action, side, current_text, available)?;

	Ok(true)
}

fn clicks_dropdown(mp: &mut MacroParams, parent: WidgetID, action: Rc<str>, current: ClickType) -> anyhow::Result<()> {
	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("text"), Rc::from("・"));
	params.insert(Rc::from("tooltip"), Rc::from("APP_SETTINGS.BINDINGS.CLICK.TYPE"));
	params.insert(Rc::from("min_width"), Rc::from("100"));

	let current_text = current.translation();
	let available = [ClickType::Any, ClickType::Double, ClickType::Triple].into();
	create_dropdown(mp, parent, params, action, Side::Left, current_text, available)?;

	Ok(())
}

fn threshold_slider(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	current: Option<[f32; 2]>,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let current = current.unwrap_or([0.5, 0.7]);

	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("id"), Rc::from(id.as_ref()));
	params.insert(Rc::from("tooltip"), Rc::from("APP_SETTINGS.BINDINGS.THRESHOLD"));
	params.insert(Rc::from("value"), Rc::from(format!("{:.2}", current[0])));
	params.insert(Rc::from("value2"), Rc::from(format!("{:.2}", current[1])));
	params.insert(Rc::from("min"), Rc::from("0.0"));
	params.insert(Rc::from("max"), Rc::from("1.0"));
	params.insert(Rc::from("step"), Rc::from("0.1"));

	mp.parser_state
		.instantiate_template(mp.doc_params, "ThresholdSlider", mp.layout, parent, params)?;

	let slider = mp.parser_state.fetch_component_as::<ComponentSlider>(&id)?;
	slider.on_value_changed(Box::new({
		let tasks = mp.tasks.clone();
		move |_common, e| {
			if matches!(e.index, wgui::components::slider::ValueIndex::Primary) {
				tasks.push(Task::UpdateThreshold(action.clone(), e.value, current[1]));
			} else {
				tasks.push(Task::UpdateThreshold(action.clone(), current[0], e.value));
			}
		}
	}));

	Ok(())
}

fn create_dropdown<B: 'static + BindingsDropdown>(
	mp: &mut MacroParams,
	parent: WidgetID,
	mut params: HashMap<Rc<str>, Rc<str>>,
	action: Rc<str>,
	side: Side,
	current_text: Translation,
	available: Rc<[B]>,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;
	params.insert(Rc::from("id"), Rc::from(id.as_ref()));

	mp.parser_state
		.instantiate_template(mp.doc_params, "DropdownButton", mp.layout, parent, params)?;

	{
		let mut label = mp
			.parser_state
			.fetch_widget_as::<WidgetLabel>(&mp.layout.state, &format!("{id}_value"))?;
		label.set_text_simple(&mut mp.layout.state.globals.get(), current_text);
	}

	let btn = mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
	btn.on_click(Rc::new({
		let tasks = mp.tasks.clone();
		let available = available.clone();
		move |_common, e: ButtonClickEvent| {
			let mut cells = available
				.iter()
				.map(|item| {
					let title = item.translation();

					context_menu::Cell {
						action_name: Some(item.action_str(&*action, side)),
						title,
						tooltip: None,
						attribs: vec![],
					}
				})
				.collect::<Vec<_>>();

			if let Some(action_str) = B::clear_str(&*action, side) {
				cells.insert(
					0,
					context_menu::Cell {
						action_name: Some(action_str),
						title: Translation::from_translation_key("APP_SETTINGS.OPTION.NONE"),
						tooltip: None,
						attribs: vec![],
					},
				);
			}

			tasks.push(Task::OpenContextMenu(e.mouse_pos_absolute.unwrap_or_default(), cells));
			Ok(())
		}
	}));

	Ok(())
}

fn apply_subpath(side_mut: &mut Option<OneOrMany<String>>, side_str: &str, subpath_str: &str, profile: &ControllerProfile,) {
		let (Ok(side), Ok(subpath)) = (Side::try_from(side_str), SubpathKind::try_from(subpath_str)) else {
			return;
		};
		let Some(subpath_obj) = profile.find_userpath(side).and_then(|p| p.find_subpath(subpath)) else {
			return;
		};

		let comp : Component = if let Some(first) = side_mut.as_ref().map(|x| match x {
			OneOrMany::One(x) => x.as_str(),
			OneOrMany::Many(x) => x.first().unwrap().as_str(),
		}) {
		let Ok(parsed) = ParsedOpenXrInputPath::try_from(first) else {
			return;
		};

		let mut parsed_compo = parsed.component;
		if !subpath_obj.components.contains(&parsed_compo) {
			parsed_compo = *subpath_obj.components.first().unwrap();
		}
		parsed_compo
		} else {
		*subpath_obj.components.first().unwrap()
	};

	let comp_str = comp.as_ref().to_lowercase();

		*side_mut = Some(OneOrMany::One(format!(
			"/user/hand/{side_str}/input/{subpath_str}/{comp_str}"
		)));
}

fn apply_comp(side_mut: &mut Option<OneOrMany<String>>, side: &str, comp: &str) {
		if side_mut.is_none() {
			return;
		}

		let first = match side_mut.as_ref().unwrap() {
			OneOrMany::One(x) => x.as_str(),
			OneOrMany::Many(x) => x.first().unwrap().as_str(),
		};

		let Ok(parsed) = ParsedOpenXrInputPath::try_from(first) else {
			return;
		};

		let subpath = parsed.subpath.as_ref().to_lowercase();

		*side_mut = Some(OneOrMany::One(format!(
			"/user/hand/{side}/input/{subpath}/{comp}"
		)));
}
