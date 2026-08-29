use std::{
	cell::RefCell,
	rc::{Rc, Weak},
};
use taffy::{
	AlignItems,
	prelude::{length, percent},
};

use crate::{
	animation::{Animation, AnimationDuration, AnimationEasing},
	color::{WguiColor, WguiColorName},
	components::{
		Component, ComponentBase, ComponentTrait, DestroyData, RefreshData,
		radio_group::ComponentRadioGroup,
		tooltip::{self, ComponentTooltip, TooltipTrait},
	},
	drawing,
	event::{CallbackDataCommon, EventListenerCollection, EventListenerID, EventListenerKind},
	i18n::Translation,
	layout::{self, WidgetID, WidgetPair},
	renderer_vk::text::{FontWeight, TextStyle},
	sound::WguiSoundType,
	widget::{
		ConstructEssentials, EventResult,
		label::{WidgetLabel, WidgetLabelParams},
		rectangle::{WidgetRectangle, WidgetRectangleParams},
		util::WLength,
	},
};

pub struct Params {
	pub text: Translation,
	pub style: taffy::Style,
	pub color_checked: Option<WguiColor>,
	pub box_size: f32,
	pub checked: bool,
	pub radio_group: Option<Rc<ComponentRadioGroup>>,
	pub value: Option<Rc<str>>,
	pub tooltip: Option<tooltip::TooltipInfo>,
}

impl Default for Params {
	fn default() -> Self {
		Self {
			text: Translation::from_raw_text(""),
			style: Default::default(),
			color_checked: None,
			box_size: 24.0,
			checked: false,
			radio_group: None,
			value: None,
			tooltip: None,
		}
	}
}

pub struct CheckboxToggleEvent {
	pub checked: bool,
	pub value: Option<Rc<str>>,
}

pub type CheckboxToggleCallback = Box<dyn Fn(&mut CallbackDataCommon, CheckboxToggleEvent) -> anyhow::Result<()>>;

struct State {
	checked: bool,
	hovered: bool,
	down: bool,
	on_toggle: Option<CheckboxToggleCallback>,
	self_ref: Weak<ComponentCheckbox>,
	active_tooltip: Option<Rc<ComponentTooltip>>,
}

impl TooltipTrait for State {
	fn get(&mut self) -> &mut Option<Rc<ComponentTooltip>> {
		&mut self.active_tooltip
	}
}

#[allow(clippy::struct_field_names)]
struct Data {
	#[allow(dead_code)]
	id_container: WidgetID, // Rectangle, transparent if not hovered
	id_outer_box: WidgetID, // Rectangle, has the border
	id_inner_box: WidgetID, // Rectangle, parent of outer_box
	id_label: WidgetID,     // Label, parent of container
	value: Option<Rc<str>>, // arbitrary value assigned to the element
	radio_group: Option<Weak<ComponentRadioGroup>>,

	color_checked: WguiColor,
}

pub struct ComponentCheckbox {
	base: ComponentBase,
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
}

const COLOR_UNCHECKED: WguiColor = WguiColor::Raw(drawing::Color::new(0., 0., 0., 0.));
const COLOR_HOVERED: WguiColor = WguiColorName::Tertiary.to_wgui_color();

impl ComponentTrait for ComponentCheckbox {
	fn base(&self) -> &ComponentBase {
		&self.base
	}

	fn base_mut(&mut self) -> &mut ComponentBase {
		&mut self.base
	}

	fn refresh(&self, _data: &mut RefreshData) {
		// nothing to do
	}

	fn destroy(&self, data: &mut DestroyData) {
		if let Some(comp) = self.state.borrow_mut().active_tooltip.take() {
			comp.destroy(data);
			data.destroy_widgets.push(comp.base().id);
		}
	}
}

fn set_box_checked(widgets: &layout::WidgetMap, data: &Data, checked: bool, hovered: bool) {
	widgets.call(data.id_inner_box, |rect: &mut WidgetRectangle| {
		rect.params.color = if checked {
			if hovered { COLOR_HOVERED } else { data.color_checked }
		} else {
			COLOR_UNCHECKED
		}
	});
}

