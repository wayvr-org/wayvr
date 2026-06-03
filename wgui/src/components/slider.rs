use std::{cell::RefCell, f32, rc::Rc};

use glam::{Mat4, Vec2, Vec3};
use taffy::prelude::{length, percent};

use crate::{
	animation::{Animation, AnimationEasing},
	components::{
		Component, ComponentBase, ComponentTrait, RefreshData,
		tooltip::{self, ComponentTooltip, TooltipTrait},
	},
	drawing::{self},
	event::{
		self, CallbackDataCommon, CallbackMetadata, DeviceBitmask, EventAlterables, EventListenerCollection,
		EventListenerKind, StyleSetRequest,
	},
	i18n::Translation,
	layout::{WidgetID, WidgetPair},
	renderer_vk::{
		text::{FontWeight, HorizontalAlign, TextStyle},
		util,
	},
	widget::{
		ConstructEssentials, EventResult,
		div::WidgetDiv,
		label::{WidgetLabel, WidgetLabelParams},
		rectangle::{WidgetRectangle, WidgetRectangleParams},
		util::WLength,
	},
};

#[derive(Default, Clone)]
pub struct Value(pub f32);

#[derive(Default)]
pub struct Limits {
	pub min_value: f32,
	pub max_value: f32,
	pub step: f32,
}

impl Value {
	pub const fn get(&self) -> f32 {
		self.0
	}

	pub fn set(&mut self, limits: &Limits, new_value: f32) {
		let span = limits.max_value - limits.min_value;
		let clamped = new_value.max(limits.min_value).min(limits.max_value);

		// get the step index from min
		let mut k = ((clamped - limits.min_value) / limits.step).round();

		let k_max = (span / limits.step).round();
		if k < 0.0 {
			k = 0.0;
		}
		if k > k_max {
			k = k_max;
		}

		let snapped = limits.min_value + k * limits.step;
		self.0 = snapped.max(limits.min_value).min(limits.max_value);
	}
}

impl Limits {
	fn to_normalized(&self, value: f32) -> f32 {
		(value - self.min_value) / (self.max_value - self.min_value)
	}

	fn get_from_normalized(&self, normalized: f32) -> f32 {
		normalized * (self.max_value - self.min_value) + self.min_value
	}
}

#[derive(Default)]
pub struct Params {
	pub style: taffy::Style,
	pub limits: Limits,
	pub value1: Value,
	pub value2: Option<Value>, // range slider support
	pub show_value: bool,
	pub tooltip: Option<tooltip::TooltipInfo>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ValueIndex {
	Primary,
	Secondary, /* for range sliders */
}

struct DraggedBy {
	index: ValueIndex,
	device: DeviceBitmask,
}

struct State {
	dragged_by: Option<DraggedBy>,
	hovered_body: bool,
	hovered1: bool,
	hovered2: bool,
	value1: Value,
	value2: Option<Value>,
	limits: Limits,
	on_value_changed: Option<SliderValueChangedCallback>,
	active_tooltip: Option<Rc<ComponentTooltip>>,
}

impl TooltipTrait for State {
	fn get(&mut self) -> &mut Option<Rc<ComponentTooltip>> {
		&mut self.active_tooltip
	}
}

#[allow(clippy::struct_field_names)]
struct SliderHandleData {
	id_handle_rect: WidgetID,  // Rectangle
	id_text: Option<WidgetID>, // Text
	id_handle: WidgetID,
}

struct Data {
	body_node: taffy::NodeId,
	handle1: SliderHandleData,
	handle2: Option<SliderHandleData>,
}

pub struct SliderValueChangedEvent {
	pub index: ValueIndex,
	pub value: f32,
}

pub type SliderValueChangedCallback = Box<dyn Fn(&mut CallbackDataCommon, SliderValueChangedEvent)>;

pub struct ComponentSlider {
	base: ComponentBase,
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
}

impl ComponentTrait for ComponentSlider {
	fn refresh(&self, data: &mut RefreshData) {
		let mut common = data.layout.common();
		let mut state = self.state.borrow_mut();

		let value1 = state.value1.get();
		state.set_value(&mut common, &self.data, ValueIndex::Primary, value1);

		if let Some(value2) = state.value2.as_ref().map(Value::get) {
			state.set_value(&mut common, &self.data, ValueIndex::Secondary, value2);
		}
	}

	fn base(&self) -> &ComponentBase {
		&self.base
	}

	fn base_mut(&mut self) -> &mut ComponentBase {
		&mut self.base
	}
}

impl ComponentSlider {
	pub fn get_value_primary(&self) -> f32 {
		self.get_value(ValueIndex::Primary).unwrap() /* safe */
	}

