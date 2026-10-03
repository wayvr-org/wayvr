use glam::Vec2;
use std::rc::Rc;
use strum::EnumProperty;
use wgui::{
	assets::AssetPathRef,
	components::{
		button::{ButtonClickEvent, ComponentButton},
		slider::ComponentSlider,
	},
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	log::LogErr,
	parser::{Fetchable, ParseDocumentParams, ParserState, TemplateParams},
	task::Tasks,
	widget::label::WidgetLabel,
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
		openxr_bindings::{BindingsDropdown, ClickType, ParsedOpenXrInputPath},
		popup_manager::{MountPopupOnceParams, MountPopupOnceParamsExtra, PopupHolder, PopupPadding},
		wgui_simple,
	},
	views::{ViewTrait, ViewUpdateParams},
};

#[derive(Clone)]
enum Task {
	Save,
	Cancel,
	OpenContextMenu(context_menu::Position, Vec<context_menu::Cell>),
	UpdateThreshold(Rc<str>, XrInputSide, usize, f32),
	AddChordMember(Rc<str>, XrInputSide),
	RemoveChordMember(Rc<str>, XrInputSide, usize),
	UpdateChordThreshold(Rc<str>, XrInputSide, usize, usize, f32),
}

pub struct Params<'a> {
	pub globals: WguiGlobals,
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub controller_profile: &'static XrControllerProfile,
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
				Task::OpenContextMenu(position, cells) => {
					self.context_menu.open(context_menu::OpenParams {
						on_custom_attribs: None,
						position,
						blueprint: context_menu::Blueprint::Cells(cells),
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
		if let TickResult::Action(name) = self.context_menu.tick(par.layout, &mut self.parser_state)? {
			let parts = name.split(';').collect::<Vec<_>>();
			let cur_profile = &mut self.profiles[self.cur_profile_idx];

			match parts.as_slice() {
				[action, action_name, side, value] => {
					let action_mut = get_action_mut(cur_profile, action_name);

					match *action {
						"clear" => {
							*action_binding_mut(action_mut, side) = None;
						}
						"subpath" => {
							apply_subpath(
								action_binding_mut(action_mut, side),
								side,
								value,
								self.controller_profile,
							);
						}
						"comp" => {
							apply_comp(action_binding_mut(action_mut, side), side, value);
						}
						"click" => apply_click_count(action_mut, value),
						_ => log::warn!("Unknown action {action}"),
					}
					self.refresh(par.layout)?;
				}
				[kind, action_name, output_side, member_idx, value] => {
					let (Ok(output_side), Ok(member_idx)) = (XrInputSide::try_from(*output_side), member_idx.parse::<usize>())
					else {
						return Ok(());
					};
					let action_mut = get_action_mut(cur_profile, action_name);
					let Some(member) = action_chord_mut(action_mut, output_side)
						.as_mut()
						.and_then(|members| members.get_mut(member_idx))
					else {
						return Ok(());
					};

					match *kind {
						"chord_hand" => {
							if let Ok(side) = XrInputSide::try_from(*value) {
								apply_chord_hand(member, side, self.controller_profile);
							}
						}
						"chord_subpath" => {
							if let Ok(subpath) = XrInputSubpathKind::try_from(*value) {
								apply_chord_subpath(member, subpath, self.controller_profile);
							}
						}
						"chord_comp" => {
							if let Ok(component) = XrInputComponent::try_from(*value) {
								apply_chord_component(member, component);
							}
						}
						_ => log::warn!("Unknown chord action {kind}"),
					}
					self.refresh(par.layout)?;
				}
				_ => log::warn!("Malformed bindings context action: {name}"),
			}
		}

		Ok(())
	}
}

impl View {
	pub fn new(params: Params) -> anyhow::Result<Self> {
		let doc_params = &ParseDocumentParams {
			globals: params.globals.clone(),
			path: AssetPathRef::BuiltIn("gui/view/bindings.xml"),
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
			doc_params: &ParseDocumentParams {
				globals: self.globals.clone(),
				path: AssetPathRef::BuiltIn("gui/view/bindings.xml"),
				extra: Default::default(),
			},
			layout,
			tasks: self.tasks.clone(),
			idx: 0,
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

pub fn mount_popup(
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
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
	profile: &XrControllerProfile,
	current: &mut OpenXrInputAction,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params = TemplateParams::new();
	params.insert("id", &id);
	params.insert_str(
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
		parsed.as_ref().map(|x| x.component),
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
	if parsed.as_ref().is_some_and(|x| x.component.is_analog()) {
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

	let available_components: Rc<[XrInputComponent]> = current
		.as_ref()
		.and_then(|par| user_path.find_subpath(par.subpath))
		.map(|subp| subp.components)
		.unwrap_or_default()
		.into();

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

	if !component_dropdown(
		mp,
		parent,
		par.action.clone(),
		par.side,
		available_components,
		current.as_ref().map(|x| x.component),
	)? {
		return Ok(());
	}

	clicks_dropdown(mp, parent, par.action.clone(), par.click_type)?;

	if let Some(component) = current.as_ref().map(|x| x.component)
		&& component.is_analog()
		&& &**par.action != "scroll"
	// hax
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
	let mut params = TemplateParams::new();
	params.insert("tooltip", "APP_SETTINGS.BINDINGS.SUBPATH");
	params.insert("min_width", "100");

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

	create_dropdown(mp, parent, params, action, side, current_text, available)?;

	Ok(())
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

	let mut params = TemplateParams::new();
	params.insert("text", "・");
	params.insert("tooltip", "APP_SETTINGS.BINDINGS.COMPONENT");
	params.insert("min_width", "100");

	let current_text = current
		.map(|c| c.translation())
		.unwrap_or_else(|| Translation::from_raw_text_rc(Default::default()));

	create_dropdown(mp, parent, params, action, side, current_text, available)?;

	Ok(true)
}

fn clicks_dropdown(mp: &mut MacroParams, parent: WidgetID, action: Rc<str>, current: ClickType) -> anyhow::Result<()> {
	let mut params = TemplateParams::new();
	params.insert("text", "・");
	params.insert("tooltip", "APP_SETTINGS.BINDINGS.CLICK.TYPE");
	params.insert("min_width", "100");

	let current_text = current.translation();
	let available = [ClickType::Any, ClickType::Double, ClickType::Triple].into();
	create_dropdown(mp, parent, params, action, XrInputSide::Left, current_text, available)?;

	Ok(())
}

fn side_translation(side: XrInputSide) -> Translation {
	Translation::from_translation_key(match side {
		XrInputSide::Left => "APP_SETTINGS.BINDINGS.LEFT",
		XrInputSide::Right => "APP_SETTINGS.BINDINGS.RIGHT",
	})
}

fn chord_hand_dropdown(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	output_side: XrInputSide,
	member_idx: usize,
	profile: &XrControllerProfile,
	current: XrInputSide,
) -> anyhow::Result<()> {
	let mut params = TemplateParams::new();
	params.insert("tooltip", "APP_SETTINGS.BINDINGS.CHORD_INPUT_HAND");
	params.insert("min_width", "100");

	let cells = [XrInputSide::Left, XrInputSide::Right]
		.into_iter()
		.filter(|side| profile.find_userpath(*side).is_some())
		.map(|side| context_menu::Cell {
			action_name: Some(
				format!(
					"chord_hand;{};{};{};{}",
					action,
					output_side.as_ref(),
					member_idx,
					side.as_ref()
				)
				.into(),
			),
			title: side_translation(side),
			tooltip: None,
			attribs: vec![],
		})
		.collect();

	create_dropdown_with_cells(mp, parent, params, side_translation(current), cells)
}

fn chord_subpath_dropdown(
	mp: &mut MacroParams,
	parent: WidgetID,
	action: Rc<str>,
	output_side: XrInputSide,
	member_idx: usize,
	available: Rc<[XrInputSubpathKind]>,
	current: Option<XrInputSubpathKind>,
) -> anyhow::Result<()> {
	let mut params = TemplateParams::new();
	params.insert("tooltip", "APP_SETTINGS.BINDINGS.SUBPATH");
	params.insert("min_width", "100");

	let current_text = current
		.map(|subpath| subpath.translation())
		.unwrap_or_else(|| Translation::from_translation_key("APP_SETTINGS.OPTION.NONE"));
	let cells = available
		.iter()
		.map(|subpath| context_menu::Cell {
			action_name: Some(
				format!(
					"chord_subpath;{};{};{};{}",
					action,
					output_side.as_ref(),
					member_idx,
					subpath.as_ref()
				)
				.into(),
			),
			title: subpath.translation(),
			tooltip: None,
			attribs: vec![],
		})
		.collect();

	create_dropdown_with_cells(mp, parent, params, current_text, cells)
}

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

	let mut params = TemplateParams::new();
	params.insert("text", "・");
	params.insert("tooltip", "APP_SETTINGS.BINDINGS.COMPONENT");
	params.insert("min_width", "100");

	let current_text = current
		.map(|component| component.translation())
		.unwrap_or_else(|| Translation::from_raw_text_rc(Default::default()));
	let cells = available
		.iter()
		.map(|component| context_menu::Cell {
			action_name: Some(
				format!(
					"chord_comp;{};{};{};{}",
					action,
					output_side.as_ref(),
					member_idx,
					component.as_ref()
				)
				.into(),
			),
			title: component.translation(),
			tooltip: None,
			attribs: vec![],
		})
		.collect();

	create_dropdown_with_cells(mp, parent, params, current_text, cells)?;
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
	let id = mp.idx.to_string();
	mp.idx += 1;

	let current = current.unwrap_or(DEFAULT_BUTTON_THRESHOLDS);

	let mut params = TemplateParams::new();
	params.insert("id", &id);
	params.insert("tooltip", "APP_SETTINGS.BINDINGS.THRESHOLD");
	params.insert_str("value", format!("{:.2}", current[0]));
	params.insert_str("value2", format!("{:.2}", current[1]));
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
	let id = mp.idx.to_string();
	mp.idx += 1;

	let current = current.unwrap_or(DEFAULT_BUTTON_THRESHOLDS);

	let mut params = TemplateParams::new();
	params.insert("id", &id);
	params.insert("tooltip", "APP_SETTINGS.BINDINGS.THRESHOLD");
	params.insert_str("value", format!("{:.2}", current[0]));
	params.insert_str("value2", format!("{:.2}", current[1]));
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

fn create_dropdown<B: 'static + BindingsDropdown>(
	mp: &mut MacroParams,
	parent: WidgetID,
	params: TemplateParams,
	action: Rc<str>,
	side: XrInputSide,
	current_text: Translation,
	available: Rc<[B]>,
) -> anyhow::Result<()> {
	let mut cells = available
		.iter()
		.map(|item| context_menu::Cell {
			action_name: Some(item.action_str(&action, side)),
			title: item.translation(),
			tooltip: None,
			attribs: vec![],
		})
		.collect::<Vec<_>>();

	if let Some(action_str) = B::clear_str(&action, side) {
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

	create_dropdown_with_cells(mp, parent, params, current_text, cells)
}

fn create_dropdown_with_cells(
	mp: &mut MacroParams,
	parent: WidgetID,
	mut params: TemplateParams,
	current_text: Translation,
	cells: Vec<context_menu::Cell>,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;
	params.insert("id", &id);

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
		let cells = cells.clone();
		move |_common, e: ButtonClickEvent| {
			tasks.push(Task::OpenContextMenu(e.mouse_pos_absolute.into(), cells.clone()));
			Ok(())
		}
	}));

	Ok(())
}

fn apply_subpath(
	side_mut: &mut Option<OneOrMany<String>>,
	side_str: &str,
	subpath_str: &str,
	profile: &XrControllerProfile,
) {
	let (Ok(side), Ok(subpath)) = (
		XrInputSide::try_from(side_str),
		XrInputSubpathKind::try_from(subpath_str),
	) else {
		return;
	};
	let Some(subpath_obj) = profile.find_userpath(side).and_then(|p| p.find_subpath(subpath)) else {
		return;
	};

	let comp: XrInputComponent = if let Some(first) = side_mut.as_ref().map(|x| match x {
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

	let comp_str = comp.as_ref();
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

	let subpath = parsed.subpath.as_ref();

	*side_mut = Some(OneOrMany::One(format!("/user/hand/{side}/input/{subpath}/{comp}")));
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

fn action_binding_mut<'a>(action: &'a mut OpenXrInputAction, side: &str) -> &'a mut Option<OneOrMany<String>> {
	if side == "right" {
		&mut action.right
	} else {
		&mut action.left
	}
}

fn apply_click_count(action: &mut OpenXrInputAction, value: &str) {
	match value {
		"triple" => {
			action.triple_click = Some(true);
			action.double_click = None;
		}
		"double" => {
			action.triple_click = None;
			action.double_click = Some(true);
		}
		_ => {
			action.triple_click = None;
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

fn set_chord_member_path(member: &mut OpenXrInputChordMember, path: String, component: XrInputComponent) {
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

fn apply_chord_hand(member: &mut OpenXrInputChordMember, side: XrInputSide, profile: &XrControllerProfile) {
	let parsed = ParsedOpenXrInputPath::try_from(member.path()).ok();
	let Some(user_path) = profile.find_userpath(side) else {
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
		.map(|parsed| parsed.component)
		.filter(|component| subpath.components.contains(component))
		.unwrap_or(subpath.components[0]);
	let path = format!(
		"/user/hand/{}/input/{}/{}",
		side.as_ref(),
		subpath.kind.as_ref(),
		component.as_ref()
	);
	set_chord_member_path(member, path, component);
}

fn apply_chord_subpath(
	member: &mut OpenXrInputChordMember,
	subpath: XrInputSubpathKind,
	profile: &XrControllerProfile,
) {
	let Ok(parsed) = ParsedOpenXrInputPath::try_from(member.path()) else {
		return;
	};
	let Some(subpath_obj) = profile
		.find_userpath(parsed.side)
		.and_then(|user_path| user_path.find_subpath(subpath))
	else {
		return;
	};
	let Some(first_component) = subpath_obj.components.first().copied() else {
		return;
	};

	let component = if subpath_obj.components.contains(&parsed.component) {
		parsed.component
	} else {
		first_component
	};
	let path = format!(
		"/user/hand/{}/input/{}/{}",
		parsed.side.as_ref(),
		subpath.as_ref(),
		component.as_ref()
	);
	set_chord_member_path(member, path, component);
}

fn apply_chord_component(member: &mut OpenXrInputChordMember, component: XrInputComponent) {
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
