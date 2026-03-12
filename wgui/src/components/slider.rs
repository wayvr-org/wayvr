use std::{cell::RefCell, rc::Rc};

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
		self, CallbackDataCommon, CallbackMetadata, EventAlterables, EventListenerCollection, EventListenerKind,
		StyleSetRequest,
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

#[derive(Default)]
pub struct ValuesMinMax {
	pub value: f32,
	pub min_value: f32,
	pub max_value: f32,
	pub step: f32,
}

impl ValuesMinMax {
	fn to_normalized(&self) -> f32 {
		(self.value - self.min_value) / (self.max_value - self.min_value)
	}

	fn get_from_normalized(&self, normalized: f32) -> f32 {
		normalized * (self.max_value - self.min_value) + self.min_value
	}

	fn set_value(&mut self, new_value: f32) -> &mut Self {
		let span = self.max_value - self.min_value;
		let clamped = new_value.max(self.min_value).min(self.max_value);

		// get the step index from min
		let mut k = ((clamped - self.min_value) / self.step).round();

		let k_max = (span / self.step).floor();
		if k < 0.0 {
			k = 0.0;
		}
		if k > k_max {
			k = k_max;
		}

		let snapped = self.min_value + k * self.step;
		self.value = snapped.max(self.min_value).min(self.max_value);

		self
	}
}

#[derive(Default)]
pub struct Params {
	pub style: taffy::Style,
	pub values: ValuesMinMax,
	pub show_value: bool,
	pub tooltip: Option<tooltip::TooltipInfo>,
}

struct State {
	dragged_by: Option<usize>,
	hovered: bool,
	values: ValuesMinMax,
	on_value_changed: Option<SliderValueChangedCallback>,
	active_tooltip: Option<Rc<ComponentTooltip>>,
}

impl TooltipTrait for State {
	fn get(&mut self) -> &mut Option<Rc<ComponentTooltip>> {
		&mut self.active_tooltip
	}
}

struct Data {
	body_node: taffy::NodeId,
	slider_handle_rect_id: WidgetID,  // Rectangle
	slider_text_id: Option<WidgetID>, // Text
	slider_handle_id: WidgetID,
}

pub struct SliderValueChangedEvent {
	pub value: f32,
}

pub type SliderValueChangedCallback =
	Box<dyn Fn(&mut CallbackDataCommon, SliderValueChangedEvent) -> anyhow::Result<()>>;

pub struct ComponentSlider {
	base: ComponentBase,
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
}

impl ComponentTrait for ComponentSlider {
	fn refresh(&self, init_data: &mut RefreshData) {
		let mut state = self.state.borrow_mut();
		let value = state.values.value;
		state.set_value(init_data.common, &self.data, value);
	}

	fn base(&self) -> &ComponentBase {
		&self.base
	}

	fn base_mut(&mut self) -> &mut ComponentBase {
		&mut self.base
	}
}

impl ComponentSlider {
	pub fn get_value(&self) -> f32 {
		self.state.borrow().values.value
	}

