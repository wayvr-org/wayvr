use glam::Vec2;
use std::rc::Rc;
use strum::EnumProperty;
use wgui::{
	assets::AssetPathRef,
	components::{button::ComponentButton, slider::ComponentSlider},
	i18n::Translation,
	layout::{Layout, WidgetID},
	log::LogErr,
	parser::{Fetchable, ParseDocumentParams, ParserState, TemplateParams},
	task::Tasks,
	windowing::context_menu::{self, TickResult},
};
use wlx_common::{
	config_io,
	openxr_actions::{OneOrMany, OpenXrInputAction, OpenXrInputChordMember, OpenXrInputProfile, load_xr_input_profiles},
	openxr_bindings_schema::{
		DEFAULT_BUTTON_THRESHOLDS, XrControllerProfile, XrInputComponent, XrInputSide, XrInputSubpathKind,
	},
};

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	tab::settings::horiz_cell,
	util::{
		openxr_bindings::{BindingsDropdown, BindingsDropdownAction, ClickType, ParsedOpenXrInputPath},
		popup_manager::{MountPopupOnceParams, MountPopupOnceParamsExtra, PopupHolder, PopupPadding},
		wgui_simple::{self, DropdownOpenEvent},
	},
	views::{ViewTrait, ViewUpdateParams},
};

#[derive(Clone)]
enum DropdownAction {
	ChordHand {
		action: Rc<str>,
		side: XrInputSide,
		new_side: XrInputSide,
		member_idx: usize,
	},
	ChordSubpath {
		action: Rc<str>,
		side: XrInputSide,
		subpath: XrInputSubpathKind,
		member_idx: usize,
	},
	ChordComp {
		action: Rc<str>,
		side: XrInputSide,
		component: XrInputComponent,
		member_idx: usize,
	},
	Bindings(BindingsDropdownAction),
}

impl DropdownAction {
	fn into_cell(self) -> context_menu::Action {
		context_menu::Action::Custom(Rc::new(self))
	}
}

#[derive(Clone)]
enum Task {
	Save,
	Cancel,
	OpenDropdown(DropdownOpenEvent),
	UpdateThreshold(Rc<str>, XrInputSide, usize, f32),
	AddChordMember(Rc<str>, XrInputSide),
	RemoveChordMember(Rc<str>, XrInputSide, usize),
	UpdateChordThreshold(Rc<str>, XrInputSide, usize, usize, f32),
}

pub struct Params<'a> {
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub controller_profile: &'static XrControllerProfile,
	pub close_callback: Box<dyn FnOnce()>,
}

pub struct View {
	parser_state: ParserState,
	tasks: Tasks<Task>,
	list_parent: WidgetID,
	profiles: Vec<OpenXrInputProfile>,
	cur_profile_idx: usize,
	context_menu: context_menu::ContextMenu,
	controller_profile: &'static XrControllerProfile,
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
				Task::OpenDropdown(evt) => {
					self.context_menu.open(context_menu::OpenParams {
						on_custom_attribs: None,
						position: context_menu::Position::Widget(evt.widget),
						blueprint: context_menu::Blueprint::Cells(evt.cells),
					});
				}
				Task::UpdateThreshold(action_name, side, i, val) => {
					let cur_profile = &mut self.profiles[self.cur_profile_idx];
					let action_mut = get_action_mut(cur_profile, &action_name);

					let threshold = if matches!(side, XrInputSide::Right) {
						action_mut.threshold_right.get_or_insert(DEFAULT_BUTTON_THRESHOLDS)
					} else {
						action_mut.threshold_left.get_or_insert(DEFAULT_BUTTON_THRESHOLDS)
					};

					threshold[i] = val;
				}
				Task::AddChordMember(action_name, output_side) => {
					if let Some(member) = default_chord_member(self.controller_profile, output_side) {
						let cur_profile = &mut self.profiles[self.cur_profile_idx];
						let action_mut = get_action_mut(cur_profile, &action_name);
						action_chord_mut(action_mut, output_side)
							.get_or_insert_default()
							.push(member);
						self.refresh(par.layout)?;
					}
				}
				Task::RemoveChordMember(action_name, output_side, member_idx) => {
					let cur_profile = &mut self.profiles[self.cur_profile_idx];
					let action_mut = get_action_mut(cur_profile, &action_name);
					let chord = action_chord_mut(action_mut, output_side);
					let chord_empty = if let Some(members) = chord.as_mut()
						&& member_idx < members.len()
					{
						members.remove(member_idx);
						members.is_empty()
					} else {
						false
					};
					if chord_empty {
						*chord = None;
					}
					self.refresh(par.layout)?;
				}
				Task::UpdateChordThreshold(action_name, output_side, member_idx, i, val) => {
					let cur_profile = &mut self.profiles[self.cur_profile_idx];
					let action_mut = get_action_mut(cur_profile, &action_name);
					if let Some(member) = action_chord_mut(action_mut, output_side)
						.as_mut()
						.and_then(|members| members.get_mut(member_idx))
					{
						let mut threshold = member.threshold().unwrap_or(DEFAULT_BUTTON_THRESHOLDS);
						threshold[i] = val;
						set_chord_member_threshold(member, threshold);
					}
				}
			}
		}

		// Dropdown handling
		if let TickResult::Action(action) = self.context_menu.tick(par.layout, &mut self.parser_state)? {
			match action {
				context_menu::Action::Empty => {}
				context_menu::Action::Named(_) => {}
				context_menu::Action::Custom(action) => {
					let action = action.as_ref().downcast_ref::<DropdownAction>().unwrap() /* want panic */;
					self.handle_action(par.layout, action)?;
				}
			}
		}

		Ok(())
	}
}

