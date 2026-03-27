use std::{
	cell::{RefCell, RefMut},
	collections::{HashMap, HashSet},
	io::Write,
	rc::{Rc, Weak},
};

use crate::{
	animation::Animations,
	components::{self, Component, ComponentWeak, FocusChangeData, RefreshData},
	drawing::{
		self, ANSI_BOLD_CODE, ANSI_RESET_CODE, Boundary, PushScissorStackResult, push_scissor_stack, push_transform_stack,
	},
	event::{self, CallbackDataCommon, EventAlterables},
	globals::WguiGlobals,
	sound::WguiSoundType,
	task::Tasks,
	theme::WguiTheme,
	widget::{self, EventParams, EventResult, WidgetObj, WidgetState, WidgetStateFlags, div::WidgetDiv},
};

use anyhow::Context;
use glam::{Vec2, vec2};
use slotmap::{SecondaryMap, SlotMap, new_key_type};
use taffy::{NodeId, TaffyTree, TraversePartialTree};

new_key_type! {
	pub struct WidgetID;
}

#[derive(Clone)]
pub struct Widget(Rc<RefCell<WidgetState>>);
pub struct WeakWidget(Weak<RefCell<WidgetState>>);

impl Widget {
	pub fn new(widget_state: WidgetState) -> Self {
		Self(Rc::new(RefCell::new(widget_state)))
	}

	pub fn get_as<T: 'static>(&self) -> Option<RefMut<'_, T>> {
		RefMut::filter_map(self.0.borrow_mut(), |w| w.obj.get_as_mut::<T>()).ok()
	}

	pub fn cast<T: 'static>(&self) -> anyhow::Result<RefMut<'_, T>> {
		self.get_as().context("Widget cast failed")
	}

	pub fn downgrade(&self) -> WeakWidget {
		WeakWidget(Rc::downgrade(&self.0))
	}

	pub fn state(&self) -> RefMut<'_, WidgetState> {
		self.0.borrow_mut()
	}
}

impl WeakWidget {
	pub fn upgrade(&self) -> Option<Widget> {
		self.0.upgrade().map(Widget)
	}
}

pub struct WidgetMap(SlotMap<WidgetID, Widget>);
pub type WidgetNodeMap = SecondaryMap<WidgetID, taffy::NodeId>;

#[derive(Clone)]
pub struct WidgetPair {
	pub id: WidgetID,
	pub widget: Widget,
}

impl WidgetMap {
	fn new() -> Self {
		Self(SlotMap::<WidgetID, Widget>::with_key())
	}

	pub fn get_as<T: 'static>(&self, handle: WidgetID) -> Option<RefMut<'_, T>> {
		self.0.get(handle)?.get_as::<T>()
	}

	pub fn cast_as<T: 'static>(&self, handle: WidgetID) -> anyhow::Result<RefMut<'_, T>> {
		self.get_as(handle).context("Widget cast failed")
	}

	pub fn get(&self, handle: WidgetID) -> Option<&Widget> {
		self.0.get(handle)
	}

	pub fn insert(&mut self, obj: Widget) -> WidgetID {
		self
			.0
			.try_insert_with_key::<_, ()>(|widget_id| {
				obj.state().obj.set_id(widget_id);
				Ok(obj)
			})
			.unwrap()
	}

	pub fn remove_single(&mut self, handle: WidgetID) {
		self.0.remove(handle);
	}

	// cast to specific widget type, does nothing if widget ID is expired or the type is wrong
	pub fn call<WIDGET, FUNC>(&self, widget_id: WidgetID, func: FUNC)
	where
		WIDGET: WidgetObj,
		FUNC: FnOnce(&mut WIDGET),
	{
		let Some(widget) = self.get(widget_id) else {
			debug_assert!(false);
			return;
		};

		if let Some(mut casted) = widget.get_as::<WIDGET>() {
			func(&mut casted);
		}
	}
}

pub struct LayoutState {
	pub globals: WguiGlobals,
	pub theme: Rc<WguiTheme>,
	pub widgets: WidgetMap,
	pub nodes: WidgetNodeMap,
	pub tree: taffy::tree::TaffyTree<WidgetID>,
}