	pub fn set_value(&self, common: &mut CallbackDataCommon, value: f32) {
		let mut state = self.state.borrow_mut();
		state.set_value(common, &self.data, value);
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
	values: &ValuesMinMax,
	slider_handle_id: WidgetID,
	body_node: taffy::NodeId,
	slider_handle_style: &taffy::Style,
	tree: &taffy::tree::TaffyTree<WidgetID>,
) -> bool {
	/* returns false if nothing changed */
	let norm = values.to_normalized();

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
	fn update_value_to_mouse(
		&mut self,
		event_data: &event::CallbackData<'_>,
		data: &Data,
		common: &mut CallbackDataCommon,
	) {
		let mouse_pos = event_data
			.metadata
			.get_mouse_pos_relative(&common.alterables.transform_stack)
			.unwrap(); // safe

		let norm = map_mouse_x_to_normalized(
			mouse_pos.x - HANDLE_WIDTH / 2.0,
			get_width(data.body_node, &common.state.tree) - HANDLE_WIDTH,
		);

		let target_value = self.values.get_from_normalized(norm);
		let val = target_value;

		self.set_value(common, data, val);
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

	fn set_value(&mut self, common: &mut CallbackDataCommon, data: &Data, value: f32) {
		let before = self.values.value;
		self.values.set_value(value);

		let changed = self.values.value != before;

		let Some(slider_handle_node_id) = common.state.nodes.get(data.slider_handle_id) else {
			return;
		};

		let Ok(style) = common.state.tree.style(*slider_handle_node_id) else {
			return;
		};
		if !conf_handle_style(
			common.alterables,
			&self.values,
			data.slider_handle_id,
			data.body_node,
			style,
			&common.state.tree,
		) {
			return; // nothing changed visually
		}

		common.alterables.mark_dirty(data.slider_handle_id);
		common.alterables.mark_redraw();

		if let Some(slider_text_id) = data.slider_text_id
			&& let Some(mut label) = common.state.widgets.get_as::<WidgetLabel>(slider_text_id)
		{
			Self::update_text(common, &mut label, self.values.value);
		}

		if changed
			&& let Some(on_value_changed) = &self.on_value_changed
			&& let Err(e) = on_value_changed(
				common,
				SliderValueChangedEvent {
					value: self.values.value,
				},
			) {
			log::error!("{e:?}"); // FIXME: proper error handling
		}
	}
}

const BODY_COLOR: drawing::Color = drawing::Color::new(0.6, 0.65, 0.7, 0.2);
const BODY_BORDER_COLOR: drawing::Color = drawing::Color::new(0.4, 0.45, 0.5, 1.0);
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
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
	listeners: &mut EventListenerCollection,
	tooltip_info: Option<tooltip::TooltipInfo>,
	anim_mult: f32,
) -> event::EventListenerID {
	listeners.register(
		EventListenerKind::MouseEnter,
		Box::new(move |common, event_data, (), ()| {
			common.alterables.trigger_haptics();
			state.borrow_mut().hovered = true;
			on_enter_anim(common, data.slider_handle_rect_id, anim_mult);
			ComponentTooltip::register_hover_in(common, &tooltip_info, event_data.widget_id, state.clone());

			Ok(EventResult::Pass)
		}),
	)
}

fn register_event_mouse_leave(
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
	listeners: &mut EventListenerCollection,
	anim_mult: f32,
) -> event::EventListenerID {
	listeners.register(
		EventListenerKind::MouseLeave,
		Box::new(move |common, _data, (), ()| {
			common.alterables.trigger_haptics();

			{
				let mut state = state.borrow_mut();
				state.hovered = false;
				state.active_tooltip = None;
			}
			on_leave_anim(common, data.slider_handle_rect_id, anim_mult);

			Ok(EventResult::Pass)
		}),
	)
}

fn register_event_mouse_motion(
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
	listeners: &mut EventListenerCollection,
) -> event::EventListenerID {
	listeners.register(
		EventListenerKind::MouseMotion,
		Box::new(move |common, event_data, (), ()| {
			let mut state = state.borrow_mut();

			let CallbackMetadata::MousePosition(pos) = event_data.metadata else {
				unreachable!();
			};

			if state.dragged_by.is_some_and(|device| device == pos.device) {
				state.update_value_to_mouse(event_data, &data, common);
				Ok(EventResult::Consumed)
			} else {
				Ok(EventResult::Pass)
			}
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

			if state.hovered {
				state.dragged_by = Some(btn.device);
				state.update_value_to_mouse(event_data, &data, common);
				Ok(EventResult::Consumed)
			} else {
				Ok(EventResult::Pass)
			}
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

	let state = State {
		dragged_by: None,
		hovered: false,
		values: params.values,
		on_value_changed: None,
		active_tooltip: None,
	};

	let slider_text: Option<(WidgetPair, taffy::NodeId)> = if params.show_value {
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

	let data = Rc::new(Data {
		slider_handle_rect_id: slider_handle_rect.id,
		body_node: slider_body_node,
		slider_handle_id: slider_handle.id,
		slider_text_id: slider_text.map(|s| s.0.id),
	});

	let state = Rc::new(RefCell::new(state));

	let base = ComponentBase {
		id: root.id,
		lhandles: {
			let listeners = &mut root.widget.state().event_listeners;
			let anim_mult = ess.layout.state.theme.animation_mult;
			vec![
				register_event_mouse_enter(data.clone(), state.clone(), listeners, params.tooltip, anim_mult),
				register_event_mouse_leave(data.clone(), state.clone(), listeners, anim_mult),
				register_event_mouse_motion(data.clone(), state.clone(), listeners),
				register_event_mouse_press(data.clone(), state.clone(), listeners),
				register_event_mouse_release(state.clone(), listeners),
			]
		},
	};

	let slider = Rc::new(ComponentSlider { base, data, state });

	ess.layout.register_component_refresh(&Component(slider.clone()));
	Ok((root, slider))
}
