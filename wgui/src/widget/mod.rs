use anyhow::Context;
use glam::Vec2;
use taffy::{NodeId, TaffyTree};

use super::drawing::RenderPrimitive;

use crate::{
	any::AnyTrait,
	drawing::{self, PrimitiveExtent},
	event::{
		self, CallbackData, CallbackDataCommon, CallbackMetadata, Event, EventAlterables, EventListenerCollection,
		EventListenerKind::{self, InternalStateChange, MouseLeave},
		MouseWheelEvent,
	},
	globals::Globals,
	layout::{Layout, LayoutState, WidgetID},
	stack::{ScissorStack, TransformStack},
};

pub mod custom_draw;
pub mod div;
pub mod image;
pub mod label;
pub mod rectangle;
pub mod sprite;
pub mod util;

pub struct WidgetData {
	hovered: usize,
	pressed: usize,
	pub scrolling_target: Vec2,   // normalized, 0.0-1.0. Not used in case if overflow != scroll
	pub scrolling_cur: Vec2,      // normalized, used for smooth scrolling animation
	pub scrolling_cur_prev: Vec2, // for motion interpolation while rendering between ticks
	pub transform: glam::Mat4,
	pub cached_absolute_boundary: drawing::Boundary, // updated in Layout::push_event_widget
}

impl WidgetData {
	pub const fn set_device_pressed(&mut self, device: usize, pressed: bool) -> bool {
		let bit = 1 << device;
		let state_changed;
		if pressed {
			state_changed = self.pressed == 0;
			self.pressed |= bit;
		} else {
			state_changed = self.pressed == bit;
			self.pressed &= !bit;
		}
		state_changed
	}

	pub const fn set_device_hovered(&mut self, device: usize, hovered: bool) -> bool {
		let bit = 1 << device;
		let state_changed;
		if hovered {
			state_changed = self.hovered == 0;
			self.hovered |= bit;
		} else {
			state_changed = self.hovered == bit;
			self.hovered &= !bit;
		}
		state_changed
	}

	pub const fn get_pressed(&self, device: usize) -> bool {
		self.pressed & (1 << device) != 0
	}

	pub const fn get_hovered(&self, device: usize) -> bool {
		self.hovered & (1 << device) != 0
	}

	pub const fn is_pressed(&self) -> bool {
		self.pressed != 0
	}

	pub const fn is_hovered(&self) -> bool {
		self.hovered != 0
	}
}

pub struct WidgetStateFlags {
	pub interactable: bool,

	// consume any incoming mouse event which is hovered at given widget
	pub consume_mouse_events: bool,

	// force a new render pass before rendering this widget
	pub new_pass: bool,
}

impl Default for WidgetStateFlags {
	fn default() -> Self {
		Self {
			interactable: true,
			consume_mouse_events: false,
			new_pass: false,
		}
	}
}

pub struct WidgetState {
	pub data: WidgetData,
	pub obj: Box<dyn WidgetObj>,
	pub event_listeners: EventListenerCollection,
	pub flags: WidgetStateFlags,
}

impl WidgetState {
	pub fn get_data_obj_mut(&mut self) -> (&mut WidgetData, &mut dyn WidgetObj) {
		let data = &mut self.data;
		let obj = self.obj.as_mut();
		(data, obj)
	}

	fn new(flags: WidgetStateFlags, obj: Box<dyn WidgetObj>) -> Self {
		Self {
			data: WidgetData {
				hovered: 0,
				pressed: 0,
				scrolling_target: Vec2::default(),
				scrolling_cur: Vec2::default(),
				scrolling_cur_prev: Vec2::default(),
				transform: glam::Mat4::IDENTITY,
				cached_absolute_boundary: drawing::Boundary::default(),
			},
			obj,
			event_listeners: EventListenerCollection::default(),
			flags,
		}
	}
}

// global draw params
pub struct DrawState<'a> {
	pub globals: &'a Globals,
	pub layout: &'a Layout,
	pub primitives: &'a mut Vec<RenderPrimitive>,
	pub transform_stack: &'a mut TransformStack,
	pub scissor_stack: &'a mut ScissorStack,
	pub alterables: &'a mut EventAlterables,
}

// per-widget draw params
pub struct DrawParams<'a> {
	pub node_id: taffy::NodeId,
	pub style: &'a taffy::Style,
	pub taffy_layout: &'a taffy::Layout,
}

pub enum WidgetType {
	Div,
	Label,
	Sprite,
	Rectangle,
	CustomDraw,
}

impl WidgetType {
	pub const fn as_str(&self) -> &str {
		match self {
			WidgetType::Div => "div",
			WidgetType::Label => "label",
			WidgetType::Sprite => "sprite",
			WidgetType::Rectangle => "rectangle",
			WidgetType::CustomDraw => "custom_draw",
		}
	}
}

