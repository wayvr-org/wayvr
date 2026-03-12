use std::{
	cell::{Ref, RefCell},
	rc::{Rc, Weak},
};

use glam::FloatExt;
use taffy::prelude::{auto, length, percent};

use crate::{
	animation::{Animation, AnimationEasing},
	components::{Component, ComponentBase, ComponentTrait, FocusChangeData, RefreshData},
	drawing::{self, Color},
	event::{self, CallbackDataCommon, CallbackMetadata, EventListenerCollection, EventListenerKind, StyleSetRequest},
	i18n::Translation,
	layout::{WidgetID, WidgetPair},
	renderer_vk::text::{TextShadow, TextStyle},
	widget::{
		ConstructEssentials, EventResult,
		div::WidgetDiv,
		label::{WidgetLabel, WidgetLabelParams},
		rectangle::{WidgetRectangle, WidgetRectangleParams},
		util::WLength,
	},
};

#[derive(Default)]
pub struct Params {
	pub style: taffy::Style,
	pub initial_text: String,
}

struct State {
	text: String,
	hovered: bool,
	focused: bool,
	focused_prev: bool,
	first_refresh: bool,
	self_ref: Weak<ComponentEditBox>,
}

#[allow(clippy::struct_field_names)]
struct Data {
	#[allow(dead_code)]
	id_rect_container: WidgetID,
	id_rect_bottom: WidgetID,
	id_rect_cursor: WidgetID,
	id_label: WidgetID,
}

pub struct ComponentEditBox {
	base: ComponentBase,
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
}

fn anim_bottom_rect(
	common: &mut CallbackDataCommon,
	accent_color: drawing::Color,
	id_rect: WidgetID,
	anim_mult: f32,
	focused: bool,
) {
	common.alterables.animate(Animation::new(
		id_rect,
		(10.0 * anim_mult) as _,
		AnimationEasing::OutQuad,
		{
			Box::new(move |common, data| {
				let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap();
				let pos_bidir = if focused { data.pos } else { 1.0 - data.pos };

				rect.set_color(
					common,
					accent_color.lerp(&drawing::Color::new(1.0, 1.0, 1.0, 1.0), pos_bidir),
				);

				common.alterables.set_style(
					data.widget_id,
					StyleSetRequest::Size(taffy::Size {
						width: percent(0.95.lerp(1.0, pos_bidir)),
						height: length(1.0 + pos_bidir),
					}),
				);

				common.alterables.set_style(
					data.widget_id,
					StyleSetRequest::Margin(taffy::Rect {
						bottom: length(pos_bidir),
						left: auto(),
						right: auto(),
						top: auto(),
					}),
				);

				common.alterables.mark_redraw();
			})
		},
	));
}

fn refresh_all(common: &mut CallbackDataCommon, data: &Data, state: &mut State) -> Option<()> {
	let theme = &common.state.theme;
	let editbox_color = theme.editbox_color;
	let anim_mult = theme.animation_mult;
	let accent_color = theme.accent_color;

	let (rect_color, border_color) = if state.focused {
		(editbox_color.add_rgb(0.15), editbox_color.add_rgb(0.15 + 0.25))
	} else if state.hovered {
		(editbox_color.add_rgb(0.1), editbox_color.add_rgb(0.1 + 0.15))
	} else {
		(editbox_color, editbox_color.add_rgb(0.15))
	};

	// update background color
	let mut rect = common.state.widgets.get_as::<WidgetRectangle>(data.id_rect_container)?;
	rect.params.border_color = border_color;
	rect.set_color(common, rect_color);

	if state.focused_prev != state.focused || state.first_refresh {
		anim_bottom_rect(common, accent_color, data.id_rect_bottom, anim_mult, state.focused);
		state.focused_prev = state.focused;
	}

	// Cursor
	common.alterables.set_style(
		data.id_rect_cursor,
		StyleSetRequest::Display(if state.focused {
			taffy::Display::Flex
		} else {
			taffy::Display::None
		}),
	);

	state.first_refresh = false;

	Some(())
}