pub struct ModifyLayoutStateData<'a> {
	pub layout: &'a mut Layout,
}

pub struct LayoutUpdateParams {
	pub size: Vec2, // logical resolution
	pub timestep_alpha: f32,
}

pub struct LayoutUpdateResult {
	pub sounds_to_play: Vec<WguiSoundType>,
}

pub type LayoutModifyStateFunc = Box<dyn FnOnce(ModifyLayoutStateData) -> anyhow::Result<()>>;
pub type LayoutDispatchFunc = Box<dyn FnOnce(&mut CallbackDataCommon) -> anyhow::Result<()>>;

pub enum LayoutTask {
	RemoveWidget(WidgetID),
	SetWidgetStyle(WidgetID, event::StyleSetRequest),
	ModifyLayoutState(LayoutModifyStateFunc),
	PlaySound(WguiSoundType),
	Dispatch(LayoutDispatchFunc),
	SetFocus(ComponentWeak),
	Unfocus,
}

pub type LayoutTasks = Tasks<LayoutTask>;

pub struct Layout {
	pub state: LayoutState,

	pub tasks: LayoutTasks,

	components_to_refresh_once: HashSet<Component>,
	registered_components_to_refresh: HashMap<taffy::NodeId, ComponentWeak>,
	sounds_to_play_once: Vec<WguiSoundType>,
	focused_component: Option<ComponentWeak>,

	pub widgets_to_tick: Vec<WidgetID>,

	// *Main root*
	// contains content_root_widget and topmost widgets
	pub tree_root_widget: WidgetID,
	pub tree_root_node: taffy::NodeId,

	// *Main topmost widget*
	// main topmost widget, always present, parent of `tree_root_widget`
	pub content_root_widget: WidgetID,
	pub content_root_node: taffy::NodeId,

	pub prev_size: Vec2,
	pub content_size: Vec2,

	pub needs_redraw: bool,
	pub haptics_triggered: bool,

	pub animations: Animations,
}

#[derive(Default)]
pub struct LayoutParams {
	pub resize_to_parent: bool,
	pub theme: Rc<WguiTheme>,
}

fn add_child_internal(
	tree: &mut taffy::TaffyTree<WidgetID>,
	widgets: &mut WidgetMap,
	nodes: &mut WidgetNodeMap,
	parent_node: Option<taffy::NodeId>,
	widget_state: WidgetState,
	style: taffy::Style,
) -> anyhow::Result<(WidgetPair, taffy::NodeId)> {
	let new_widget = Widget::new(widget_state);

	let child_id = widgets.insert(new_widget.clone());
	let child_node = tree.new_leaf_with_context(style, child_id)?;

	if let Some(parent_node) = parent_node {
		tree.add_child(parent_node, child_node)?;
	}

	nodes.insert(child_id, child_node);

	Ok((
		WidgetPair {
			id: child_id,
			widget: new_widget,
		},
		child_node,
	))
}

pub struct LayoutCommon<'a> {
	alterables: EventAlterables,
	pub layout: &'a mut Layout,
}

impl LayoutCommon<'_> {
	pub const fn common(&mut self) -> CallbackDataCommon<'_> {
		CallbackDataCommon {
			alterables: &mut self.alterables,
			state: &self.layout.state,
		}
	}

	pub fn finish(self) -> anyhow::Result<()> {
		self.layout.process_alterables(self.alterables)?;
		Ok(())
	}
}

