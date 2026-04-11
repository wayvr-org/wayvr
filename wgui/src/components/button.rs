use crate::{
	animation::{Animation, AnimationEasing},
	assets::AssetPath,
	components::{
		self, Component, ComponentBase, ComponentTrait, RefreshData,
		tooltip::{ComponentTooltip, TooltipTrait},
	},
	drawing::{self, Boundary, Color},
	event::{CallbackDataCommon, EventListenerCollection, EventListenerID, EventListenerKind},
	i18n::Translation,
	layout::{WidgetID, WidgetPair},
	renderer_vk::{
		text::{FontWeight, TextStyle, custom_glyph::CustomGlyphData},
		util::centered_matrix,
	},
	sound::WguiSoundType,
	widget::{
		self, ConstructEssentials, EventResult, WidgetData,
		label::{WidgetLabel, WidgetLabelParams},
		rectangle::{WidgetRectangle, WidgetRectangleParams},
		sprite::{WidgetSprite, WidgetSpriteParams},
		util::WLength,
	},
};
use glam::{Mat4, Vec2, Vec3};
use std::{
	cell::RefCell,
	rc::Rc,
	time::{Duration, Instant},
};
use taffy::{AlignItems, JustifyContent, prelude::length};

pub struct Params<'a> {
	pub text: Option<Translation>, // if unset, label will not be populated
	pub sprite_src: Option<AssetPath<'a>>,
	pub color: Option<drawing::Color>,
	pub border: f32,
	pub border_color: Option<drawing::Color>,
	pub hover_border_color: Option<drawing::Color>,
	pub hover_color: Option<drawing::Color>,
	pub round: WLength,
	pub style: taffy::Style,
	pub text_style: TextStyle,
	pub tooltip: Option<components::tooltip::TooltipInfo>,
	/// make this a toggle-style button that stays depressed
	/// until "un-clicked". this is visual only.
	/// set the initial state using `set_sticky_state`
	pub sticky: bool,
	pub long_press_time: f32,
}

impl Default for Params<'_> {
	fn default() -> Self {
		Self {
			text: Some(Translation::from_raw_text("")),
			sprite_src: None,
			color: None,
			hover_color: None,
			border_color: None,
			border: 2.0,
			hover_border_color: None,
			round: WLength::Units(4.0),
			style: Default::default(),
			text_style: TextStyle::default(),
			tooltip: None,
			sticky: false,
			long_press_time: 0.0,
		}
	}
}

#[derive(Clone)]
pub struct ButtonClickEvent {
	pub mouse_pos_absolute: Option<Vec2>,
	pub boundary: Boundary,
}
pub type ButtonClickCallback = Rc<dyn Fn(&mut CallbackDataCommon, ButtonClickEvent) -> anyhow::Result<()>>;

pub struct Colors {
	pub color: drawing::Color,
	pub border_color: drawing::Color,
	pub hover_color: drawing::Color,
	pub hover_border_color: drawing::Color,
}

struct State {
	hovered: bool,
	down: bool,
	sticky_down: bool,
	on_click: Option<ButtonClickCallback>,
	active_tooltip: Option<Rc<ComponentTooltip>>,
	colors: Colors,
	last_pressed: Instant,
}

impl TooltipTrait for State {
	fn get(&mut self) -> &mut Option<Rc<ComponentTooltip>> {
		&mut self.active_tooltip
	}
}

struct Data {
	id_label: WidgetID, // Label
	id_rect: WidgetID,  // Rectangle
	sticky: bool,
}

pub struct ComponentButton {
	base: ComponentBase,
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
}

impl ComponentTrait for ComponentButton {
	fn base(&self) -> &ComponentBase {
		&self.base
	}

	fn base_mut(&mut self) -> &mut ComponentBase {
		&mut self.base
	}

	fn refresh(&self, data: &mut RefreshData) {
		let mut state = self.state.borrow_mut();

		if state.active_tooltip.is_some() {
			let l_state = &data.layout.state;
			if let Some(node_id) = l_state.nodes.get(self.base.get_id()) {
				if !widget::is_node_visible(&l_state.tree, *node_id) {
					state.active_tooltip = None; // destroy the tooltip, this button is now hidden
				}
			} else {
				debug_assert!(false);
			}
		}
	}
}