impl ComponentTrait for ComponentEditBox {
	fn base(&self) -> &ComponentBase {
		&self.base
	}

	fn base_mut(&mut self) -> &mut ComponentBase {
		&mut self.base
	}

	fn refresh(&self, data: &mut RefreshData) {
		let mut state = self.state.borrow_mut();
		let res = refresh_all(data.common, &self.data, &mut state);
		debug_assert!(res.is_some());
	}

	fn on_focus_change(&self, data: &mut FocusChangeData) {
		let mut state = self.state.borrow_mut();
		state.focused = data.focused;
		data.common.alterables.refresh_component_once(&state.self_ref);
	}
}

fn update_text(common: &mut CallbackDataCommon, state: &mut State, data: &Data, text: String) {
	let Some(mut label) = common.state.widgets.get_as::<WidgetLabel>(data.id_label) else {
		return;
	};

	label.set_text(common, Translation::from_raw_text(&text));

	common.alterables.refresh_component_once(&state.self_ref);

	state.text = text;
}

impl ComponentEditBox {
	pub fn set_text(&self, common: &mut CallbackDataCommon, text: &str) {
		let mut state = self.state.borrow_mut();
		update_text(common, &mut state, &self.data, String::from(text));
	}

	pub fn get_text(&self) -> Ref<'_, String> {
		Ref::map(self.state.borrow(), |x| &x.text)
	}
}

fn register_event_text_input(
	state: Rc<RefCell<State>>,
	data: Rc<Data>,
	listeners: &mut EventListenerCollection,
) -> event::EventListenerID {
	listeners.register(
		EventListenerKind::TextInput,
		Box::new(move |common, evt, (), ()| {
			let mut state = state.borrow_mut();
			if !state.focused {
				return Ok(EventResult::Pass);
			}

			let CallbackMetadata::TextInput(input) = &evt.metadata else {
				unreachable!();
			};

			let Some(input_text) = &input.text else {
				return Ok(EventResult::Pass); // nothing to do
			};

			let Some(ch) = input_text.chars().next() else {
				return Ok(EventResult::Pass); // ???
			};

			let mut new_text = std::mem::take(&mut state.text);

			if ch == '\x08' {
				// Backspace
				new_text.pop();
			} else {
				let printable = !input_text.chars().any(char::is_control);
				if printable {
					new_text.push_str(input_text);
				}
			}

			update_text(common, &mut state, &data, new_text);

			Ok(EventResult::Consumed)
		}),
	)
}

fn register_event_mouse_enter(
	state: Rc<RefCell<State>>,
	listeners: &mut EventListenerCollection,
) -> event::EventListenerID {
	listeners.register(
		EventListenerKind::MouseEnter,
		Box::new(move |common, _evt, (), ()| {
			let mut state = state.borrow_mut();
			state.hovered = true;
			common.alterables.trigger_haptics();
			common.alterables.refresh_component_once(&state.self_ref);
			Ok(EventResult::Pass)
		}),
	)
}

