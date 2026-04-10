use std::{
	any::{Any, TypeId},
	cell::RefMut,
	collections::HashSet,
	rc::{Rc, Weak},
};

use glam::Vec2;
use slotmap::{DenseSlotMap, new_key_type};

use crate::{
	animation::{self, Animation},
	components::{ComponentTrait, ComponentWeak},
	globals,
	i18n::I18n,
	layout::{LayoutDispatchFunc, LayoutState, LayoutTask, WidgetID},
	sound::WguiSoundType,
	stack::{ScissorStack, Transform, TransformStack},
	widget::{EventResult, WidgetData, WidgetObj},
};

#[derive(Debug, Clone, Copy)]
pub enum MouseButtonIndex {
	Left,
	Right,
	Middle,
}

#[derive(Debug, Clone, Copy)]
pub struct MouseButtonEvent {
	pub index: MouseButtonIndex,
	pub pos: Vec2,
	pub device: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct MousePosition {
	pub pos: Vec2,
	pub device: usize,
}

pub struct MouseLeaveEvent {
	pub device: usize,
}

pub struct MouseMotionEvent {
	pub pos: Vec2,
	pub device: usize,
}

pub struct MouseWheelEvent {
	pub pos: Vec2,   /* mouse position */
	pub delta: Vec2, /* wheel delta */
	pub device: usize,
}

#[derive(Clone)]
pub struct TextInputEvent {
	pub text: Option<Rc<str>>,
}

pub struct InternalStateChangeEvent {
	pub metadata: usize,
}

pub enum Event {
	InternalStateChange(InternalStateChangeEvent),
	MouseDown(MouseButtonEvent),
	MouseLeave(MouseLeaveEvent),
	MouseMotion(MouseMotionEvent),
	MouseUp(MouseButtonEvent),
	MouseWheel(MouseWheelEvent),
	TextInput(TextInputEvent),
}

impl Event {
	fn test_transform_pos(transform: &Transform, pos: Vec2) -> bool {
		pos.x >= transform.abs_pos.x
			&& pos.x < transform.abs_pos.x + transform.visual_dim.x
			&& pos.y >= transform.abs_pos.y
			&& pos.y < transform.abs_pos.y + transform.visual_dim.y
	}

	pub fn test_mouse_within_transform(&self, transform: &Transform) -> bool {
		match self {
			Self::MouseDown(evt) => Self::test_transform_pos(transform, evt.pos),
			Self::MouseMotion(evt) => Self::test_transform_pos(transform, evt.pos),
			Self::MouseUp(evt) => Self::test_transform_pos(transform, evt.pos),
			Self::MouseWheel(evt) => Self::test_transform_pos(transform, evt.pos),
			_ => false,
		}
	}
}

pub enum StyleSetRequest {
	Display(taffy::Display),
	Margin(taffy::Rect<taffy::LengthPercentageAuto>),
	Width(taffy::Dimension),
	Height(taffy::Dimension),
	Size(taffy::Size<taffy::Dimension>),
}

// alterables which will be dispatched in the next loop iteration phase
#[derive(Default)]
pub struct EventAlterables {
	pub dirty_widgets: Vec<WidgetID>,
	pub components_to_refresh_once: Vec<ComponentWeak>,
	pub style_set_requests: Vec<(WidgetID, StyleSetRequest)>,
	pub animations: Vec<animation::Animation>,
	pub widgets_to_tick: HashSet<WidgetID>, // widgets which needs to be ticked in the next `Layout::update()` fn
	pub transform_stack: TransformStack,
	pub scissor_stack: ScissorStack,
	pub tasks: Vec<LayoutTask>,
	pub needs_redraw: bool,
	pub trigger_haptics: bool,
}

// helper functions
impl EventAlterables {
	pub const fn mark_redraw(&mut self) {
		self.needs_redraw = true;
	}

	pub fn set_style(&mut self, widget_id: WidgetID, request: StyleSetRequest) {
		self.style_set_requests.push((widget_id, request));
	}

	pub fn mark_dirty(&mut self, widget_id: WidgetID) {
		self.dirty_widgets.push(widget_id);
	}

	pub fn mark_tick(&mut self, widget_id: WidgetID) {
		self.widgets_to_tick.insert(widget_id);
	}

	pub const fn trigger_haptics(&mut self) {
		self.trigger_haptics = true;
	}

	pub fn animate(&mut self, animation: Animation) {
		self.animations.push(animation);
	}

	pub fn play_sound(&mut self, sound_type: WguiSoundType) {
		self.tasks.push(LayoutTask::PlaySound(sound_type));
	}

	pub fn dispatch(&mut self, func: LayoutDispatchFunc) {
		self.tasks.push(LayoutTask::Dispatch(func));
	}

	pub fn focus<ComponentType: ComponentTrait>(&mut self, component: &Weak<ComponentType>) {
		self.tasks.push(LayoutTask::SetFocus(component.clone()));
	}

	pub fn unfocus(&mut self) {
		self.tasks.push(LayoutTask::Unfocus);
	}

	pub fn refresh_component_once<ComponentType: ComponentTrait>(&mut self, component: &Weak<ComponentType>) {
		self.components_to_refresh_once.push(component.clone());
	}
}

pub struct CallbackDataCommon<'a> {
	pub state: &'a LayoutState,
	pub alterables: &'a mut EventAlterables,
}