pub trait WidgetObj: AnyTrait {
	// every widget stores their of id for convenience reasons
	fn get_id(&self) -> WidgetID;
	fn set_id(&mut self, id: WidgetID); // always set at insertion
	fn get_type(&self) -> WidgetType;
	fn debug_print(&self) -> String;

	fn draw(&mut self, state: &mut DrawState, params: &DrawParams);

	fn measure(
		&mut self,
		_globals: &Globals,
		_known_dimensions: taffy::Size<Option<f32>>,
		_available_space: taffy::Size<taffy::AvailableSpace>,
	) -> taffy::Size<f32> {
		taffy::Size::ZERO
	}
}

pub struct EventParams<'a> {
	pub node_id: taffy::NodeId,
	pub style: &'a taffy::Style,
	pub state: &'a LayoutState,
	pub alterables: &'a mut EventAlterables,
	pub layout: &'a taffy::Layout,
}

#[derive(Clone, Copy, Eq, PartialEq, PartialOrd)]
pub enum EventResult {
	NoHit,    // event was pushed but has not found a listener (yet)
	Pass,     // widget acknowledged it and allows the event to propagate further
	Consumed, // widget triggered an action, do not propagate further
}

impl EventResult {
	#[must_use]
	pub const fn can_propagate(self) -> bool {
		!matches!(self, EventResult::Consumed)
	}

	#[must_use]
	pub fn merge(self, other: Self) -> Self {
		if self > other { self } else { other }
	}
}

fn get_scroll_enabled(style: &taffy::Style) -> (bool, bool) {
	(
		style.overflow.x == taffy::Overflow::Scroll,
		style.overflow.y == taffy::Overflow::Scroll,
	)
}

pub struct ScrollbarInfo {
	// total contents size of the currently scrolling widget
	content_size: Vec2,
	// 0.0 - 1.0
	// 1.0: scrollbar handle not visible (inactive)
	handle_size: Vec2,
}

pub fn get_scrollbar_info(l: &taffy::Layout) -> Option<ScrollbarInfo> {
	let overflow_start_threshold_units = 3.0; // don't show scrollbars for nearly non-scrollable lists

	let overflow = Vec2::new(l.scroll_width(), l.scroll_height());
	if overflow.x < overflow_start_threshold_units && overflow.y < overflow_start_threshold_units {
		return None; // not overflowing
	}

	let content_size = Vec2::new(l.content_size.width, l.content_size.height);
	let handle_size = 1.0 - (overflow / content_size);

	Some(ScrollbarInfo {
		content_size,
		handle_size,
	})
}

impl dyn WidgetObj {
	pub fn get_as<T: 'static>(&self) -> Option<&T> {
		let any = self.as_any();
		any.downcast_ref::<T>()
	}

	pub fn cast<T: 'static>(&self) -> anyhow::Result<&T> {
		self.get_as().context("cast failed")
	}

	pub fn get_as_mut<T: 'static>(&mut self) -> Option<&mut T> {
		let any = self.as_any_mut();
		any.downcast_mut::<T>()
	}

	pub fn cast_mut<T: 'static>(&mut self) -> anyhow::Result<&mut T> {
		self.get_as_mut().context("cast failed")
	}
}

struct InvokeData<'a, 'b, U1: 'static, U2: 'static> {
	widget_id: WidgetID,
	node_id: taffy::NodeId,
	event_result: &'a mut EventResult,
	user_data: &'a mut (&'b mut U1, &'b mut U2),
	params: &'a mut EventParams<'a>,
}

#[must_use]
enum InvokeListenersResult {
	NobodyListened,
	AtLeastOneCalled,
}

impl WidgetState {
	fn invoke_listeners<U1: 'static, U2: 'static>(
		&mut self,
		call_data: &mut InvokeData<'_, '_, U1, U2>,
		kind: event::EventListenerKind,
		metadata: CallbackMetadata,
	) -> anyhow::Result<InvokeListenersResult> {
		let mut data = CallbackData {
			obj: self.obj.as_mut(),
			widget_data: &mut self.data,
			widget_id: call_data.widget_id,
			node_id: call_data.node_id,
			metadata,
		};

		let mut common = CallbackDataCommon {
			state: call_data.params.state,
			alterables: call_data.params.alterables,
		};

		let mut res = InvokeListenersResult::NobodyListened;

		for listener in self.event_listeners.iter_filtered::<U1, U2>(kind) {
			let new_result = listener.call_with(&mut common, &mut data, call_data.user_data)?;
			res = InvokeListenersResult::AtLeastOneCalled;
			// Consider all listeners on this widget, even if we had a Consume.
			// Store the highest value for return.
			*call_data.event_result = call_data.event_result.merge(new_result);
		}

		Ok(res)
	}