impl ComponentCheckbox {
	pub fn set_text(&self, common: &mut CallbackDataCommon, text: Translation) {
		let Some(mut label) = common.state.widgets.get_as::<WidgetLabel>(self.data.id_label) else {
			return;
		};

		label.set_text(common, text);
	}

	pub fn set_checked(&self, common: &mut CallbackDataCommon, checked: bool) {
		let hovered;
		{
			let mut state = self.state.borrow_mut();
			if state.checked == checked {
				return;
			}
			state.checked = checked;
			hovered = state.hovered;
		}
		set_box_checked(&common.state.widgets, &self.data, checked, hovered);
		common.alterables.mark_redraw();
	}

	pub fn get_checked(&self) -> bool {
		let state = self.state.borrow_mut();
		state.checked
	}

	pub fn get_value(&self) -> Option<Rc<str>> {
		self.data.value.clone()
	}

	/// Set checked state without triggering visual changes.
	pub(super) fn set_checked_internal(&self, checked: bool) {
		self.state.borrow_mut().checked = checked;
	}

	pub fn on_toggle(&self, func: CheckboxToggleCallback) {
		self.state.borrow_mut().on_toggle = Some(func);
	}
}

fn anim_hover(anim_data: &mut crate::animation::CallbackData<'_>, pos: f32, _pressed: bool) {
	let rect = anim_data.obj.as_any_mut().downcast_mut::<WidgetRectangle>().unwrap();
	rect.params.border = 2.0;
	rect.params.border_color = if pos > 0.0 {
		COLOR_HOVERED
	} else {
		WguiColorName::OnBackground.into()
	};
}

fn anim_hover_in(state: &Rc<RefCell<State>>, data: &Rc<Data>) -> Animation {
	let down = state.borrow().down;
	Animation::new(
		data.id_outer_box,
		AnimationDuration::Seconds(0.0833),
		AnimationEasing::OutQuad,
		Box::new(move |common, anim_data| {
			anim_hover(anim_data, anim_data.pos, down);
			common.alterables.mark_redraw();
		}),
	)
}

fn anim_hover_out(state: &Rc<RefCell<State>>, data: &Rc<Data>) -> Animation {
	let down = state.borrow().down;
	Animation::new(
		data.id_outer_box,
		AnimationDuration::Seconds(0.0833),
		AnimationEasing::OutQuad,
		Box::new(move |common, anim_data| {
			anim_hover(anim_data, 1.0 - anim_data.pos, down);
			common.alterables.mark_redraw();
		}),
	)
}

fn register_event_mouse_enter(
	state: Rc<RefCell<State>>,
	data: Rc<Data>,
	listeners: &mut EventListenerCollection,
	tooltip_info: Option<tooltip::TooltipInfo>,
) -> EventListenerID {
	listeners.register(
		EventListenerKind::MouseEnter,
		Box::new(move |common, _event_data, (), ()| {
			common.alterables.trigger_haptics();
			common.alterables.animate(anim_hover_in(&state, &data));

			ComponentTooltip::register_hover_in(common, &tooltip_info, data.id_container, state.clone());

			let checked = {
				let mut state = state.borrow_mut();
				state.hovered = true;
				state.checked
			};

			if checked {
				common
					.state
					.widgets
					.call(data.id_inner_box, |rect: &mut WidgetRectangle| {
						rect.params.color = COLOR_HOVERED;
					});
			}

			Ok(EventResult::Pass)
		}),
	)
}