impl View {
	pub fn new(params: Params) -> anyhow::Result<Self> {
		let doc_params = ParseDocumentParams::builtin("gui/view/bindings.xml");

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

		let parser_state = wgui::parser::parse_from_assets(&doc_params, params.layout, params.parent_id)?;
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

	fn handle_action(&mut self, layout: &mut Layout, action: &DropdownAction) -> anyhow::Result<()> {
		let cur_profile = &mut self.profiles[self.cur_profile_idx];

		match action {
			DropdownAction::ChordHand {
				action,
				side,
				new_side,
				member_idx,
			} => {
				let action_mut = get_action_mut(cur_profile, action);
				if let Some(member) = get_member(action_mut, side, *member_idx) {
					apply_chord_hand(member, new_side, self.controller_profile);
				}
			}
			DropdownAction::ChordSubpath {
				action,
				side,
				subpath,
				member_idx,
			} => {
				let action_mut = get_action_mut(cur_profile, action);
				if let Some(member) = get_member(action_mut, side, *member_idx) {
					apply_chord_subpath(member, subpath, self.controller_profile);
				}
			}
			DropdownAction::ChordComp {
				action,
				side,
				member_idx,
				component,
			} => {
				let action_mut = get_action_mut(cur_profile, action);
				if let Some(member) = get_member(action_mut, side, *member_idx) {
					apply_chord_component(member, component);
				}
			}
			DropdownAction::Bindings(action) => match action {
				BindingsDropdownAction::Click { action, click_type } => {
					let action_mut = get_action_mut(cur_profile, action);
					apply_click_count(action_mut, click_type)
				}
				BindingsDropdownAction::Subpath { action, side, subpath } => {
					let action_mut = get_action_mut(cur_profile, action);
					apply_subpath(
						action_binding_mut(action_mut, side),
						action,
						side,
						subpath,
						self.controller_profile,
					);
				}
				BindingsDropdownAction::Comp { action, side, comp } => {
					let action_mut = get_action_mut(cur_profile, action);
					apply_comp(action_binding_mut(action_mut, side), side, comp);
				}
				BindingsDropdownAction::Clear { action, side } => {
					let action_mut = get_action_mut(cur_profile, action);
					*action_binding_mut(action_mut, side) = None;
				}
			},
		}
		self.refresh(layout)?;
		Ok(())
	}

	fn refresh(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		let action_names = [
			"click",
			"grab",
			"scroll",
			"show_hide",
			"toggle_dashboard",
			"space_drag",
			"space_rotate",
			"space_reset",
			"click_modifier_right",
			"click_modifier_middle",
			"alt_click",
			"move_mouse",
		];

		let mut mp = MacroParams {
			parser_state: &mut self.parser_state,
			doc_params: &ParseDocumentParams::builtin("gui/view/bindings.xml"),
			layout,
			tasks: self.tasks.clone(),
			id_counter: 0,
		};

		mp.layout.remove_children(self.list_parent);

		for action in action_names {
			let current = get_action_mut(&mut self.profiles[self.cur_profile_idx], action);
			input_controls_for_action(
				&mut mp,
				self.list_parent,
				action.into(),
				self.controller_profile,
				current,
			)?;
		}

		Ok(())
	}

	fn ensure_pose_and_haptics(&mut self) {
		let cur_profile = &mut self.profiles[self.cur_profile_idx];

		let profile_left = self.controller_profile.find_userpath(XrInputSide::Left);
		let profile_right = self.controller_profile.find_userpath(XrInputSide::Right);

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

		let has_haptic = profile_left
			.map(|x| x.find_subpath(XrInputSubpathKind::Haptic).is_some())
			.unwrap_or_default();
		if action.left.is_none() && has_haptic {
			let path = "/user/hand/left/output/haptic";
			action.left = Some(OneOrMany::One(path.into()));
		}

		let has_haptic = profile_right
			.map(|x| x.find_subpath(XrInputSubpathKind::Haptic).is_some())
			.unwrap_or_default();
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

fn get_member<'a>(
	action_mut: &'a mut OpenXrInputAction,
	output_side: &XrInputSide,
	member_idx: usize,
) -> Option<&'a mut OpenXrInputChordMember> {
	action_chord_mut(action_mut, *output_side)
		.as_mut()
		.and_then(|members| members.get_mut(member_idx))
}

pub fn mount_popup(
	frontend_tasks: FrontendTasks,
	popup: PopupHolder<View>,
	controller_profile: &'static XrControllerProfile,
) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_raw_text(controller_profile.display_name),
			Box::new(move |data| {
				let close_callback = popup.get_close_callback(data.layout);
				let view = View::new(Params {
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
	pub id_counter: usize,
}

impl<'a> MacroParams<'a> {
	pub fn gen_id(&mut self) -> Rc<str> {
		let id = format!("dyn{}", self.id_counter);
		self.id_counter += 1;
		id.into()
	}
}

fn input_controls_for_action(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	profile: &XrControllerProfile,
	current: &mut OpenXrInputAction,
) -> anyhow::Result<()> {
	let id = mp.gen_id();

	let mut params = TemplateParams::new();
	params.insert("id", id.clone());
	params.insert(
		"translation",
		format!("APP_SETTINGS.BINDINGS.ACTION.{}", action.as_ref().to_uppercase()),
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
		InputControlsForHandParams {
			parent,
			current: current_left,
			side: XrInputSide::Left,
			action: &action,
			click_type,
			profile,
			threshold: current.threshold_left,
		},
	)?;

	let current_right = current.right.as_ref().map(|x| match x {
		OneOrMany::One(s) => s.as_str(),
		OneOrMany::Many(s) => s.first().unwrap().as_str(), // safe
	});

	input_controls_for_hand(
		mp,
		InputControlsForHandParams {
			parent,
			current: current_right,
			side: XrInputSide::Right,
			action: &action,
			click_type,
			profile,
			threshold: current.threshold_right,
		},
	)?;

	if &*action != "scroll" {
		if is_side_agnostic_action(action.as_ref()) {
			chord_controls_for_output(
				mp,
				parent,
				&action,
				XrInputSide::Left,
				current.left_chord.as_deref(),
				click_type,
				profile,
				false,
			)?;
		} else {
			for output_side in [XrInputSide::Left, XrInputSide::Right] {
				if profile.find_userpath(output_side).is_none() {
					continue;
				}
				let chord = if matches!(output_side, XrInputSide::Right) {
					current.right_chord.as_deref()
				} else {
					current.left_chord.as_deref()
				};
				chord_controls_for_output(mp, parent, &action, output_side, chord, click_type, profile, true)?;
			}
		}
	}

	Ok(())
}

fn is_side_agnostic_action(action: &str) -> bool {
	matches!(action, "show_hide" | "toggle_dashboard")
}

#[allow(clippy::too_many_arguments)]
fn chord_controls_for_output(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: &Rc<str>,
	output_side: XrInputSide,
	members: Option<&[OpenXrInputChordMember]>,
	click_type: ClickType,
	profile: &XrControllerProfile,
	show_output_side: bool,
) -> anyhow::Result<()> {
	let header = horiz_cell(mp.layout, parent)?;

	if show_output_side {
		wgui_simple::create_icon(
			mp.layout,
			header,
			Vec2::new(24.0, 24.0),
			AssetPathRef::BuiltIn(&format!("dashboard/hand_{}.svg", output_side.as_ref())),
		)?;
		wgui_simple::create_label(mp.layout, header, side_translation(output_side))?;
	}

	wgui_simple::create_label(
		mp.layout,
		header,
		Translation::from_translation_key("APP_SETTINGS.BINDINGS.CHORD"),
	)?;

	clicks_dropdown(mp, header, action.clone(), click_type)?;

	let tasks = mp.tasks.clone();
	let action_for_add = action.clone();
	wgui_simple::create_button(wgui_simple::CreateButtonParams {
		id_parent: header,
		layout: mp.layout,
		content: Translation::from_translation_key("APP_SETTINGS.BINDINGS.CHORD_ADD_INPUT"),
		icon_builtin: AssetPathRef::BuiltIn("dashboard/add.svg"),
		on_click: Rc::new(move |_common, _event| {
			tasks.push(Task::AddChordMember(action_for_add.clone(), output_side));
			Ok(())
		}),
	})?;

	for (member_idx, member) in members.unwrap_or_default().iter().enumerate() {
		chord_member_controls(mp, parent, action, output_side, member_idx, member, profile)?;
	}

	Ok(())
}

#[allow(clippy::too_many_arguments)]
fn chord_member_controls(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: &Rc<str>,
	output_side: XrInputSide,
	member_idx: usize,
	member: &OpenXrInputChordMember,
	profile: &XrControllerProfile,
) -> anyhow::Result<()> {
	let row = horiz_cell(mp.layout, parent)?;
	let parsed = ParsedOpenXrInputPath::try_from(member.path())
		.log_warn(member.path())
		.ok();
	let physical_side = parsed.as_ref().map(|x| x.side).unwrap_or(output_side);

	chord_hand_dropdown(mp, row, action.clone(), output_side, member_idx, profile, physical_side)?;

	let available_subpaths: Rc<[XrInputSubpathKind]> = profile
		.find_userpath(physical_side)
		.map(|user_path| {
			user_path
				.paths
				.iter()
				.filter(|x| !x.kind.get_bool("Hidden").unwrap_or_default())
				.map(|x| x.kind)
				.collect::<Vec<_>>()
				.into()
		})
		.unwrap_or_default();

	chord_subpath_dropdown(
		mp,
		row,
		action.clone(),
		output_side,
		member_idx,
		available_subpaths,
		parsed.as_ref().map(|x| x.subpath),
	)?;

	let available_components: Rc<[XrInputComponent]> = parsed
		.as_ref()
		.and_then(|parsed| {
			profile
				.find_userpath(parsed.side)
				.and_then(|user_path| user_path.find_subpath(parsed.subpath))
		})
		.map(|subpath| subpath.components)
		.unwrap_or_default()
		.into();

	chord_component_dropdown(
		mp,
		row,
		action.clone(),
		output_side,
		member_idx,
		available_components,
		parsed.and_then(|x| x.component),
	)?;

	let tasks = mp.tasks.clone();
	let action_for_remove = action.clone();
	wgui_simple::create_button(wgui_simple::CreateButtonParams {
		id_parent: row,
		layout: mp.layout,
		content: Translation::from_raw_text(""),
		icon_builtin: AssetPathRef::BuiltIn("dashboard/trash.svg"),
		on_click: Rc::new(move |_common, _event| {
			tasks.push(Task::RemoveChordMember(
				action_for_remove.clone(),
				output_side,
				member_idx,
			));
			Ok(())
		}),
	})?;

	// put threshold slider on its own row
	if parsed.and_then(|x| x.component).is_some_and(|x| x.is_analog()) {
		chord_threshold_slider(mp, parent, action.clone(), output_side, member_idx, member.threshold())?;
	}

	Ok(())
}

struct InputControlsForHandParams<'a> {
	parent: WidgetID,
	current: Option<&'a str>,
	side: XrInputSide,
	action: &'a Rc<str>,
	click_type: ClickType,
	profile: &'a XrControllerProfile,
	threshold: Option<[f32; 2]>,
}

fn input_controls_for_hand(mp: &mut MacroParams, par: InputControlsForHandParams) -> anyhow::Result<()> {
	let Some(user_path) = par.profile.find_userpath(par.side) else {
		return Ok(()); // this hand is not available
	};

	let current = par
		.current
		.and_then(|cur| ParsedOpenXrInputPath::try_from(cur).log_warn(cur).ok());

	let parent = horiz_cell(mp.layout, par.parent)?;

	let available_subpaths: Rc<[XrInputSubpathKind]> = user_path
		.paths
		.iter()
		.filter(|x| !x.kind.get_bool("Hidden").unwrap_or_default())
		.map(|x| x.kind)
		.collect();

	subpath_dropdown(
		mp,
		parent,
		par.action.clone(),
		par.side,
		available_subpaths,
		current.as_ref().map(|x| x.subpath),
	)?;

	if &**par.action == "scroll" {
		clicks_dropdown(mp, parent, par.action.clone(), par.click_type)?;
		return Ok(());
	}

	let available_components: Rc<[XrInputComponent]> = current
		.as_ref()
		.and_then(|par| user_path.find_subpath(par.subpath))
		.map(|subp| subp.components)
		.unwrap_or_default()
		.into();

	if !component_dropdown(
		mp,
		parent,
		par.action.clone(),
		par.side,
		available_components,
		current.and_then(|x| x.component),
	)? {
		return Ok(());
	}

	clicks_dropdown(mp, parent, par.action.clone(), par.click_type)?;

	if let Some(component) = current.and_then(|x| x.component)
		&& component.is_analog()
	{
		threshold_slider(mp, parent, par.action.clone(), par.side, par.threshold)?;
	}

	Ok(())
}

fn subpath_dropdown(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	side: XrInputSide,
	available: Rc<[XrInputSubpathKind]>,
	current: Option<XrInputSubpathKind>,
) -> anyhow::Result<()> {
	// left/right hand icon
	wgui_simple::create_icon(
		mp.layout,
		parent,
		Vec2::new(32.0, 32.0),
		AssetPathRef::BuiltIn(&format!("dashboard/hand_{}.svg", side.as_ref())),
	)?;

	let current_text = current
		.map(|c| c.translation())
		.unwrap_or_else(|| Translation::from_translation_key("APP_SETTINGS.OPTION.NONE"));

	create_dropdown(
		mp,
		parent,
		action,
		side,
		current_text,
		available,
		DropdownParams {
			tooltip: Some("APP_SETTINGS.BINDINGS.SUBPATH"),
			min_width: Some(100.0),
			..Default::default()
		},
	)
}

fn component_dropdown(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	side: XrInputSide,
	available: Rc<[XrInputComponent]>,
	current: Option<XrInputComponent>,
) -> anyhow::Result<bool> {
	if available.is_empty() {
		return Ok(false);
	}

	let current_text = current.map(|c| c.translation()).unwrap_or_default();

	create_dropdown(
		mp,
		parent,
		action,
		side,
		current_text,
		available,
		DropdownParams {
			text: Some("・"),
			tooltip: Some("APP_SETTINGS.BINDINGS.COMPONENT"),
			min_width: Some(100.0),
		},
	)?;
	Ok(true)
}

fn clicks_dropdown(mp: &mut MacroParams, parent: WidgetID, action: Rc<str>, current: ClickType) -> anyhow::Result<()> {
	let current_text = current.translation();
	let available = [ClickType::Any, ClickType::Double, ClickType::Triple].into();
	create_dropdown(
		mp,
		parent,
		action,
		XrInputSide::Left,
		current_text,
		available,
		DropdownParams {
			text: Some("・"),
			tooltip: Some("APP_SETTINGS.BINDINGS.CLICK.TYPE"),
			min_width: Some(100.0),
		},
	)
}

fn side_translation(side: XrInputSide) -> Translation {
	Translation::from_translation_key(match side {
		XrInputSide::Left => "APP_SETTINGS.BINDINGS.LEFT",
		XrInputSide::Right => "APP_SETTINGS.BINDINGS.RIGHT",
	})
}

#[allow(clippy::too_many_arguments)]
fn chord_hand_dropdown(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	output_side: XrInputSide,
	member_idx: usize,
	profile: &XrControllerProfile,
	current: XrInputSide,
) -> anyhow::Result<()> {
	let cells = [XrInputSide::Left, XrInputSide::Right]
		.into_iter()
		.filter(|side| profile.find_userpath(*side).is_some())
		.map(|side| {
			context_menu::Cell::Button(context_menu::Button {
				action: DropdownAction::ChordHand {
					action: action.clone(),
					side: output_side,
					new_side: side,
					member_idx,
				}
				.into_cell(),
				title: side_translation(side),
				..Default::default()
			})
		})
		.collect();

	create_dropdown_with_cells(
		mp,
		parent,
		side_translation(current),
		cells,
		DropdownParams {
			tooltip: Some("APP_SETTINGS.BINDINGS.CHORD_INPUT_HAND"),
			min_width: Some(100.0),
			..Default::default()
		},
	)?;

	Ok(())
}

#[allow(clippy::too_many_arguments)]
fn chord_subpath_dropdown(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	output_side: XrInputSide,
	member_idx: usize,
	available: Rc<[XrInputSubpathKind]>,
	current: Option<XrInputSubpathKind>,
) -> anyhow::Result<()> {
	let current_text = current
		.map(|subpath| subpath.translation())
		.unwrap_or_else(|| Translation::from_translation_key("APP_SETTINGS.OPTION.NONE"));
	let cells = available
		.iter()
		.map(|subpath| {
			context_menu::Cell::Button(context_menu::Button {
				action: DropdownAction::ChordSubpath {
					action: action.clone(),
					side: output_side,
					subpath: *subpath,
					member_idx,
				}
				.into_cell(),
				title: subpath.translation(),
				..Default::default()
			})
		})
		.collect();

	create_dropdown_with_cells(
		mp,
		parent,
		current_text,
		cells,
		DropdownParams {
			tooltip: Some("APP_SETTINGS.BINDINGS.SUBPATH"),
			min_width: Some(100.0),
			..Default::default()
		},
	)
}

#[allow(clippy::too_many_arguments)]
fn chord_component_dropdown(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	output_side: XrInputSide,
	member_idx: usize,
	available: Rc<[XrInputComponent]>,
	current: Option<XrInputComponent>,
) -> anyhow::Result<bool> {
	if available.is_empty() {
		return Ok(false);
	}

	let current_text = current.map(|component| component.translation()).unwrap_or_default();
	let cells = available
		.iter()
		.map(|component| {
			context_menu::Cell::Button(context_menu::Button {
				action: DropdownAction::ChordComp {
					action: action.clone(),
					side: output_side,
					member_idx,
					component: *component,
				}
				.into_cell(),
				title: component.translation(),
				..Default::default()
			})
		})
		.collect();

	create_dropdown_with_cells(
		mp,
		parent,
		current_text,
		cells,
		DropdownParams {
			text: Some("・"),
			tooltip: Some("APP_SETTINGS.BINDINGS.COMPONENT"),
			min_width: Some(100.0),
		},
	)?;

	Ok(true)
}

fn chord_threshold_slider(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	output_side: XrInputSide,
	member_idx: usize,
	current: Option<[f32; 2]>,
) -> anyhow::Result<()> {
	let id = mp.gen_id();

	let current = current.unwrap_or(DEFAULT_BUTTON_THRESHOLDS);

	let mut params = TemplateParams::new();
	params.insert("id", id.clone());
	params.insert("tooltip", "APP_SETTINGS.BINDINGS.THRESHOLD");
	params.insert("value", format!("{:.2}", current[0]));
	params.insert("value2", format!("{:.2}", current[1]));
	params.insert("min", "0.0");
	params.insert("max", "1.0");
	params.insert("step", "0.1");

	mp.parser_state
		.instantiate_template(mp.doc_params, "ThresholdSlider", mp.layout, parent, params)?;

	let slider = mp.parser_state.fetch_component_as::<ComponentSlider>(&id)?;
	slider.on_value_changed(Box::new({
		let tasks = mp.tasks.clone();
		move |_common, e| {
			let threshold_idx = if matches!(e.index, wgui::components::slider::ValueIndex::Primary) {
				0
			} else {
				1
			};
			tasks.push(Task::UpdateChordThreshold(
				action.clone(),
				output_side,
				member_idx,
				threshold_idx,
				e.value,
			));
		}
	}));

	Ok(())
}

fn threshold_slider(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	side: XrInputSide,
	current: Option<[f32; 2]>,
) -> anyhow::Result<()> {
	let id = mp.gen_id();

	let current = current.unwrap_or(DEFAULT_BUTTON_THRESHOLDS);

	let mut params = TemplateParams::new();
	params.insert("id", id.clone());
	params.insert("tooltip", "APP_SETTINGS.BINDINGS.THRESHOLD");
	params.insert("value", format!("{:.2}", current[0]));
	params.insert("value2", format!("{:.2}", current[1]));
	params.insert("min", "0.0");
	params.insert("max", "1.0");
	params.insert("step", "0.1");

	mp.parser_state
		.instantiate_template(mp.doc_params, "ThresholdSlider", mp.layout, parent, params)?;

	let slider = mp.parser_state.fetch_component_as::<ComponentSlider>(&id)?;
	slider.on_value_changed(Box::new({
		let tasks = mp.tasks.clone();
		move |_common, e| {
			if matches!(e.index, wgui::components::slider::ValueIndex::Primary) {
				tasks.push(Task::UpdateThreshold(action.clone(), side, 0, e.value));
			} else {
				tasks.push(Task::UpdateThreshold(action.clone(), side, 1, e.value));
			}
		}
	}));

	Ok(())
}

#[derive(Default)]
struct DropdownParams<'a> {
	text: Option<&'a str>,
	tooltip: Option<&'a str>,
	min_width: Option<f32>,
}

fn create_dropdown_with_cells(
	mp: &mut MacroParams,
	parent: WidgetID,
	current_text: Translation,
	cells: Vec<context_menu::Cell>,
	par: DropdownParams,
) -> anyhow::Result<()> {
	let id_label = mp.gen_id();
	let tasks = mp.tasks.clone();
	wgui_simple::mount_dropdown(wgui_simple::MountDropdownParams {
		layout: mp.layout,
		parser_state: mp.parser_state,
		doc_params: mp.doc_params,
		parent,
		current_text,
		cells,
		id_label: &id_label,
		on_open: Box::new(move |evt| tasks.push(Task::OpenDropdown(evt))),
		text: par.text,
		translation: None,
		tooltip: par.tooltip,
		min_width: par.min_width,
		sprite_src: None,
	})?;
	Ok(())
}

#[allow(clippy::too_many_arguments)]
fn create_dropdown<B: 'static + BindingsDropdown>(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	side: XrInputSide,
	current_text: Translation,
	available: Rc<[B]>,
	par: DropdownParams,
) -> anyhow::Result<()> {
	let mut cells = available
		.iter()
		.map(|item| {
			context_menu::Cell::Button(context_menu::Button {
				action: DropdownAction::Bindings(item.get_action(&action, side)).into_cell(),
				title: item.translation(),
				..Default::default()
			})
		})
		.collect::<Vec<_>>();

	if let Some(action) = B::get_clear(&action, side) {
		cells.insert(
			0,
			context_menu::Cell::Button(context_menu::Button {
				action: DropdownAction::Bindings(action).into_cell(),
				title: Translation::from_translation_key("APP_SETTINGS.OPTION.NONE"),
				..Default::default()
			}),
		);
	}

	create_dropdown_with_cells(mp, parent, current_text, cells, par)
}

fn apply_subpath(
	side_mut: &mut Option<OneOrMany<String>>,
	action_name: &str,
	side: &XrInputSide,
	subpath: &XrInputSubpathKind,
	profile: &XrControllerProfile,
) {
	if action_name == "scroll" {
		*side_mut = Some(OneOrMany::One(format!(
			"/user/hand/{}/input/{}",
			side.as_ref(),
			subpath.as_ref()
		)));
		return;
	}

	let Some(subpath_obj) = profile.find_userpath(*side).and_then(|p| p.find_subpath(*subpath)) else {
		return;
	};

	let comp: XrInputComponent = if let Some(first) = side_mut.as_ref().map(|x| match x {
		OneOrMany::One(x) => x.as_str(),
		OneOrMany::Many(x) => x.first().unwrap().as_str(),
	}) {
		let Ok(parsed) = ParsedOpenXrInputPath::try_from(first) else {
			return;
		};

		let parsed_compo = parsed.component.unwrap_or(*subpath_obj.components.first().unwrap());
		if subpath_obj.components.contains(&parsed_compo) {
			parsed_compo
		} else {
			*subpath_obj.components.first().unwrap()
		}
	} else {
		*subpath_obj.components.first().unwrap()
	};

	let comp_str = comp.as_ref();
	*side_mut = Some(OneOrMany::One(format!(
		"/user/hand/{}/input/{}/{}",
		side.as_ref(),
		subpath.as_ref(),
		comp_str
	)));
}

fn apply_comp(side_mut: &mut Option<OneOrMany<String>>, side: &XrInputSide, comp: &XrInputComponent) {
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

	let subpath = parsed.subpath.as_ref();

	*side_mut = Some(OneOrMany::One(format!(
		"/user/hand/{}/input/{subpath}/{}",
		side.as_ref(),
		comp.as_ref()
	)));
}

fn action_chord_mut(
	action: &mut OpenXrInputAction,
	output_side: XrInputSide,
) -> &mut Option<Vec<OpenXrInputChordMember>> {
	match output_side {
		XrInputSide::Left => &mut action.left_chord,
		XrInputSide::Right => &mut action.right_chord,
	}
}

fn action_binding_mut<'a>(action: &'a mut OpenXrInputAction, side: &XrInputSide) -> &'a mut Option<OneOrMany<String>> {
	match side {
		XrInputSide::Left => &mut action.left,
		XrInputSide::Right => &mut action.right,
	}
}