fn get_color2(color: &drawing::Color, gradient_intensity: f32) -> drawing::Color {
	color.lerp(&Color::new(0.0, 0.0, 0.0, color.a), gradient_intensity)
}

fn get_hover_color(color: &drawing::Color) -> drawing::Color {
	Color::new(color.r + 0.25, color.g + 0.25, color.g + 0.25, color.a + 0.15)
}

impl ComponentButton {
	pub fn get_label(&self) -> WidgetID {
		self.data.id_label
	}

	pub fn get_rect(&self) -> WidgetID {
		self.data.id_rect
	}

	pub fn set_text(&self, common: &mut CallbackDataCommon, text: Translation) {
		let Some(mut label) = common.state.widgets.get_as::<WidgetLabel>(self.data.id_label) else {
			return;
		};

		label.set_text(common, text);
	}

	pub fn set_color(&self, common: &mut CallbackDataCommon, color: Color) {
		let gradient_intensity = common.state.theme.gradient_intensity;

		let Some(mut rect) = common.state.widgets.get_as::<WidgetRectangle>(self.data.id_rect) else {
			return;
		};

		let mut state = self.state.borrow_mut();
		state.colors.color = color;
		state.colors.hover_color = get_hover_color(&color);
		rect.params.color = color;
		rect.params.color2 = get_color2(&color, gradient_intensity);
	}

	pub fn get_time_since_last_pressed(&self) -> Duration {
		self.state.borrow().last_pressed.elapsed()
	}

	pub fn on_click(&self, func: ButtonClickCallback) {
		self.state.borrow_mut().on_click = Some(func);
	}

	/// Sets the sticky state of the button.
	///
	/// On buttons where sticky is false, sticky state won't automatically clear.
	pub fn set_sticky_state(&self, common: &mut CallbackDataCommon, sticky_down: bool) {
		let mut state = self.state.borrow_mut();

		// only play anim if we're not changing the border highlight
		let dirty = !state.hovered && !state.down && state.sticky_down != sticky_down;

		state.sticky_down = sticky_down;

		if !dirty {
			return;
		}

		let (anim_mult, gradient_intensity) = {
			let theme = &common.state.theme;
			(theme.animation_mult, theme.gradient_intensity)
		};

		let anim_ticks = if sticky_down { 5. } else { 10. };

		let state = self.state.clone();
		let anim = Animation::new(
			self.data.id_rect,
			(anim_ticks * anim_mult) as _,
			AnimationEasing::OutCubic,
			Box::new(move |common, anim_data| {
				let rect = anim_data.obj.get_as_mut::<WidgetRectangle>().unwrap();
				let mult = if sticky_down {
					anim_data.pos
				} else {
					1.0 - anim_data.pos
				};

				let state = state.borrow();
				let colors = &state.colors;
				let bgcolor = colors.color.lerp(&colors.hover_color, mult * 0.5);
				rect.params.color = bgcolor;
				rect.params.color2 = get_color2(&bgcolor, gradient_intensity);
				rect.params.border_color = colors.border_color.lerp(&colors.hover_border_color, mult);
				common.alterables.mark_redraw();
			}),
		);

		common.alterables.animations.push(anim);
	}
}

#[allow(clippy::too_many_arguments)]
fn anim_hover(
	common: &mut CallbackDataCommon,
	rect: &mut WidgetRectangle,
	widget_data: &mut WidgetData,
	colors: &Colors,
	widget_boundary: Boundary,
	pos: f32,
	pressed: bool,
	sticky_down: bool,
) {
	let mult = pos * if pressed { 1.5 } else { 1.0 };

	let (init_border_color, init_color) = if sticky_down {
		(colors.hover_border_color, colors.color.lerp(&colors.hover_color, 0.5))
	} else {
		(colors.border_color, colors.color)
	};

	let bgcolor = init_color.lerp(&colors.hover_color, mult);

	let gradient_intensity = common.state.theme.gradient_intensity;

	//let t = Mat4::from_scale(Vec3::splat(1.0 + pos * 0.5)) * Mat4::from_rotation_z(pos * 1.0);

	let t = Mat4::from_scale(Vec3::splat(1.0 + pos * 0.05));
	widget_data.transform = centered_matrix(widget_boundary.size, &t);

	rect.params.color = bgcolor;
	rect.params.color2 = get_color2(&bgcolor, gradient_intensity);

	rect.params.border_color = init_border_color.lerp(&colors.hover_border_color, mult);
}