fn register_event_mouse_leave(
	state: Rc<RefCell<State>>,
	data: Rc<Data>,
	listeners: &mut EventListenerCollection,
) -> EventListenerID {
	listeners.register(
		EventListenerKind::MouseLeave,
		Box::new(move |common, _event_data, (), ()| {
			common.alterables.trigger_haptics();
			common.alterables.animate(anim_hover_out(&state, &data));

			let checked = {
				let mut state = state.borrow_mut();
				state.hovered = false;
				state.active_tooltip = None;
				state.checked
			};

			if checked {
				common
					.state
					.widgets
					.call(data.id_inner_box, |rect: &mut WidgetRectangle| {
						rect.params.color = data.color_checked;
					});
			}

			Ok(EventResult::Pass)
		}),
	)
}

fn register_event_mouse_cancel(state: Rc<RefCell<State>>, listeners: &mut EventListenerCollection) -> EventListenerID {
	listeners.register(
		EventListenerKind::MouseCancel,
		Box::new(move |_common, _event_data, (), ()| {
			let mut state = state.borrow_mut();
			state.down = false;
			Ok(EventResult::Pass)
		}),
	)
}

fn register_event_mouse_press(
	state: Rc<RefCell<State>>,
	data: Rc<Data>,
	listeners: &mut EventListenerCollection,
) -> EventListenerID {
	listeners.register(
		EventListenerKind::MousePress,
		Box::new(move |common, _event_data, (), ()| {
			let mut state = state.borrow_mut();
			let pressed_hovered = state.hovered;

			common
				.state
				.widgets
				.call(data.id_outer_box, |rect: &mut WidgetRectangle| {
					rect.params.border = 2.0;
					rect.params.border_color = if pressed_hovered {
						COLOR_HOVERED
					} else {
						WguiColorName::OnBackground.into()
					};
				});

			common.alterables.trigger_haptics();
			common.alterables.mark_redraw();
			common.alterables.unfocus();

			if state.hovered {
				state.down = true;
				Ok(EventResult::Consumed)
			} else {
				Ok(EventResult::Pass)
			}
		}),
	)
}

fn register_event_mouse_release(
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
	listeners: &mut EventListenerCollection,
) -> EventListenerID {
	listeners.register(
		EventListenerKind::MouseRelease,
		Box::new(move |common, _event_data, (), ()| {
			let mut state = state.borrow_mut();
			let released_hovered = state.hovered;
			let was_down = state.down;

			common
				.state
				.widgets
				.call(data.id_outer_box, |rect: &mut WidgetRectangle| {
					rect.params.border = 2.0;
					rect.params.border_color = if released_hovered {
						COLOR_HOVERED
					} else {
						WguiColorName::OnBackground.into()
					};
				});

			common.alterables.trigger_haptics();
			common.alterables.mark_redraw();

			if was_down {
				state.down = false;

				if let Some(self_ref) = state.self_ref.upgrade()
					&& let Some(radio) = data.radio_group.as_ref().and_then(Weak::upgrade)
				{
					radio.set_selected_internal(common, &self_ref)?;
					state.checked = true; // can't uncheck radiobox by clicking the checked box again
					common.alterables.play_sound(WguiSoundType::CheckboxCheck);
				} else {
					state.checked = !state.checked;
					common.alterables.play_sound(if state.checked {
						WguiSoundType::CheckboxCheck
					} else {
						WguiSoundType::CheckboxUncheck
					});
				}

				set_box_checked(&common.state.widgets, &data, state.checked, state.hovered);
				if state.hovered
					&& let Some(on_toggle) = &state.on_toggle
				{
					on_toggle(
						common,
						CheckboxToggleEvent {
							checked: state.checked,
							value: data.value.clone(),
						},
					)?;
				}
				Ok(EventResult::Consumed)
			} else {
				Ok(EventResult::Pass)
			}
		}),
	)
}