impl CallbackDataCommon<'_> {
	pub fn i18n(&self) -> RefMut<'_, I18n> {
		self.state.globals.i18n()
	}

	// helper functions
	pub fn mark_widget_dirty(&mut self, id: WidgetID) {
		self.alterables.mark_dirty(id);
		self.alterables.mark_redraw();
	}

	pub fn globals(&self) -> RefMut<'_, globals::Globals> {
		self.state.globals.get()
	}
}

pub struct CallbackData<'a> {
	pub obj: &'a mut dyn WidgetObj,
	pub widget_data: &'a mut WidgetData,
	pub widget_id: WidgetID,
	pub node_id: taffy::NodeId,
	pub metadata: CallbackMetadata,
}

pub enum CallbackMetadata {
	None,
	MouseButton(MouseButtonEvent),
	MousePosition(MousePosition),
	TextInput(TextInputEvent),
	Custom(usize),
}

impl CallbackMetadata {
	// helper function

	pub const fn get_mouse_pos_absolute(&self) -> Option<Vec2> {
		match *self {
			CallbackMetadata::MouseButton(b) => Some(b.pos),
			CallbackMetadata::MousePosition(b) => Some(b.pos),
			CallbackMetadata::Custom(_) | CallbackMetadata::None => None,
			CallbackMetadata::TextInput(_) => None,
		}
	}

	pub fn get_mouse_pos_relative(&self, transform_stack: &TransformStack) -> Option<Vec2> {
		let mouse_pos_abs = self.get_mouse_pos_absolute()?;
		Some(mouse_pos_abs - transform_stack.get().abs_pos)
	}
	pub fn get_mouse_pos_normalized(&self, transform_stack: &TransformStack) -> Option<Vec2> {
		let pos_relative = self.get_mouse_pos_relative(transform_stack)?;
		Some(pos_relative / transform_stack.parent().raw_dim)
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventListenerKind {
	MousePress,
	MouseRelease,
	MouseEnter,
	MouseMotion,
	MouseLeave,
	TextInput,
	InternalStateChange,
}

pub type EventCallbackInternal = Box<
	dyn for<'a, 'b, 'c, 'd> Fn(
		&'a mut CallbackDataCommon<'b>,
		&'a mut CallbackData<'c>,
		&'d mut dyn Any,
		&'d mut dyn Any,
	) -> anyhow::Result<EventResult>,
>;

pub type EventCallback<U1, U2> = Box<
	dyn for<'a, 'b, 'c, 'd> Fn(
		&'a mut CallbackDataCommon<'b>,
		&'a mut CallbackData<'c>,
		&'d mut U1,
		&'d mut U2,
	) -> anyhow::Result<EventResult>,
>;

new_key_type! {
	pub struct EventListenerID;
}

pub struct EventListener {
	kind: EventListenerKind,
	callback: EventCallbackInternal,
	tid1: Option<TypeId>,
	tid2: Option<TypeId>,
}

impl EventListener {
	pub fn call_with<U1: 'static, U2: 'static>(
		&self,
		common: &mut CallbackDataCommon,
		data: &mut CallbackData,
		user_data: &mut (&mut U1, &mut U2),
	) -> anyhow::Result<EventResult> {
		let a1: &mut (dyn Any + 'static) = if self.tid1.is_none() { &mut () } else { user_data.0 };
		let a2: &mut (dyn Any + 'static) = if self.tid2.is_none() { &mut () } else { user_data.1 };
		(self.callback)(common, data, a1, a2)
	}
}

#[derive(Default)]
pub struct EventListenerCollection {
	inner: DenseSlotMap<EventListenerID, EventListener>,
}

impl EventListenerCollection {
	/// Iterates over event handlers with a matching U type
	pub fn iter_filtered<U1: 'static, U2: 'static>(
		&self,
		kind: EventListenerKind,
	) -> impl Iterator<Item = &EventListener> {
		let tid1 = TypeId::of::<U1>();
		let tid2 = TypeId::of::<U2>();
		self
			.inner
			.values()
			.filter(move |p| p.tid1.is_none_or(|a| a == tid1) && p.tid2.is_none_or(|a| a == tid2) && p.kind == kind)
	}

	pub fn register<U1: 'static, U2: 'static>(
		&mut self,
		kind: EventListenerKind,
		callback: EventCallback<U1, U2>,
	) -> EventListenerID {
		let tid_unit = TypeId::of::<()>();

		let tid1 = TypeId::of::<U1>();
		let tid2 = TypeId::of::<U2>();

		let callback_inner: EventCallbackInternal = Box::new(move |common, data, u1_any, u2_any| {
			if let Some(u1) = u1_any.downcast_mut::<U1>()
				&& let Some(u2) = u2_any.downcast_mut::<U2>()
			{
				callback(common, data, u1, u2)
			} else {
				Ok(EventResult::Pass)
			}
		});

		let new_item = EventListener {
			kind,
			callback: callback_inner,
			tid1: (tid1 != tid_unit).then_some(tid1),
			tid2: (tid2 != tid_unit).then_some(tid2),
		};

		self.inner.insert(new_item)
	}

	pub fn remove(&mut self, event_listener_id: EventListenerID) -> Option<EventListener> {
		self.inner.remove(event_listener_id)
	}
}