fn apply_click_count(action: &mut OpenXrInputAction, click_type: &ClickType) {
	match click_type {
		ClickType::Any => {
			action.triple_click = None;
			action.double_click = None;
		}
		ClickType::Double => {
			action.triple_click = None;
			action.double_click = Some(true);
		}
		ClickType::Triple => {
			action.triple_click = Some(true);
			action.double_click = None;
		}
	}
}

fn default_chord_member(profile: &XrControllerProfile, preferred_side: XrInputSide) -> Option<OpenXrInputChordMember> {
	let other_side = match preferred_side {
		XrInputSide::Left => XrInputSide::Right,
		XrInputSide::Right => XrInputSide::Left,
	};

	for side in [preferred_side, other_side] {
		let Some(user_path) = profile.find_userpath(side) else {
			continue;
		};
		let Some(subpath) = user_path
			.paths
			.iter()
			.find(|subpath| !subpath.kind.get_bool("Hidden").unwrap_or_default() && !subpath.components.is_empty())
		else {
			continue;
		};
		let component = *subpath.components.first()?;
		return Some(OpenXrInputChordMember::Path(format!(
			"/user/hand/{}/input/{}/{}",
			side.as_ref(),
			subpath.kind.as_ref(),
			component.as_ref()
		)));
	}

	None
}