	pub fn get_scroll_shift_smooth(&self, info: &ScrollbarInfo, l: &taffy::Layout, timestep_alpha: f32) -> (Vec2, bool) {
		//return (self.get_scroll_shift_raw(info, l), false);

		let currently_animating = self.data.scrolling_cur != self.data.scrolling_cur_prev;

		let scrolling = self
			.data
			.scrolling_cur_prev
			.lerp(self.data.scrolling_cur, timestep_alpha);

		(
			Vec2::new(
				(info.content_size.x - l.content_box_width()) * scrolling.x,
				(info.content_size.y - l.content_box_height()) * scrolling.y,
			),
			currently_animating,
		)
	}

	pub fn get_scroll_shift_raw(&self, info: &ScrollbarInfo, l: &taffy::Layout) -> Vec2 {
		Vec2::new(
			(info.content_size.x - l.content_box_width()) * self.data.scrolling_target.x,
			(info.content_size.y - l.content_box_height()) * self.data.scrolling_target.y,
		)
	}

	pub fn draw_all(&mut self, state: &mut DrawState, params: &DrawParams) {
		self.obj.draw(state, params);
	}

	pub fn tick(&mut self, this_widget_id: WidgetID, alterables: &mut EventAlterables) {
		let scrolling_cur = &mut self.data.scrolling_cur;
		let scrolling_cur_prev = &mut self.data.scrolling_cur_prev;
		let scrolling_target = &mut self.data.scrolling_target;

		*scrolling_cur_prev = *scrolling_cur;

		if scrolling_cur != scrolling_target {
			// the magic part
			*scrolling_cur = scrolling_cur.lerp(*scrolling_target, 0.2);

			// trigger tick request again
			alterables.mark_tick(this_widget_id);
			alterables.mark_redraw();

			let epsilon = 0.00001;
			if (scrolling_cur.x - scrolling_target.x).abs() < epsilon
				&& (scrolling_cur.y - scrolling_target.y).abs() < epsilon
			{
				*scrolling_cur = *scrolling_target;
			}
		}
	}

	pub fn draw_scrollbars(&mut self, state: &mut DrawState, params: &DrawParams, info: &ScrollbarInfo) {
		let (enabled_horiz, enabled_vert) = get_scroll_enabled(params.style);
		if !enabled_horiz && !enabled_vert {
			return;
		}

		let transform_widget = state.transform_stack.get();
		let transform_parent = state.transform_stack.parent();

		let thickness = 6.0;
		let margin = 6.0;

		let rect_params = drawing::Rectangle {
			color: drawing::Color::new(1.0, 1.0, 1.0, 0.5),
			round_units: 4,
			..Default::default()
		};

		// Horizontal handle
		if enabled_horiz && info.handle_size.x < 1.0 {
			state.primitives.push(drawing::RenderPrimitive::Rectangle(
				PrimitiveExtent {
					boundary: drawing::Boundary::from_pos_size(
						Vec2::new(
							// X
							transform_widget.content_rel_pos.x
								+ transform_parent.raw_dim.x * (1.0 - info.handle_size.x) * self.data.scrolling_cur.x,
							// Y
							transform_widget.content_rel_pos.y + transform_widget.raw_dim.y - thickness - margin,
						),
						Vec2::new(transform_widget.raw_dim.x * info.handle_size.x, thickness),
					),
					transform: transform_parent.transform,
				},
				rect_params,
			));
		}

		// Vertical handle
		if enabled_vert && info.handle_size.y < 1.0 {
			state.primitives.push(drawing::RenderPrimitive::Rectangle(
				PrimitiveExtent {
					boundary: drawing::Boundary::from_pos_size(
						Vec2::new(
							// X
							transform_widget.content_rel_pos.x + transform_widget.raw_dim.x - thickness - margin,
							// Y
							transform_widget.content_rel_pos.y
								+ transform_parent.raw_dim.y * (1.0 - info.handle_size.y) * self.data.scrolling_cur.y,
						),
						Vec2::new(thickness, transform_widget.raw_dim.y * info.handle_size.y),
					),
					transform: transform_parent.transform,
				},
				rect_params,
			));
		}
	}