fn anim_hover_create(state: Rc<RefCell<State>>, widget_id: WidgetID, fade_in: bool, anim_mult: f32) -> Animation {
	Animation::new(
		widget_id,
		((if fade_in { 5. } else { 10. }) * anim_mult) as _,
		AnimationEasing::OutCubic,
		Box::new(move |common, anim_data| {
			let rect = anim_data.obj.get_as_mut::<WidgetRectangle>().unwrap();
			let state = state.borrow();
			anim_hover(
				common,
				rect,
				anim_data.data,
				&state.colors,
				anim_data.widget_boundary,
				if fade_in { anim_data.pos } else { 1.0 - anim_data.pos },
				state.down,
				state.sticky_down,
			);
			common.alterables.mark_redraw();
		}),
	)
}

fn register_event_mouse_enter(
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
	listeners: &mut EventListenerCollection,
	tooltip_info: Option<components::tooltip::TooltipInfo>,
	anim_mult: f32,
) -> EventListenerID {
	listeners.register(
		EventListenerKind::MouseEnter,
		Box::new(move |common, event_data, (), ()| {
			common.alterables.play_sound(WguiSoundType::ButtonMouseEnter);
			common.alterables.trigger_haptics();
			common.alterables.mark_redraw();
			common
				.alterables
				.animate(anim_hover_create(state.clone(), event_data.widget_id, true, anim_mult));

			ComponentTooltip::register_hover_in(common, &tooltip_info, data.id_rect, state.clone());

			state.borrow_mut().hovered = true;
			Ok(EventResult::Pass)
		}),
	)
}

fn register_event_mouse_leave(
	state: Rc<RefCell<State>>,
	listeners: &mut EventListenerCollection,
	anim_mult: f32,
) -> EventListenerID {
	listeners.register(
		EventListenerKind::MouseLeave,
		Box::new(move |common, event_data, (), ()| {
			common.alterables.trigger_haptics();
			common
				.alterables
				.animate(anim_hover_create(state.clone(), event_data.widget_id, false, anim_mult));
			let mut state = state.borrow_mut();
			state.active_tooltip = None;
			state.hovered = false;
			Ok(EventResult::Pass)
		}),
	)
}

