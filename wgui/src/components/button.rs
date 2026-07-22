use crate::{
	animation::{Animation, AnimationEasing},
	assets::AssetPath,
	color::{ParentColor, WguiColor, WguiColorName},
	components::{
		self, Component, ComponentBase, ComponentTrait, RefreshData,
		tooltip::{ComponentTooltip, TooltipTrait},
	},
	drawing::{self, Boundary},
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
use slotmap::Key;
use std::{
	cell::RefCell,
	rc::Rc,
	time::{Duration, Instant},
};
use taffy::{AlignItems, JustifyContent, prelude::length};

pub struct Params<'a> {
	pub text: Option<Translation>, // if unset, label will not be populated
	pub sprite_src: Option<AssetPath<'a>>,
	pub sprite_color: Option<WguiColor>,
	pub color: Option<WguiColor>,
	pub border: f32,
	pub border_color: Option<WguiColor>,
	pub hover_color: Option<WguiColor>,
	pub hover_border_color: Option<WguiColor>,
	pub sticky_color: Option<WguiColor>,
	pub sticky_border_color: Option<WguiColor>,
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
			sprite_color: None,
			text: Some(Translation::from_raw_text("")),
			sprite_src: None,
			color: None,
			border_color: None,
			hover_color: None,
			hover_border_color: None,
			sticky_color: None,
			sticky_border_color: None,
			border: 2.0,
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
	pub color: WguiColor,
	pub border_color: WguiColor,
	pub hover_color: WguiColor,
	pub hover_border_color: WguiColor,
	pub sticky_color: WguiColor,
	pub sticky_border_color: WguiColor,
}

struct State {
	hovered: bool,
	down: bool,
	sticky_down: bool,
	on_click: Option<ButtonClickCallback>,
	active_tooltip: Option<Rc<ComponentTooltip>>,
	colors: Colors,
	last_pressed: Instant,
	id_label: WidgetID,  // Label
	id_sprite: WidgetID, // Sprite
	children_discovered: bool,
}

impl TooltipTrait for State {
	fn get(&mut self) -> &mut Option<Rc<ComponentTooltip>> {
		&mut self.active_tooltip
	}
}

struct Data {
	id_rect: WidgetID, // Rectangle
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

		if !state.children_discovered {
			state.children_discovered = true;

			let mut children = vec![];
			data
				.layout
				.collect_children_ids_recursive(self.data.id_rect, &mut children);

			for child in children {
				if let Some(mut widget) = data.layout.state.widgets.get_as::<WidgetSprite>(child) {
					if !state.id_sprite.is_null() && state.id_sprite != child {
						log::error!("Button with more than one sprite!");
					}
					// apply initial color from button
					if let Some(apply_color) = color_to_apply(widget.parent_color(), state.colors.color) {
						let common = &mut CallbackDataCommon {
							state: &data.layout.state,
							alterables: &mut data.layout.alterables,
						};
						widget.set_color(common, apply_color);
					}
					state.id_sprite = child;
				} else if let Some(mut widget) = data.layout.state.widgets.get_as::<WidgetLabel>(child) {
					if !state.id_label.is_null() && state.id_label != child {
						log::error!("Button with more than one label!");
					}
					if let Some(apply_color) = color_to_apply(widget.parent_color(), state.colors.color) {
						let common = &mut CallbackDataCommon {
							state: &data.layout.state,
							alterables: &mut data.layout.alterables,
						};
						widget.set_color(common, apply_color, true);
					}
					state.id_label = child;
				}
			}
		}

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

	fn destroy(&self, data: &mut components::DestroyData) {
		if let Some(comp) = self.state.borrow_mut().active_tooltip.take() {
			comp.destroy(data);
			data.destroy_widgets.push(comp.base().id);
		}
	}
}

fn get_color2(color: &WguiColor, gradient_intensity: f32) -> WguiColor {
	color.mult_rgb(1.0 - gradient_intensity)
}

impl ComponentButton {
	pub fn get_rect(&self) -> WidgetID {
		self.data.id_rect
	}

	pub fn set_text(&self, common: &mut CallbackDataCommon, text: Translation) {
		let state = self.state.borrow();
		let Some(mut label) = common.state.widgets.get_as::<WidgetLabel>(state.id_label) else {
			return;
		};

		label.set_text(common, text);
	}