fn set_chord_member_path(member: &mut OpenXrInputChordMember, path: String, component: &XrInputComponent) {
	let threshold = if component.is_analog() {
		member.threshold()
	} else {
		None
	};

	*member = if threshold.is_some() {
		OpenXrInputChordMember::Detailed { path, threshold }
	} else {
		OpenXrInputChordMember::Path(path)
	};
}

fn set_chord_member_threshold(member: &mut OpenXrInputChordMember, threshold: [f32; 2]) {
	*member = OpenXrInputChordMember::Detailed {
		path: member.path().to_owned(),
		threshold: Some(threshold),
	};
}

fn apply_chord_hand(member: &mut OpenXrInputChordMember, side: &XrInputSide, profile: &XrControllerProfile) {
	let parsed = ParsedOpenXrInputPath::try_from(member.path()).ok();
	let Some(user_path) = profile.find_userpath(*side) else {
		return;
	};

	let subpath = parsed
		.and_then(|parsed| user_path.find_subpath(parsed.subpath))
		.filter(|subpath| !subpath.kind.get_bool("Hidden").unwrap_or_default() && !subpath.components.is_empty())
		.or_else(|| {
			user_path
				.paths
				.iter()
				.find(|subpath| !subpath.kind.get_bool("Hidden").unwrap_or_default() && !subpath.components.is_empty())
		});
	let Some(subpath) = subpath else {
		return;
	};

	let component = parsed
		.and_then(|parsed| parsed.component)
		.filter(|component| subpath.components.contains(component))
		.unwrap_or(subpath.components[0]);
	let path = format!(
		"/user/hand/{}/input/{}/{}",
		side.as_ref(),
		subpath.kind.as_ref(),
		component.as_ref()
	);
	set_chord_member_path(member, path, &component);
}