fn register_event_mouse_press(state: Rc<RefCell<State>>, listeners: &mut EventListenerCollection) -> EventListenerID {
	listeners.register(
		EventListenerKind::MousePress,
		Box::new(move |common, event_data, (), ()| {
			let mut state = state.borrow_mut();

			let rect = event_data.obj.get_as_mut::<WidgetRectangle>().unwrap();
			anim_hover(
				common,
				rect,
				event_data.widget_data,
				&state.colors,
				common.state.get_node_boundary(event_data.node_id),
				1.0,
				true,
				state.sticky_down,
			);

			common.alterables.trigger_haptics();
			common.alterables.play_sound(WguiSoundType::ButtonPress);
			common.alterables.mark_redraw();
			common.alterables.unfocus();

			if state.hovered {
				state.down = true;
				state.last_pressed = Instant::now();
				state.active_tooltip = None;
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
		Box::new(move |common, event_data, (), ()| {
			let rect = event_data.obj.get_as_mut::<WidgetRectangle>().unwrap();
			let mut state = state.borrow_mut();
			if data.sticky {
				state.sticky_down = !state.sticky_down;
			}

			common.alterables.trigger_haptics();
			common.alterables.play_sound(WguiSoundType::ButtonRelease);
			common.alterables.mark_redraw();

			if state.down {
				state.down = false;
				if state.hovered {
					anim_hover(
						common,
						rect,
						event_data.widget_data,
						&state.colors,
						common.state.get_node_boundary(event_data.node_id),
						1.0,
						false,
						state.sticky_down,
					);

					if let Some(on_click) = state.on_click.clone() {
						let evt = ButtonClickEvent {
							mouse_pos_absolute: event_data.metadata.get_mouse_pos_absolute(),
							boundary: event_data.widget_data.cached_absolute_boundary,
						};

						common.alterables.dispatch(Box::new(move |common| {
							(*on_click)(common, evt)?;
							Ok(())
						}));
					}
				}
				Ok(EventResult::Consumed)
			} else {
				Ok(EventResult::Pass)
			}
		}),
	)
}

pub fn construct(ess: &mut ConstructEssentials, params: Params) -> anyhow::Result<(WidgetPair, Rc<ComponentButton>)> {
	let mut style = params.style;
	let theme = &ess.layout.state.theme;

	// force-override style
	style.align_items = Some(AlignItems::Center);
	if style.justify_content.is_none() {
		style.justify_content = Some(JustifyContent::Center);
	}
	style.overflow.x = taffy::Overflow::Hidden;
	style.overflow.y = taffy::Overflow::Hidden;

	// update colors to default ones if they are not specified
	let color = params.color.unwrap_or(theme.button_color);

	let border_color = params
		.border_color
		.unwrap_or_else(|| Color::new(color.r, color.g, color.b, color.a + 0.25));

	let hover_color = params.hover_color.unwrap_or_else(|| get_hover_color(&color));

	let hover_border_color = params
		.hover_border_color
		.unwrap_or_else(|| Color::new(color.r + 0.5, color.g + 0.5, color.g + 0.5, color.a + 0.5));

	let gradient_intensity = theme.gradient_intensity;

	let light_text = {
		let mult = if theme.dark_mode { color.a } else { 1.0 - color.a };
		(color.r + color.g + color.b) * mult < 1.5
	};

	let (root, _) = ess.layout.add_child(
		ess.parent,
		WidgetRectangle::create(WidgetRectangleParams {
			color,
			color2: get_color2(&color, gradient_intensity),
			gradient: drawing::GradientMode::Vertical,
			round: params.round,
			border_color,
			border: params.border,
		}),
		style,
	)?;

	let id_rect = root.id;

	let default_margin = taffy::Rect {
		top: length(4.0),
		bottom: length(4.0),
		left: length(4.0),
		right: length(4.0),
	};

	if let Some(sprite_path) = params.sprite_src {
		let sprite = WidgetSprite::create(WidgetSpriteParams {
			glyph_data: Some(CustomGlyphData::from_assets(&ess.layout.state.globals, sprite_path)?),
			..Default::default()
		});

		ess.layout.add_child(
			root.id,
			sprite,
			taffy::Style {
				min_size: taffy::Size {
					width: length(20.0),
					height: length(20.0),
				},
				margin: default_margin,
				..Default::default()
			},
		)?;
	}

	let id_label = if let Some(content) = params.text {
		let widget_label = WidgetLabel::create(
			&mut ess.layout.state,
			WidgetLabelParams {
				content,
				style: TextStyle {
					weight: Some(FontWeight::Bold),
					color: Some(if light_text {
						Color::new(1.0, 1.0, 1.0, 1.0)
					} else {
						Color::new(0.0, 0.0, 0.0, 1.0)
					}),
					..params.text_style
				},
			},
		);

		let (label, _node_label) = ess.layout.add_child(
			id_rect,
			widget_label,
			taffy::Style {
				margin: default_margin,
				..Default::default()
			},
		)?;
		label.id
	} else {
		WidgetID::default()
	};

	let data = Rc::new(Data {
		id_label,
		id_rect,
		sticky: params.sticky,
	});

	let state = Rc::new(RefCell::new(State {
		down: false,
		hovered: false,
		on_click: None,
		active_tooltip: None,
		sticky_down: false,
		last_pressed: Instant::now(),
		colors: Colors {
			color,
			border_color,
			hover_color,
			hover_border_color,
		},
	}));

	let base = ComponentBase {
		id: root.id,
		lhandles: {
			let listeners = &mut root.widget.state().event_listeners;
			let anim_mult = ess.layout.state.theme.animation_mult;
			vec![
				register_event_mouse_enter(data.clone(), state.clone(), listeners, params.tooltip, anim_mult),
				register_event_mouse_leave(state.clone(), listeners, anim_mult),
				register_event_mouse_press(state.clone(), listeners),
				register_event_mouse_release(data.clone(), state.clone(), listeners),
			]
		},
	};

	let button = Rc::new(ComponentButton { base, data, state });

	ess.layout.register_component_refresh(&Component(button.clone()));
	Ok((root, button))
}