impl Layout {
	// helper function
	pub fn start_common(&mut self) -> LayoutCommon<'_> {
		LayoutCommon {
			alterables: EventAlterables::default(),
			layout: self,
		}
	}

	pub fn add_topmost_child(
		&mut self,
		widget: WidgetState,
		style: taffy::Style,
	) -> anyhow::Result<(WidgetPair, taffy::NodeId)> {
		self.mark_redraw();
		add_child_internal(
			&mut self.state.tree,
			&mut self.state.widgets,
			&mut self.state.nodes,
			Some(self.tree_root_node),
			widget,
			style,
		)
	}

	pub fn add_child(
		&mut self,
		parent_widget_id: WidgetID,
		widget: WidgetState,
		style: taffy::Style,
	) -> anyhow::Result<(WidgetPair, taffy::NodeId)> {
		let parent_node = *self.state.nodes.get(parent_widget_id).unwrap();

		self.mark_redraw();

		add_child_internal(
			&mut self.state.tree,
			&mut self.state.widgets,
			&mut self.state.nodes,
			Some(parent_node),
			widget,
			style,
		)
	}

	pub fn get_parent(&self, widget_id: WidgetID) -> Option<(WidgetID, taffy::NodeId)> {
		let node_id = self.state.nodes.get(widget_id)?;

		self.state.tree.parent(*node_id).map(|parent_id| {
			let parent_widget_id = self.state.tree.get_node_context(parent_id).unwrap();
			(*parent_widget_id, parent_id)
		})
	}

	fn collect_children_ids_recursive(&self, widget_id: WidgetID, out: &mut Vec<(WidgetID, taffy::NodeId)>) {
		let Some(node_id) = self.state.nodes.get(widget_id) else {
			return;
		};

		for child_id in self.state.tree.child_ids(*node_id) {
			let child_widget_id = self.state.tree.get_node_context(child_id).unwrap();
			out.push((*child_widget_id, child_id));
			self.collect_children_ids_recursive(*child_widget_id, out);
		}
	}

	fn remove_widget_single(&mut self, widget_id: WidgetID, node_id: Option<taffy::NodeId>) {
		self.state.widgets.remove_single(widget_id);
		self.state.nodes.remove(widget_id);
		if let Some(node_id) = node_id {
			self.registered_components_to_refresh.remove(&node_id);
			let _ = self.state.tree.remove(node_id);
		}
	}

	// removes all children of a specific widget
	pub fn remove_children(&mut self, widget_id: WidgetID) {
		let mut ids = Vec::new();
		self.collect_children_ids_recursive(widget_id, &mut ids);

		if !ids.is_empty() {
			self.mark_redraw();
		}

		for (widget_id, node_id) in ids {
			self.remove_widget_single(widget_id, Some(node_id));
		}
	}

	// remove widget and its children, recursively
	pub fn remove_widget(&mut self, widget_id: WidgetID) {
		self.remove_children(widget_id);
		let node_id = self.state.nodes.get(widget_id);
		self.remove_widget_single(widget_id, node_id.copied());
		self.mark_redraw();
	}

	pub const fn mark_redraw(&mut self) {
		self.needs_redraw = true;
	}

	fn process_pending_components(&mut self, alterables: &mut EventAlterables) {
		for comp in &self.components_to_refresh_once {
			let mut common = CallbackDataCommon {
				state: &self.state,
				alterables,
			};

			comp.0.refresh(&mut RefreshData { common: &mut common });
		}
		self.components_to_refresh_once.clear();
	}

	fn process_pending_widget_ticks(&mut self, alterables: &mut EventAlterables) {
		for widget_id in &self.widgets_to_tick {
			let Some(widget) = self.state.widgets.get(*widget_id) else {
				continue;
			};

			widget.state().tick(*widget_id, alterables);
		}
		self.widgets_to_tick.clear();
	}

	// call ComponentTrait::refresh() *once* in the next tick
	pub fn defer_component_refresh(&mut self, component: Component) {
		self.components_to_refresh_once.insert(component);
	}

	// call ComponentTrait::refresh() *every time time* the layout is dirty
	pub fn register_component_refresh(&mut self, component: &Component) {
		let widget_id = component.0.base().get_id();
		let Some(node_id) = self.state.nodes.get(widget_id) else {
			debug_assert!(false);
			return;
		};

		self.registered_components_to_refresh.insert(*node_id, component.weak());
	}

	/// Convenience function to avoid repeated `WidgetID` → `WidgetState` look-ups.
	pub fn add_event_listener<U1: 'static, U2: 'static>(
		&self,
		widget_id: WidgetID,
		kind: event::EventListenerKind,
		callback: event::EventCallback<U1, U2>,
	) -> Option<event::EventListenerID> {
		Some(
			self
				.state
				.widgets
				.get(widget_id)?
				.state()
				.event_listeners
				.register(kind, callback),
		)
	}

	fn push_event_children<'a, U1: 'static, U2: 'static>(
		&self,
		parent_node_id: taffy::NodeId,
		event: &event::Event,
		event_result: &mut EventResult,
		alterables: &mut EventAlterables,
		user_data: &mut (&'a mut U1, &'a mut U2),
	) -> anyhow::Result<()> {
		let count = self.state.tree.child_count(parent_node_id);

		let mut iter = |idx: usize| -> anyhow::Result<bool> {
			let child_id = self.state.tree.get_child_id(parent_node_id, idx);
			self.push_event_widget(child_id, event, event_result, alterables, user_data)?;
			Ok(!event_result.can_propagate())
		};

		// reverse iter
		for idx in (0..count).rev() {
			if iter(idx)? {
				break;
			}
		}

		Ok(())
	}

	fn push_event_widget<'a, U1: 'static, U2: 'static>(
		&self,
		node_id: taffy::NodeId,
		event: &event::Event,
		event_result: &mut EventResult,
		alterables: &mut EventAlterables,
		user_data: &mut (&'a mut U1, &'a mut U2),
	) -> anyhow::Result<()> {
		let l = self.state.tree.layout(node_id)?;
		let Some(widget_id) = self.state.tree.get_node_context(node_id).copied() else {
			anyhow::bail!("invalid widget ID");
		};

		let style = self.state.tree.style(node_id)?;

		if style.display == taffy::Display::None {
			return Ok(());
		}

		let Some(widget) = self.state.widgets.get(widget_id) else {
			debug_assert!(false);
			anyhow::bail!("invalid widget");
		};

		let mut widget = widget.0.borrow_mut();
		let (scroll_shift, info) = match widget::get_scrollbar_info(l) {
			Some(info) => (widget.get_scroll_shift_raw(&info, l), Some(info)),
			None => (Vec2::default(), None),
		};

		// see drawing.rs draw_widget too
		push_transform_stack(&mut alterables.transform_stack, l, scroll_shift, &widget);

		widget.data.cached_absolute_boundary = drawing::Boundary::construct_absolute(&alterables.transform_stack);

		let scissor_result = push_scissor_stack(
			&mut alterables.transform_stack,
			&mut alterables.scissor_stack,
			scroll_shift,
			&info,
			style,
		);

		if scissor_result
			.as_ref()
			.is_none_or(PushScissorStackResult::should_display)
		{
			// check children first
			self.push_event_children(node_id, event, event_result, alterables, user_data)?;

			if event_result.can_propagate() {
				let mut params = EventParams {
					state: &self.state,
					layout: l,
					alterables,
					node_id,
					style,
				};

				widget.process_event(widget_id, node_id, event, event_result, user_data, &mut params)?;
			}
		}

		if scissor_result.is_some() {
			alterables.scissor_stack.pop();
		}
		alterables.transform_stack.pop();

		Ok(())
	}

	pub const fn check_toggle_needs_redraw(&mut self) -> bool {
		if self.needs_redraw {
			self.needs_redraw = false;
			true
		} else {
			false
		}
	}

	pub const fn check_toggle_haptics_triggered(&mut self) -> bool {
		if self.haptics_triggered {
			self.haptics_triggered = false;
			true
		} else {
			false
		}
	}

	pub fn push_event<U1: 'static, U2: 'static>(
		&mut self,
		event: &event::Event,
		user1: &mut U1,
		user2: &mut U2,
	) -> anyhow::Result<EventResult> {
		let mut alterables = EventAlterables::default();
		let mut event_result = EventResult::NoHit;
		self.push_event_widget(
			self.tree_root_node,
			event,
			&mut event_result,
			&mut alterables,
			&mut (user1, user2),
		)?;
		self.process_alterables(alterables)?;
		Ok(event_result)
	}

	pub fn new(globals: WguiGlobals, params: LayoutParams) -> anyhow::Result<Self> {
		let mut state = LayoutState {
			tree: TaffyTree::new(),
			widgets: WidgetMap::new(),
			nodes: WidgetNodeMap::default(),
			globals,
			theme: params.theme,
		};

		let size = if params.resize_to_parent {
			taffy::Size::percent(1.0)
		} else {
			taffy::Size::auto()
		};

		let (tree_root_widget, tree_root_node) = add_child_internal(
			&mut state.tree,
			&mut state.widgets,
			&mut state.nodes,
			None, // no parent
			WidgetState {
				flags: WidgetStateFlags {
					interactable: false,
					..Default::default()
				},
				..WidgetDiv::create()
			},
			taffy::Style {
				size,
				..Default::default()
			},
		)?;

		let (content_root_widget, content_root_node) = add_child_internal(
			&mut state.tree,
			&mut state.widgets,
			&mut state.nodes,
			Some(tree_root_node),
			WidgetState {
				flags: WidgetStateFlags {
					interactable: false,
					..Default::default()
				},
				..WidgetDiv::create()
			},
			taffy::Style {
				size,
				..Default::default()
			},
		)?;

		Ok(Self {
			state,
			prev_size: Vec2::default(),
			content_size: Vec2::default(),
			tree_root_node,
			tree_root_widget: tree_root_widget.id,
			content_root_node,
			content_root_widget: content_root_widget.id,
			needs_redraw: true,
			haptics_triggered: false,
			animations: Animations::default(),
			components_to_refresh_once: HashSet::new(),
			registered_components_to_refresh: HashMap::new(),
			widgets_to_tick: Vec::new(),
			tasks: LayoutTasks::new(),
			sounds_to_play_once: Vec::new(),
			focused_component: None,
		})
	}

	fn refresh_recursively(&self, node_id: taffy::NodeId, to_refresh: &mut Vec<Component>) {
		// skip refreshing clean nodes
		if !self.state.tree.dirty(node_id).unwrap() {
			return;
		}

		if let Some(component) = self.registered_components_to_refresh.get(&node_id)
			&& let Some(c) = component.upgrade()
		{
			to_refresh.push(Component(c));
		}

		for child_id in self.state.tree.child_ids(node_id) {
			self.refresh_recursively(child_id, to_refresh);
		}
	}

	fn try_recompute_layout(&mut self, size: Vec2) -> anyhow::Result<()> {
		if !self.state.tree.dirty(self.tree_root_node)? && self.prev_size == size {
			// Nothing to do
			return Ok(());
		}

		log::trace!("re-computing layout, size {}x{}", size.x, size.y);
		self.mark_redraw();
		self.prev_size = size;

		let mut to_refresh = Vec::<Component>::new();
		self.refresh_recursively(self.tree_root_node, &mut to_refresh);

		if !to_refresh.is_empty() {
			log::trace!("refreshing {} registered components", to_refresh.len());
			for c in &to_refresh {
				self.components_to_refresh_once.insert(c.clone());
			}
		}

		let globals = self.state.globals.get();

		self.state.tree.compute_layout_with_measure(
			self.tree_root_node,
			taffy::Size {
				width: taffy::AvailableSpace::Definite(size.x),
				height: taffy::AvailableSpace::Definite(size.y),
			},
			|known_dimensions, available_space, _node_id, node_context, _style| {
				if let taffy::Size {
					width: Some(width),
					height: Some(height),
				} = known_dimensions
				{
					return taffy::Size { width, height };
				}

				match node_context {
					None => taffy::Size::ZERO,
					Some(h) => {
						if let Some(w) = self.state.widgets.get(*h) {
							w.0
								.borrow_mut()
								.obj
								.measure(&globals, known_dimensions, available_space)
						} else {
							taffy::Size::ZERO
						}
					}
				}
			},
		)?;
		let root_size = self.state.tree.layout(self.tree_root_node).unwrap().size;
		if self.content_size.x != root_size.width || self.content_size.y != root_size.height {
			log::debug!(
				"content size changed: {:.0}x{:.0} → {:.0}x{:.0}",
				self.content_size.x,
				self.content_size.y,
				root_size.width,
				root_size.height
			);
		}
		self.content_size = vec2(root_size.width, root_size.height);
		Ok(())
	}

	pub fn update(&mut self, params: &mut LayoutUpdateParams) -> anyhow::Result<LayoutUpdateResult> {
		let mut alterables = EventAlterables::default();
		self
			.animations
			.process(&self.state, &mut alterables, params.timestep_alpha);
		self.process_alterables(alterables)?;
		self.try_recompute_layout(params.size)?;

		Ok(LayoutUpdateResult {
			sounds_to_play: std::mem::take(&mut self.sounds_to_play_once),
		})
	}

	pub fn tick(&mut self) -> anyhow::Result<()> {
		let mut alterables = EventAlterables::default();
		self.animations.tick(&self.state, &mut alterables);
		self.process_pending_components(&mut alterables);
		self.process_pending_widget_ticks(&mut alterables);
		self.process_alterables(alterables)?;
		Ok(())
	}

	pub fn process_tasks(&mut self) -> anyhow::Result<()> {
		let mut tasks = self.tasks.drain();
		while let Some(task) = tasks.pop_front() {
			match task {
				LayoutTask::RemoveWidget(widget_id) => {
					self.remove_widget(widget_id);
				}
				LayoutTask::ModifyLayoutState(callback) => {
					callback(ModifyLayoutStateData { layout: self })?;
				}
				LayoutTask::PlaySound(sound) => {
					if !self.sounds_to_play_once.contains(&sound) {
						self.sounds_to_play_once.push(sound);
					}
				}
				LayoutTask::Dispatch(func) => {
					let mut c = self.start_common();
					func(&mut c.common())?;
					c.finish()?;
				}
				LayoutTask::SetWidgetStyle(widget_id, style_request) => {
					self.set_style_request(widget_id, &style_request);
				}
				LayoutTask::SetFocus(weak) => {
					if let Some(c) = weak.upgrade() {
						self.set_focus(Some(&components::Component(c)))?;
					}
				}
				LayoutTask::Unfocus => self.set_focus(None)?,
			}
		}

		Ok(())
	}

	pub fn set_focus(&mut self, to_focus: Option<&Component>) -> anyhow::Result<()> {
		let mut c = self.start_common();

		if let Some(focused) = &c.layout.focused_component
			&& let Some(focused) = focused.upgrade()
		{
			// Unfocus
			focused.on_focus_change(&mut FocusChangeData {
				common: &mut c.common(),
				focused: false,
			});
			c.layout.focused_component = None;
		}

		if let Some(to_focus) = to_focus {
			to_focus.0.on_focus_change(&mut FocusChangeData {
				common: &mut c.common(),
				focused: true,
			});

			c.layout.focused_component = Some(to_focus.weak());
		}

		c.finish()?;
		Ok(())
	}

	fn set_style_request(&mut self, widget_id: WidgetID, style_request: &event::StyleSetRequest) {
		let Some(node_id) = self.state.nodes.get(widget_id) else {
			return;
		};

		// taffy requires us to copy this whole 536-byte style struct.
		// we can't get `&mut Style` directly from taffy unfortunately
		let mut cur_style = self.state.tree.style(*node_id).unwrap().clone() /* always safe */;

		match style_request {
			event::StyleSetRequest::Display(display) => {
				// refresh the component in case if visibility/display mode has changed
				if cur_style.display != *display
					&& let Some(component) = self.registered_components_to_refresh.get(node_id)
					&& let Some(c) = component.upgrade()
				{
					self.components_to_refresh_once.insert(Component(c));
				}

				cur_style.display = *display;
			}
			event::StyleSetRequest::Margin(margin) => {
				cur_style.margin = *margin;
			}
			event::StyleSetRequest::Width(val) => {
				cur_style.size.width = *val;
			}
			event::StyleSetRequest::Height(val) => {
				cur_style.size.height = *val;
			}
			event::StyleSetRequest::Size(size) => {
				cur_style.size = *size;
			}
		}

		if let Err(e) = self.state.tree.set_style(*node_id, cur_style) {
			log::error!("failed to set style for taffy widget ID {node_id:?}: {e:?}");
		}
	}

	pub fn process_alterables(&mut self, alterables: EventAlterables) -> anyhow::Result<()> {
		for task in alterables.tasks {
			self.tasks.push(task);
		}

		self.process_tasks()?;

		for dirty_widget_id in alterables.dirty_widgets {
			if let Some(dirty_node_id) = self.state.nodes.get(dirty_widget_id) {
				self.state.tree.mark_dirty(*dirty_node_id)?;
			}
		}

		if alterables.needs_redraw {
			self.mark_redraw();
		}

		if alterables.trigger_haptics {
			self.haptics_triggered = true;
		}

		if !alterables.animations.is_empty() {
			self.mark_redraw();
			for anim in alterables.animations {
				self.animations.add(anim);
			}
		}

		if !alterables.widgets_to_tick.is_empty() {
			for widget_id in &alterables.widgets_to_tick {
				self.widgets_to_tick.push(*widget_id);
			}
		}

		for c in alterables.components_to_refresh_once {
			if let Some(c) = c.upgrade() {
				self.defer_component_refresh(components::Component(c));
			}
		}

		for (widget_id, style_request) in &alterables.style_set_requests {
			self.set_style_request(*widget_id, style_request);
		}

		Ok(())
	}

	pub fn print_tree(&self) {
		let mut buf = Vec::<u8>::new();
		self.print_tree_recur(&mut buf, 0, self.tree_root_node);
		let str = format!(
			"\n=== tree ===\n[widget type] [WidgetID] [taffy NodeID] [other data]\n{}",
			unsafe { str::from_utf8_unchecked(&buf) }
		);
		std::io::stdout().write_all(str.as_bytes()).unwrap();
	}

	fn print_tree_recur(&self, buf: &mut Vec<u8>, depth: u32, node_id: taffy::NodeId) {
		// indent
		for _ in 0..depth {
			buf.push(b'|');
			buf.push(b' ');
		}

		let widget_id = self.state.tree.get_node_context(node_id).unwrap();
		let layout = self.state.tree.layout(node_id).unwrap();

		let widget = self.state.widgets.get(*widget_id).unwrap();

		let state = widget.state();

		let type_color = match state.obj.get_type() {
			widget::WidgetType::Div => drawing::Color::new(1.0, 1.0, 1.0, 1.0),
			widget::WidgetType::Label => drawing::Color::new(0.4, 1.0, 0.0, 1.0),
			widget::WidgetType::Sprite => drawing::Color::new(0.0, 0.8, 1.0, 1.0),
			widget::WidgetType::Rectangle => drawing::Color::new(1.0, 0.5, 0.2, 1.0),
			widget::WidgetType::CustomDraw => drawing::Color::new(1.0, 1.0, 1.0, 1.0),
		};

		let line = format!(
			"{}{}{}{} 0x{:x?} 0x{:x?}: [pos: {}x{}][size: {}x{}]{}\n",
			ANSI_BOLD_CODE,
			type_color.debug_ansi_format(),
			state.obj.get_type().as_str(),
			ANSI_RESET_CODE,
			state.obj.get_id().0.as_ffi(),
			u64::from(node_id),
			layout.location.x,
			layout.location.y,
			layout.content_size.width,
			layout.content_size.height,
			state.obj.debug_print()
		);

		buf.append(&mut line.into_bytes());

		for child_id in self.state.tree.child_ids(node_id) {
			self.print_tree_recur(buf, depth + 1, child_id);
		}
	}
}