fn apply_chord_subpath(
	member: &mut OpenXrInputChordMember,
	subpath: &XrInputSubpathKind,
	profile: &XrControllerProfile,
) {
	let Ok(parsed) = ParsedOpenXrInputPath::try_from(member.path()) else {
		return;
	};
	let Some(subpath_obj) = profile
		.find_userpath(parsed.side)
		.and_then(|user_path| user_path.find_subpath(*subpath))
	else {
		return;
	};
	let Some(first_component) = subpath_obj.components.first().copied() else {
		return;
	};

	let component = if let Some(comp) = parsed.component {
		if subpath_obj.components.contains(&comp) {
			comp
		} else {
			first_component
		}
	} else {
		first_component
	};
	let path = format!(
		"/user/hand/{}/input/{}/{}",
		parsed.side.as_ref(),
		subpath.as_ref(),
		component.as_ref()
	);
	set_chord_member_path(member, path, &component);
}

fn apply_chord_component(member: &mut OpenXrInputChordMember, component: &XrInputComponent) {
	let Ok(parsed) = ParsedOpenXrInputPath::try_from(member.path()) else {
		return;
	};
	let path = format!(
		"/user/hand/{}/input/{}/{}",
		parsed.side.as_ref(),
		parsed.subpath.as_ref(),
		component.as_ref()
	);
	set_chord_member_path(member, path, component);
}