	fn process_wheel(&mut self, params: &mut EventParams, wheel: &MouseWheelEvent) -> bool {
		let (enabled_horiz, enabled_vert) = get_scroll_enabled(params.style);
		if !enabled_horiz && !enabled_vert {
			return false;
		}

		let l = params.layout;
		let overflow = Vec2::new(l.scroll_width(), l.scroll_height());
		if overflow.x == 0.0 && overflow.y == 0.0 {
			return false; // not overflowing
		}

		let Some(info) = get_scrollbar_info(params.layout) else {
			return false;
		};

		let step_pixels = 64.0;

		let mut handle_scroll =
			|scrolling_target: &mut f32, wheel_delta: f32, handle_size: f32, content_length: f32, content_box_length: f32| {
				if handle_size >= 1.0 || wheel_delta == 0.0 {
					return;
				}

				let mult = (1.0 / (content_box_length - content_length)) * step_pixels;
				let new_scroll = (*scrolling_target + wheel_delta * mult).clamp(0.0, 1.0);
				if *scrolling_target == new_scroll {
					return;
				}

				*scrolling_target = new_scroll;
				params.alterables.mark_tick(self.obj.get_id());
			};

		handle_scroll(
			&mut self.data.scrolling_target.x,
			wheel.delta.x,
			info.handle_size.x,
			info.content_size.x,
			l.content_box_width(),
		);

		handle_scroll(
			&mut self.data.scrolling_target.y,
			wheel.delta.y,
			info.handle_size.y,
			info.content_size.y,
			l.content_box_height(),
		);

		true
	}

	pub fn process_event<'a, 'b, U1: 'static, U2: 'static>(
		&mut self,
		widget_id: WidgetID,
		node_id: taffy::NodeId,
		event: &Event,
		event_result: &'a mut EventResult,
		user_data: &'a mut (&'b mut U1, &'b mut U2),
		params: &'a mut EventParams<'a>,
	) -> anyhow::Result<()> {
		let hovered = event.test_mouse_within_transform(params.alterables.transform_stack.get());

		let mut invoke_data = InvokeData {
			widget_id,
			node_id,
			event_result,
			user_data,
			params,
		};

		let mut res: Option<InvokeListenersResult> = None;

		match &event {
			Event::TextInput(e) => {
				res = Some(self.invoke_listeners(
					&mut invoke_data,
					EventListenerKind::TextInput,
					CallbackMetadata::TextInput(e.clone()),
				)?);
			}
			Event::MouseDown(e) => {
				if hovered && self.data.set_device_pressed(e.device, true) {
					res = Some(self.invoke_listeners(
						&mut invoke_data,
						EventListenerKind::MousePress,
						CallbackMetadata::MouseButton(*e),
					)?);
				}
			}
			Event::MouseUp(e) => {
				if self.data.set_device_pressed(e.device, false) {
					res = Some(self.invoke_listeners(
						&mut invoke_data,
						EventListenerKind::MouseRelease,
						CallbackMetadata::MouseButton(*e),
					)?);
				}
			}
			Event::MouseMotion(e) => {
				let hover_state_changed = self.data.set_device_hovered(e.device, hovered);

				if hover_state_changed {
					if self.data.is_hovered() {
						res =
							Some(self.invoke_listeners(&mut invoke_data, EventListenerKind::MouseEnter, CallbackMetadata::None)?);
					} else {
						res =
							Some(self.invoke_listeners(&mut invoke_data, EventListenerKind::MouseLeave, CallbackMetadata::None)?);
					}
				} else {
					res = Some(self.invoke_listeners(
						&mut invoke_data,
						EventListenerKind::MouseMotion,
						CallbackMetadata::MousePosition(event::MousePosition {
							pos: e.pos,
							device: e.device,
						}),
					)?);

					if self.flags.interactable {
						*invoke_data.event_result = invoke_data.event_result.merge(EventResult::Pass);
					}
				}
			}
			Event::MouseWheel(e) => {
				if hovered && self.process_wheel(invoke_data.params, e) {
					*invoke_data.event_result = EventResult::Consumed;
					return Ok(());
				}
			}
			Event::MouseLeave(e) => {
				if self.data.set_device_hovered(e.device, false) {
					res = Some(self.invoke_listeners(&mut invoke_data, MouseLeave, CallbackMetadata::None)?);
				}
			}
			Event::InternalStateChange(e) => {
				res = Some(self.invoke_listeners(
					&mut invoke_data,
					InternalStateChange,
					CallbackMetadata::Custom(e.metadata),
				)?);
			}
		}

		if let Some(res) = res {
			match res {
				InvokeListenersResult::NobodyListened => {
					if hovered && self.flags.consume_mouse_events {
						*invoke_data.event_result = EventResult::Consumed;
					}
				}
				InvokeListenersResult::AtLeastOneCalled => {}
			}
		}

		Ok(())
	}
}

pub struct ConstructEssentials<'a> {
	pub layout: &'a mut Layout,
	pub parent: WidgetID,
}

/// Determines whether a given node is visible within the layout tree.
///
/// Traversal is definitely a little bit more expensive than just checking the value, but
/// Taffy doesn't calculate that for us, so here it is.
pub fn is_node_visible(tree: &TaffyTree<WidgetID>, node_id: NodeId) -> bool {
	let mut cur = Some(node_id);
	while let Some(node_id) = cur {
		if let Ok(style) = tree.style(node_id)
			&& style.display == taffy::Display::None
		{
			return false;
		}
		cur = tree.parent(node_id);
	}
	true
}