pub fn construct(ess: &mut ConstructEssentials, params: Params) -> anyhow::Result<(WidgetPair, Rc<ComponentCheckbox>)> {
	let mut style = params.style;

	// force-override style
	style.flex_wrap = taffy::FlexWrap::NoWrap;
	style.align_items = Some(AlignItems::CENTER);

	// make checkbox interaction box larger by setting padding and negative margin
	style.padding = taffy::Rect {
		left: length(4.0_f32),
		right: length(8.0_f32),
		top: length(4.0_f32),
		bottom: length(4.0_f32),
	};

	style.margin = taffy::Rect {
		left: length(-4.0_f32),
		right: length(-8.0_f32),
		top: length(-4.0_f32),
		bottom: length(-4.0_f32),
	};
	//style.align_self = Some(taffy::AlignSelf::Start); // do not stretch self to the parent
	style.gap = length(4.0_f32);

	let (round_5, round_8) = if params.radio_group.is_some() {
		(WLength::Percent(1.0), WLength::Percent(1.0))
	} else {
		(WLength::Units(5.0), WLength::Units(8.0))
	};

	let color_checked = params.color_checked.unwrap_or_else(|| WguiColorName::Primary.into());

	let (root, _) = ess.layout.add_child(
		ess.parent,
		WidgetRectangle::create(WidgetRectangleParams {
			color: WguiColor::from(WguiColorName::OnPrimary).with_alpha(0.0),
			round: round_5,
			..Default::default()
		}),
		style,
	)?;

	let id_container = root.id;

	let box_size = taffy::Size {
		width: length(params.box_size),
		height: length(params.box_size),
	};

	let (outer_box, _) = ess.layout.add_child(
		id_container,
		WidgetRectangle::create(WidgetRectangleParams {
			border: 2.0,
			border_color: WguiColorName::OnBackground.into(),
			round: round_8,
			color: WguiColor::from(WguiColorName::OnPrimary).with_alpha(0.0),
			..Default::default()
		}),
		taffy::Style {
			size: box_size,
			padding: taffy::Rect::length(4.0_f32),
			min_size: box_size,
			max_size: box_size,
			..Default::default()
		},
	)?;

	let (inner_box, _) = ess.layout.add_child(
		outer_box.id,
		WidgetRectangle::create(WidgetRectangleParams {
			round: round_5,
			color: if params.checked { color_checked } else { COLOR_UNCHECKED },
			..Default::default()
		}),
		taffy::Style {
			size: taffy::Size {
				width: percent(1.0_f32),
				height: percent(1.0_f32),
			},
			..Default::default()
		},
	)?;

	let widget_label = WidgetLabel::create(
		&mut ess.layout.state,
		WidgetLabelParams {
			content: params.text,
			style: TextStyle {
				weight: Some(FontWeight::Bold),
				color: Some(WguiColorName::OnBackground.into()),
				..Default::default()
			},
			..Default::default()
		},
	);
	let (label, _node_label) = ess.layout.add_child(id_container, widget_label, Default::default())?;

	let data = Rc::new(Data {
		id_container,
		id_outer_box: outer_box.id,
		id_inner_box: inner_box.id,
		id_label: label.id,
		value: params.value,
		radio_group: params.radio_group.as_ref().map(Rc::downgrade),
		color_checked,
	});

	let state = Rc::new(RefCell::new(State {
		checked: params.checked,
		down: false,
		hovered: false,
		on_toggle: None,
		self_ref: Weak::new(),
		active_tooltip: None,
	}));

	let base = ComponentBase {
		id: root.id,
		lhandles: {
			let listeners = &mut root.widget.state().event_listeners;
			vec![
				register_event_mouse_enter(state.clone(), data.clone(), listeners, params.tooltip),
				register_event_mouse_leave(state.clone(), data.clone(), listeners),
				register_event_mouse_cancel(state.clone(), listeners),
				register_event_mouse_press(state.clone(), data.clone(), listeners),
				register_event_mouse_release(data.clone(), state.clone(), listeners),
			]
		},
	};

	let checkbox = Rc::new(ComponentCheckbox { base, data, state });

	if let Some(radio) = params.radio_group.as_ref() {
		radio.register_child(checkbox.clone(), params.checked);
		checkbox.state.borrow_mut().self_ref = Rc::downgrade(&checkbox);
	}

	ess.layout.defer_component_refresh(Component(checkbox.clone()));
	Ok((root, checkbox))
}