impl LayoutState {
	pub fn get_node_boundary(&self, id: NodeId) -> Boundary {
		let Ok(layout) = self.tree.layout(id) else {
			return Boundary::default();
		};

		Boundary {
			pos: Vec2::new(layout.location.x, layout.location.y),
			size: Vec2::new(layout.size.width, layout.size.height),
		}
	}

	pub fn get_node_size(&self, id: NodeId) -> Vec2 {
		let Ok(layout) = self.tree.layout(id) else {
			return Vec2::ZERO;
		};

		Vec2::new(layout.size.width, layout.size.height)
	}

	pub fn get_node_style(&self, id: NodeId) -> Option<&taffy::Style> {
		let Ok(style) = self.tree.style(id) else {
			return None;
		};

		Some(style)
	}

	pub fn get_widget_boundary(&self, id: WidgetID) -> Boundary {
		let Some(node_id) = self.nodes.get(id) else {
			return Boundary::default();
		};

		self.get_node_boundary(*node_id)
	}

	pub fn get_widget_size(&self, id: WidgetID) -> Vec2 {
		let Some(node_id) = self.nodes.get(id) else {
			return Vec2::ZERO;
		};

		self.get_node_size(*node_id)
	}

	pub fn get_widget_style(&self, id: WidgetID) -> Option<&taffy::Style> {
		self.get_node_style(*self.nodes.get(id)?)
	}
}