	pub fn get_value(&self, index: ValueIndex) -> Option<f32> {
		let state = self.state.borrow();
		match index {
			ValueIndex::Primary => Some(state.value1.get()),
			ValueIndex::Secondary => state.value2.as_ref().map(Value::get),
		}
	}

	pub fn set_value(&self, common: &mut CallbackDataCommon, index: ValueIndex, new_value: f32) {
		let mut state = self.state.borrow_mut();
		state.set_value(common, &self.data, index, new_value);
	}

	pub fn set_value_primary(&self, common: &mut CallbackDataCommon, new_value: f32) {
		self.set_value(common, ValueIndex::Primary, new_value);
	}

	pub fn on_value_changed(&self, func: SliderValueChangedCallback) {
		self.state.borrow_mut().on_value_changed = Some(func);
	}
}

// NOTICE: this can be re-used in the future
fn map_mouse_x_to_normalized(mouse_x_rel: f32, widget_width: f32) -> f32 {
	(mouse_x_rel / widget_width).clamp(0.0, 1.0)
}

fn get_width(slider_body_node: taffy::NodeId, tree: &taffy::tree::TaffyTree<WidgetID>) -> f32 {
	let layout = tree.layout(slider_body_node).unwrap(); /* shouldn't fail */
	layout.size.width
}

fn conf_handle_style(
	alterables: &mut EventAlterables,
	limits: &Limits,
	value: f32,
	slider_handle_id: WidgetID,
	body_node: taffy::NodeId,
	slider_handle_style: &taffy::Style,
	tree: &taffy::tree::TaffyTree<WidgetID>,
) -> bool {
	/* returns false if nothing has changed */
	let norm = limits.to_normalized(value);

	// convert normalized value to taffy percentage margin in percent
	let width = get_width(body_node, tree);
	let percent_margin = (HANDLE_WIDTH / width) / 2.0;

	let new_percent = percent(percent_margin + norm * (1.0 - percent_margin * 2.0));

	if slider_handle_style.margin.left == new_percent {
		return false; // nothing changed
	}

	let mut margin = slider_handle_style.margin;
	margin.left = new_percent;
	alterables.set_style(slider_handle_id, StyleSetRequest::Margin(margin));

	true
}

const PAD_PERCENT: f32 = 0.75;
const HANDLE_WIDTH: f32 = 32.0;
const HANDLE_HEIGHT: f32 = 24.0;

impl State {
	const fn get_hovered_index(&self) -> Option<ValueIndex> {
		if self.hovered1 {
			Some(ValueIndex::Primary)
		} else if self.hovered2 {
			Some(ValueIndex::Secondary)
		} else {
			None
		}
	}

	fn update_value_to_mouse(
		&mut self,
		event_data: &event::CallbackData<'_>,
		data: &Data,
		common: &mut CallbackDataCommon,
		index: ValueIndex,
	) {
		let mouse_pos = event_data
			.metadata
			.get_mouse_pos_relative(&common.alterables.transform_stack)
			.unwrap(); // safe

		let norm = map_mouse_x_to_normalized(
			mouse_pos.x - HANDLE_WIDTH / 2.0,
			get_width(data.body_node, &common.state.tree) - HANDLE_WIDTH,
		);

		let target_value = self.limits.get_from_normalized(norm);
		let val = target_value;

		self.set_value(common, data, index, val);
	}

	fn update_text(common: &mut CallbackDataCommon, text: &mut WidgetLabel, value: f32) {
		let pretty = if (-0.005..0.005).contains(&value) {
			"0".to_string() // avoid cursed "-0"
		} else {
			let s = format!("{value:.2}");
			s.trim_end_matches('0').trim_end_matches('.').to_string()
		};

		text.set_text(common, Translation::from_raw_text(&pretty));
	}