fn register_event_mouse_press(
	state: Rc<RefCell<State>>,
	listeners: &mut EventListenerCollection,
) -> event::EventListenerID {
	listeners.register(
		EventListenerKind::MousePress,
		Box::new(move |common, _evt, (), ()| {
			let state = state.borrow_mut();
			common.alterables.focus(&state.self_ref);
			common.alterables.trigger_haptics();
			common.alterables.refresh_component_once(&state.self_ref);
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
		Box::new(move |common, _evt, (), ()| {
			let mut state = state.borrow_mut();
			state.hovered = false;
			common.alterables.trigger_haptics();
			common.alterables.refresh_component_once(&state.self_ref);
			Ok(EventResult::Pass)
		}),
	)
}

pub fn construct(
	ess: &mut ConstructEssentials,
	mut params: Params,
) -> anyhow::Result<(WidgetPair, Rc<ComponentEditBox>)> {
	let text_color = ess.layout.state.theme.text_color;

	if params.style.size.width.is_auto() {
		params.style.size.width = length(128.0);
	}

	if params.style.size.height.is_auto() {
		params.style.size.height = length(32.0);
	}

	// override style
	params.style.align_items = Some(taffy::AlignItems::Center);
	params.style.position = taffy::Position::Relative;
	params.style.overflow = taffy::Point {
		x: taffy::Overflow::Scroll,
		y: taffy::Overflow::Visible,
	};
	params.style.min_size = params.style.max_size;

	let (root, _) = ess.layout.add_child(
		ess.parent,
		WidgetRectangle::create(WidgetRectangleParams {
			border: 2.0,
			round: WLength::Units(3.0),
			..Default::default()
		}),
		params.style,
	)?;

	// for centering
	let (rect_bottom_parent, _) = ess.layout.add_child(
		root.id,
		WidgetDiv::create(),
		taffy::Style {
			position: taffy::Position::Absolute,
			flex_direction: taffy::FlexDirection::Column,
			align_content: Some(taffy::AlignContent::Center),
			align_items: Some(taffy::AlignItems::Center),
			size: taffy::Size {
				width: percent(1.0),
				height: percent(1.0),
			},
			..Default::default()
		},
	)?;

	let (rect_bottom, _) = ess.layout.add_child(
		rect_bottom_parent.id,
		WidgetRectangle::create(Default::default()),
		Default::default(),
	)?;

	let id_container = root.id;

	let (label_parent, _) = ess.layout.add_child(
		root.id,
		WidgetDiv::create(),
		taffy::Style {
			padding: taffy::Rect::length(8.0),
			..Default::default()
		},
	)?;

	let widget_label = WidgetLabel::create(
		&mut ess.layout.state,
		WidgetLabelParams {
			content: Translation::from_raw_text(&params.initial_text),
			style: TextStyle {
				shadow: Some(TextShadow {
					x: 1.0,
					y: 1.0,
					color: Color::new(0.0, 0.0, 0.0, 1.0),
				}),
				..Default::default()
			},
		},
	);
	let (label, _node_label) = ess.layout.add_child(
		label_parent.id,
		widget_label,
		taffy::Style {
			position: taffy::Position::Relative,
			..Default::default()
		},
	)?;

	let (rect_cursor, _) = ess.layout.add_child(
		label_parent.id,
		WidgetRectangle::create(WidgetRectangleParams {
			color: text_color.with_alpha(0.75),
			..Default::default()
		}),
		taffy::Style {
			align_self: Some(taffy::AlignSelf::Center),
			justify_self: Some(taffy::JustifySelf::End),
			min_size: taffy::Size {
				width: length(2.0),
				height: length(16.0),
			},
			..Default::default()
		},
	)?;

	let data = Rc::new(Data {
		id_rect_container: id_container,
		id_label: label.id,
		id_rect_bottom: rect_bottom.id,
		id_rect_cursor: rect_cursor.id,
	});

	let state = Rc::new(RefCell::new(State {
		self_ref: Weak::new(),
		text: params.initial_text,
		hovered: false,
		focused: false,
		focused_prev: false,
		first_refresh: true,
	}));

	let base = ComponentBase {
		id: root.id,
		lhandles: {
			let mut root_state = root.widget.state();
			vec![
				register_event_mouse_enter(state.clone(), &mut root_state.event_listeners),
				register_event_mouse_leave(state.clone(), &mut root_state.event_listeners),
				register_event_mouse_press(state.clone(), &mut root_state.event_listeners),
				register_event_text_input(state.clone(), data.clone(), &mut root_state.event_listeners),
			]
		},
	};

	let editbox = Rc::new(ComponentEditBox { base, data, state });
	editbox.state.borrow_mut().self_ref = Rc::downgrade(&editbox);

	ess.layout.defer_component_refresh(Component(editbox.clone()));
	Ok((root, editbox))
}