	pub fn set_color(&self, common: &mut CallbackDataCommon, color: WguiColor) {
		let gradient_intensity = common.state.theme.gradient_intensity;

		let Some(mut rect) = common.state.widgets.get_as::<WidgetRectangle>(self.data.id_rect) else {
			return;
		};
		rect.params.color = color;
		rect.params.color2 = get_color2(&color, gradient_intensity);

		let mut state = self.state.borrow_mut();
		state.colors.color = color;

		if let Some(mut label) = common.state.widgets.get_as::<WidgetLabel>(state.id_label)
			&& let Some(apply_color) = color_to_apply(label.parent_color(), color)
		{
			label.set_color(common, apply_color, true);
		}
		if let Some(mut sprite) = common.state.widgets.get_as::<WidgetSprite>(state.id_sprite)
			&& let Some(apply_color) = color_to_apply(sprite.parent_color(), color)
		{
			sprite.set_color(common, apply_color);
		}
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

				let (alt_color, alt_border_color) = if sticky_down {
					(&colors.sticky_color, &colors.sticky_border_color)
				} else {
					(&colors.hover_color, &colors.hover_border_color)
				};

				{
					let bg_color = colors.color.lerp(&common.globals().palette, alt_color, mult);

					rect.params.color = bg_color;
					rect.params.color2 = get_color2(&bg_color, gradient_intensity);
					rect.params.border_color = colors
						.border_color
						.lerp(&common.globals().palette, alt_border_color, mult);

					if let Some(fg_color0) = colors.color.fg_color()
						&& let Some(fg_color1) = alt_color.fg_color()
					{
						let fg_color = fg_color0.lerp(&common.globals().palette, &fg_color1, mult);
						if let Some(mut label) = common.state.widgets.get_as::<WidgetLabel>(state.id_label)
							&& let Some(apply) = color_to_apply2(label.parent_color(), bg_color, fg_color)
						{
							label.set_color(common, apply, true);
						}

						if let Some(mut sprite) = common.state.widgets.get_as::<WidgetSprite>(state.id_sprite)
							&& let Some(apply) = color_to_apply2(sprite.parent_color(), bg_color, fg_color)
						{
							sprite.set_color(common, apply);
						}
					}
				}
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
	label: WidgetID,
	sprite: WidgetID,
	widget_data: &mut WidgetData,
	colors: &Colors,
	widget_boundary: Boundary,
	pos: f32,
	pressed: bool,
	sticky_down: bool,
) {
	let mult = pos * if pressed { 1.5 } else { 1.0 };

	let (init_border_color, init_color) = if sticky_down {
		(colors.sticky_border_color, colors.sticky_color)
	} else {
		(colors.border_color, colors.color)
	};
	let bg_color = init_color.lerp(&common.globals().palette, &colors.hover_color, mult);

	if let Some(fg_color0) = init_color.fg_color()
		&& let Some(fg_color1) = colors.hover_color.fg_color()
	{
		let fg_color = fg_color0.lerp(&common.globals().palette, &fg_color1, mult);
		if let Some(mut label) = common.state.widgets.get_as::<WidgetLabel>(label)
			&& let Some(apply) = color_to_apply2(label.parent_color(), bg_color, fg_color)
		{
			label.set_color(common, apply, true);
		}

		if let Some(mut sprite) = common.state.widgets.get_as::<WidgetSprite>(sprite)
			&& let Some(apply) = color_to_apply2(sprite.parent_color(), bg_color, fg_color)
		{
			sprite.set_color(common, apply);
		}
	}

	let globals = common.globals();

	let gradient_intensity = common.state.theme.gradient_intensity;

	//let t = Mat4::from_scale(Vec3::splat(1.0 + pos * 0.5)) * Mat4::from_rotation_z(pos * 1.0);

	let t = Mat4::from_scale(Vec3::splat(1.0 + pos * 0.05));
	widget_data.transform = centered_matrix(widget_boundary.size, &t);

	rect.params.color = bg_color;
	rect.params.color2 = get_color2(&bg_color, gradient_intensity);

	rect.params.border_color = init_border_color.lerp(&globals.palette, &colors.hover_border_color, mult);
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
				state.id_label,
				state.id_sprite,
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

fn register_event_mouse_cancel(state: Rc<RefCell<State>>, listeners: &mut EventListenerCollection) -> EventListenerID {
	listeners.register(
		EventListenerKind::MouseCancel,
		Box::new(move |_common, _event_data, (), ()| {
			let mut state = state.borrow_mut();
			state.down = false;
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
				state.id_label,
				state.id_sprite,
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

			if state.down {
				common.alterables.trigger_haptics();
				common.alterables.play_sound(WguiSoundType::ButtonRelease);
				common.alterables.mark_redraw();

				state.down = false;
				if state.hovered {
					anim_hover(
						common,
						rect,
						state.id_label,
						state.id_sprite,
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

const DEFAULT_COLOR: WguiColor = WguiColorName::BackgroundVariant.to_wgui_color();
const DEFAULT_BORDER_COLOR: WguiColor = WguiColorName::Outline.to_wgui_color();
const DEFAULT_HOVER_COLOR: WguiColor = WguiColorName::Tertiary.to_wgui_color();
const DEFAULT_HOVER_BORDER_COLOR: WguiColor = WguiColorName::Tertiary.to_wgui_color().mult_rgb(0.5);
const DEFAULT_STICKY_COLOR: WguiColor = WguiColorName::Primary.to_wgui_color();
const DEFAULT_STICKY_BORDER_COLOR: WguiColor = WguiColorName::Primary.to_wgui_color().mult_rgb(0.5);

pub fn construct(ess: &mut ConstructEssentials, params: Params) -> anyhow::Result<(WidgetPair, Rc<ComponentButton>)> {
	let mut style = params.style;
	let theme = &ess.layout.state.theme;

	if style.align_items.is_none() {
		style.align_items = Some(AlignItems::CENTER);
	}
	if style.justify_content.is_none() {
		style.justify_content = Some(JustifyContent::CENTER);
	}
	style.overflow.x = taffy::Overflow::Hidden;
	style.overflow.y = taffy::Overflow::Hidden;

	let globals = ess.layout.state.globals.get();

	// update colors to default ones if they are not specified
	let color = params.color.unwrap_or(DEFAULT_COLOR);

	let border_color = params.border_color.unwrap_or(DEFAULT_BORDER_COLOR);

	let hover_color = params.hover_color.unwrap_or(DEFAULT_HOVER_COLOR);

	let hover_border_color = params.hover_border_color.unwrap_or(DEFAULT_HOVER_BORDER_COLOR);

	let sticky_color = params.sticky_color.unwrap_or(DEFAULT_STICKY_COLOR);

	let sticky_border_color = params.sticky_border_color.unwrap_or(DEFAULT_STICKY_BORDER_COLOR);

	let gradient_intensity = theme.gradient_intensity;

	drop(globals);

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

	let id_sprite = if let Some(sprite_path) = params.sprite_src {
		let sprite = WidgetSprite::create(WidgetSpriteParams {
			glyph_data: Some(CustomGlyphData::from_assets(&ess.layout.state.globals, sprite_path)?),
			color: Some(params.sprite_color.unwrap_or(WguiColorName::OnBackground.into())),
			..Default::default()
		});

		let (sprite_pair, _) = ess.layout.add_child(
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

		sprite_pair.id
	} else {
		WidgetID::default()
	};

	let id_label = if let Some(content) = params.text {
		let widget_label = WidgetLabel::create(
			&mut ess.layout.state,
			WidgetLabelParams {
				content,
				style: TextStyle {
					weight: Some(FontWeight::Bold),
					color: Some(WguiColorName::OnBackground.into()),
					..params.text_style
				},
				..Default::default()
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
			sticky_color,
			sticky_border_color,
		},
		id_label,
		id_sprite,
		children_discovered: false,
	}));

	let base = ComponentBase {
		id: root.id,
		lhandles: {
			let listeners = &mut root.widget.state().event_listeners;
			let anim_mult = ess.layout.state.theme.animation_mult;
			vec![
				register_event_mouse_cancel(state.clone(), listeners),
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

fn color_to_apply(parent_color: ParentColor, bg_color: WguiColor) -> Option<WguiColor> {
	match parent_color {
		ParentColor::Foreground => bg_color.fg_color(),
		ParentColor::Background => Some(bg_color),
		ParentColor::Ignore => None,
	}
}

fn color_to_apply2(parent_color: ParentColor, bg_color: WguiColor, fg_color: WguiColor) -> Option<WguiColor> {
	match parent_color {
		ParentColor::Foreground => Some(fg_color),
		ParentColor::Background => Some(bg_color),
		ParentColor::Ignore => None,
	}
}