	fn set_value(&mut self, common: &mut CallbackDataCommon, data: &Data, index: ValueIndex, new_value: f32) {
		let val1 = self.value1.get();
		let val2 = self.value2.as_ref().map_or(f32::MAX, Value::get);

		let Some(value) = (match index {
			ValueIndex::Primary => Some(&mut self.value1),
			ValueIndex::Secondary => self.value2.as_mut(),
		}) else {
			return;
		};

		// Slider handle widget
		let Some(handle_data) = (match index {
			ValueIndex::Primary => Some(&data.handle1),
			ValueIndex::Secondary => data.handle2.as_ref(),
		}) else {
			unreachable!();
		};

		let before = value.get();

		if index == ValueIndex::Secondary {
			value.set(&self.limits, new_value.max(val1 + self.limits.step));
		} else {
			value.set(&self.limits, new_value.min(val2 - self.limits.step));
		}

		let has_changed = value.get() != before;

		let Some(slider_handle_node_id) = common.state.nodes.get(handle_data.id_handle) else {
			return;
		};

		let Ok(style) = common.state.tree.style(*slider_handle_node_id) else {
			return;
		};

		if !conf_handle_style(
			common.alterables,
			&self.limits,
			value.get(),
			handle_data.id_handle,
			data.body_node,
			style,
			&common.state.tree,
		) {
			return; // nothing changed visually
		}

		common.alterables.mark_dirty(handle_data.id_handle);
		common.alterables.mark_redraw();

		if let Some(id_text) = handle_data.id_text
			&& let Some(mut label) = common.state.widgets.get_as::<WidgetLabel>(id_text)
		{
			Self::update_text(common, &mut label, value.get());
		}

		if has_changed && let Some(on_value_changed) = &self.on_value_changed {
			on_value_changed(
				common,
				SliderValueChangedEvent {
					index,
					value: value.get(),
				},
			);
		}
	}
}

const BODY_COLOR: drawing::Color = drawing::Color::new(0.6, 0.65, 0.7, 0.1);
const BODY_BORDER_COLOR: drawing::Color = drawing::Color::new(0.4, 0.45, 0.5, 0.6);
const HANDLE_BORDER_COLOR: drawing::Color = drawing::Color::new(0.85, 0.85, 0.85, 1.0);
const HANDLE_BORDER_COLOR_HOVERED: drawing::Color = drawing::Color::new(0.0, 0.0, 0.0, 1.0);
const HANDLE_COLOR: drawing::Color = drawing::Color::new(1.0, 1.0, 1.0, 1.0);
const HANDLE_COLOR_HOVERED: drawing::Color = drawing::Color::new(0.9, 0.9, 0.9, 1.0);

const SLIDER_HOVER_SCALE: f32 = 0.25;
fn get_anim_transform(pos: f32, widget_size: Vec2) -> Mat4 {
	util::centered_matrix(
		widget_size,
		&Mat4::from_scale(Vec3::splat(SLIDER_HOVER_SCALE.mul_add(pos, 1.0))),
	)
}

fn anim_rect(rect: &mut WidgetRectangle, pos: f32) {
	rect.params.color = drawing::Color::lerp(&HANDLE_COLOR, &HANDLE_COLOR_HOVERED, pos);
	rect.params.border_color = drawing::Color::lerp(&HANDLE_BORDER_COLOR, &HANDLE_BORDER_COLOR_HOVERED, pos);
}

fn on_enter_anim(common: &mut event::CallbackDataCommon, handle_id: WidgetID, anim_mult: f32) {
	common.alterables.animate(Animation::new(
		handle_id,
		(20. * anim_mult) as _,
		AnimationEasing::OutBack,
		Box::new(move |common, data| {
			let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap();
			data.data.transform = get_anim_transform(data.pos, data.widget_boundary.size);
			anim_rect(rect, data.pos);
			common.alterables.mark_redraw();
		}),
	));
}

fn on_leave_anim(common: &mut event::CallbackDataCommon, handle_id: WidgetID, anim_mult: f32) {
	common.alterables.animate(Animation::new(
		handle_id,
		(10. * anim_mult) as _,
		AnimationEasing::OutQuad,
		Box::new(move |common, data| {
			let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap();
			data.data.transform = get_anim_transform(1.0 - data.pos, data.widget_boundary.size);
			anim_rect(rect, 1.0 - data.pos);
			common.alterables.mark_redraw();
		}),
	));
}

fn register_event_mouse_enter(
	state: Rc<RefCell<State>>,
	listeners: &mut EventListenerCollection,
	tooltip_info: Option<tooltip::TooltipInfo>,
) -> event::EventListenerID {
	listeners.register(
		EventListenerKind::MouseEnter,
		Box::new(move |common, event_data, (), ()| {
			common.alterables.trigger_haptics();
			state.borrow_mut().hovered_body = true;

			ComponentTooltip::register_hover_in(common, &tooltip_info, event_data.widget_id, state.clone());

			Ok(EventResult::Pass)
		}),
	)
}

fn register_event_mouse_leave(
	state: Rc<RefCell<State>>,
	listeners: &mut EventListenerCollection,
) -> event::EventListenerID {
	listeners.register(
		EventListenerKind::MouseLeave,
		Box::new(move |common, _data, (), ()| {
			common.alterables.trigger_haptics();

			{
				let mut state = state.borrow_mut();
				state.hovered_body = false;
				state.active_tooltip = None;
			}

			Ok(EventResult::Pass)
		}),
	)
}

fn get_handle_dist(common: &mut CallbackDataCommon, handle: &SliderHandleData, mouse_pos: Vec2) -> f32 {
	let center = common.state.get_widget_boundary(handle.id_handle).center();
	Vec2::distance(center, mouse_pos)
}

const MAX_HOVER_DIST: f32 = 64.0;

fn update_handle_hovers(
	common: &mut CallbackDataCommon,
	data: &Data,
	state: &mut State,
	anim_mult: f32,
	mouse_pos: Vec2,
) {
	let hovered1_prev = state.hovered1;
	let hovered2_prev = state.hovered2;

	if state.hovered_body {
		let dist1 = get_handle_dist(common, &data.handle1, mouse_pos);

		let dist2 = data
			.handle2
			.as_ref()
			.map_or(f32::MAX, |h| get_handle_dist(common, h, mouse_pos));

		state.hovered1 = dist1 <= MAX_HOVER_DIST;
		state.hovered2 = dist2 <= MAX_HOVER_DIST;

		// both of them are hovered, hover the closest one
		if state.hovered1 && state.hovered2 {
			if dist1 < dist2 {
				state.hovered2 = false;
			} else {
				state.hovered1 = false;
			}
		}
	} else {
		state.hovered1 = false;
		state.hovered2 = false;
	}

	// hover state changed, run animations
	if state.hovered1 != hovered1_prev {
		if state.hovered1 && !hovered1_prev {
			on_enter_anim(common, data.handle1.id_handle_rect, anim_mult);
		} else {
			on_leave_anim(common, data.handle1.id_handle_rect, anim_mult);
		}
	}

	if state.hovered2 != hovered2_prev
		&& let Some(handle2) = data.handle2.as_ref()
	{
		if state.hovered2 && !hovered2_prev {
			on_enter_anim(common, handle2.id_handle_rect, anim_mult);
		} else {
			on_leave_anim(common, handle2.id_handle_rect, anim_mult);
		}
	}
}

fn register_event_mouse_motion(
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
	listeners: &mut EventListenerCollection,
	anim_mult: f32,
) -> event::EventListenerID {
	listeners.register(
		EventListenerKind::MouseMotion,
		Box::new(move |common, event_data, (), ()| {
			let mut state = state.borrow_mut();

			let Some(pos_relative) = event_data
				.metadata
				.get_mouse_pos_relative(&common.alterables.transform_stack)
			else {
				unreachable!();
			};

			let CallbackMetadata::MousePosition(pos) = &event_data.metadata else {
				unreachable!();
			};

			update_handle_hovers(common, &data, &mut state, anim_mult, pos_relative);

			if let Some(dragged_by) = &state.dragged_by
				&& dragged_by.device == pos.device
			{
				let index = dragged_by.index;
				state.update_value_to_mouse(event_data, &data, common, index);
				return Ok(EventResult::Consumed);
			}

			Ok(EventResult::Pass)
		}),
	)
}

fn register_event_mouse_press(
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
	listeners: &mut EventListenerCollection,
) -> event::EventListenerID {
	listeners.register(
		EventListenerKind::MousePress,
		Box::new(move |common, event_data, (), ()| {
			common.alterables.trigger_haptics();
			common.alterables.unfocus();
			let mut state = state.borrow_mut();

			let CallbackMetadata::MouseButton(btn) = event_data.metadata else {
				unreachable!();
			};

			if !state.hovered_body {
				// this slider isn't hovered at all?
				return Ok(EventResult::Pass);
			}

			let hovered_index = state.get_hovered_index().unwrap_or(ValueIndex::Primary);
			state.dragged_by = Some(DraggedBy {
				device: btn.device,
				index: hovered_index,
			});
			state.update_value_to_mouse(event_data, &data, common, hovered_index);
			Ok(EventResult::ConsumedExclusive)
		}),
	)
}

fn register_event_mouse_release(
	state: Rc<RefCell<State>>,
	listeners: &mut EventListenerCollection,
) -> event::EventListenerID {
	listeners.register(
		EventListenerKind::MouseRelease,
		Box::new(move |common, _data, (), ()| {
			common.alterables.trigger_haptics();

			let mut state = state.borrow_mut();
			if state.dragged_by.is_some() {
				state.dragged_by = None;
				Ok(EventResult::Consumed)
			} else {
				Ok(EventResult::Pass)
			}
		}),
	)
}

fn mount_slider_handle(
	ess: &mut ConstructEssentials,
	body_id: WidgetID,
	show_value: bool,
) -> anyhow::Result<SliderHandleData> {
	let slider_handle_style = taffy::Style {
		size: taffy::Size {
			width: length(0.0),
			height: percent(1.0),
		},
		position: taffy::Position::Absolute,
		align_items: Some(taffy::AlignItems::Center),
		justify_content: Some(taffy::JustifyContent::Center),
		..Default::default()
	};

	// invisible outer handle body
	let (slider_handle, _) = ess
		.layout
		.add_child(body_id, WidgetDiv::create(), slider_handle_style)?;

	let (slider_handle_rect, _) = ess.layout.add_child(
		slider_handle.id,
		WidgetRectangle::create(WidgetRectangleParams {
			color: HANDLE_COLOR,
			border_color: HANDLE_BORDER_COLOR,
			border: 2.0,
			round: WLength::Percent(1.0),
			..Default::default()
		}),
		taffy::Style {
			position: taffy::Position::Absolute,
			size: taffy::Size {
				width: length(HANDLE_WIDTH),
				height: length(HANDLE_HEIGHT),
			},
			..Default::default()
		},
	)?;

	let slider_text: Option<(WidgetPair, taffy::NodeId)> = if show_value {
		let label = WidgetLabel::create(
			&mut ess.layout.state,
			WidgetLabelParams {
				content: Translation::default(),
				style: TextStyle {
					color: Some(drawing::Color::new(0.0, 0.0, 0.0, 1.0)), // always black
					weight: Some(FontWeight::Bold),
					align: Some(HorizontalAlign::Center),
					..Default::default()
				},
			},
		);
		Some(ess.layout.add_child(slider_handle.id, label, Default::default())?)
	} else {
		None
	};

	Ok(SliderHandleData {
		id_handle_rect: slider_handle_rect.id,
		id_text: slider_text.map(|s| s.0.id),
		id_handle: slider_handle.id,
	})
}

pub fn construct(ess: &mut ConstructEssentials, params: Params) -> anyhow::Result<(WidgetPair, Rc<ComponentSlider>)> {
	let mut style = params.style;
	style.position = taffy::Position::Relative;
	style.min_size = style.size;
	style.max_size = style.size;

	let (root, slider_body_node) = ess.layout.add_child(ess.parent, WidgetDiv::create(), style)?;
	let body_id = root.id;

	let (_background_id, _) = ess.layout.add_child(
		body_id,
		WidgetRectangle::create(WidgetRectangleParams {
			color: BODY_COLOR,
			round: WLength::Percent(1.0),
			border_color: BODY_BORDER_COLOR,
			border: 2.0,
			..Default::default()
		}),
		taffy::Style {
			size: taffy::Size {
				width: percent(1.0),
				height: percent(PAD_PERCENT),
			},
			position: taffy::Position::Absolute,
			align_self: Some(taffy::AlignItems::Center),
			justify_self: Some(taffy::JustifySelf::Center),
			..Default::default()
		},
	)?;

	let slider_handle1 = mount_slider_handle(ess, body_id, params.show_value)?;
	let slider_handle2 = if params.value2.is_some() {
		Some(mount_slider_handle(ess, body_id, params.show_value)?)
	} else {
		None
	};

	let state = State {
		dragged_by: None,
		hovered_body: false,
		hovered1: false,
		hovered2: false,
		value1: params.value1,
		value2: params.value2,
		limits: params.limits,
		on_value_changed: None,
		active_tooltip: None,
	};

	let data = Rc::new(Data {
		body_node: slider_body_node,
		handle1: slider_handle1,
		handle2: slider_handle2,
	});

	let state = Rc::new(RefCell::new(state));

	let base = ComponentBase {
		id: root.id,
		lhandles: {
			let listeners = &mut root.widget.state().event_listeners;
			let anim_mult = ess.layout.state.theme.animation_mult;
			vec![
				register_event_mouse_enter(state.clone(), listeners, params.tooltip),
				register_event_mouse_leave(state.clone(), listeners),
				register_event_mouse_motion(data.clone(), state.clone(), listeners, anim_mult),
				register_event_mouse_press(data.clone(), state.clone(), listeners),
				register_event_mouse_release(state.clone(), listeners),
			]
		},
	};

	let slider = Rc::new(ComponentSlider { base, data, state });

	ess.layout.register_component_refresh(&Component(slider.clone()));
	Ok((root, slider))
}
