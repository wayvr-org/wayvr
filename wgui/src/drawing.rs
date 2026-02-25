use std::{cell::RefCell, rc::Rc};

use cosmic_text::Buffer;
use glam::{Mat4, Vec2};
use taffy::TraversePartialTree;

use crate::{
	drawing,
	event::EventAlterables,
	globals::Globals,
	layout::Widget,
	renderer_vk::text::{
		TextShadow,
		custom_glyph::{CustomGlyph, CustomGlyphData},
	},
	stack::{self, ScissorBoundary, ScissorStack, TransformStack},
	widget::{self, ScrollbarInfo, WidgetState},
};

use super::{layout::Layout, widget::DrawState};

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Boundary {
	pub pos: Vec2,
	pub size: Vec2,
}

impl Boundary {
	pub const fn from_pos_size(pos: Vec2, size: Vec2) -> Self {
		Self { pos, size }
	}

	/// top-left is an absolute position
	pub const fn construct_absolute(transform_stack: &TransformStack) -> Self {
		let transform = transform_stack.get();

		Self {
			pos: transform.abs_pos,
			size: transform.raw_dim,
		}
	}

	/// top-left is zero
	pub const fn construct_relative(transform_stack: &TransformStack) -> Self {
		let transform = transform_stack.get();

		Self {
			pos: Vec2::ZERO,
			size: transform.raw_dim,
		}
	}

	pub const fn bottom_left(&self) -> Vec2 {
		Vec2::new(self.pos.x, self.pos.y + self.size.y)
	}

	pub const fn bottom_right(&self) -> Vec2 {
		Vec2::new(self.pos.x + self.size.x, self.pos.y + self.size.y)
	}

	pub const fn top_right(&self) -> Vec2 {
		Vec2::new(self.pos.x + self.size.x, self.pos.y)
	}

	pub const fn center(&self) -> Vec2 {
		Vec2::new(self.pos.x + self.size.x / 2.0, self.pos.y + self.size.y / 2.0)
	}

	pub const fn width(&self) -> f32 {
		self.size.x
	}

	pub const fn height(&self) -> f32 {
		self.size.y
	}

	pub const fn area(&self) -> f32 {
		self.size.x * self.size.y
	}

	pub const fn contains_point(&self, point: Vec2) -> bool {
		point.x >= self.pos.x
			&& point.x <= self.pos.x + self.size.x
			&& point.y >= self.pos.y
			&& point.y <= self.pos.y + self.size.y
	}

	pub const fn top(&self) -> f32 {
		self.pos.y
	}

	pub const fn bottom(&self) -> f32 {
		self.pos.y + self.size.y
	}

	pub const fn left(&self) -> f32 {
		self.pos.x
	}

	pub const fn right(&self) -> f32 {
		self.pos.x + self.size.x
	}
}

#[derive(Debug, Copy, Clone)]
pub struct Color {
	pub r: f32,
	pub g: f32,
	pub b: f32,
	pub a: f32,
}

pub const ANSI_RESET_CODE: &str = "\x1b[39m\x1b[49m";
pub const ANSI_BOLD_CODE: &str = "\x1b[1m";

impl Color {
	pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
		Self { r, g, b, a }
	}

	#[must_use]
	pub fn add_rgb(&self, n: f32) -> Self {
		Self {
			r: self.r + n,
			g: self.g + n,
			b: self.b + n,
			a: self.a,
		}
	}

	#[must_use]
	pub fn mult_rgb(&self, n: f32) -> Self {
		Self {
			r: self.r * n,
			g: self.g * n,
			b: self.b * n,
			a: self.a,
		}
	}

	#[must_use]
	pub fn lerp(&self, other: &Self, n: f32) -> Self {
		Self {
			r: self.r * (1.0 - n) + other.r * n,
			g: self.g * (1.0 - n) + other.g * n,
			b: self.b * (1.0 - n) + other.b * n,
			a: self.a * (1.0 - n) + other.a * n,
		}
	}

	pub fn debug_ansi_format(&self) -> String {
		let r = (self.r * 255.0).clamp(0.0, 255.0) as u8;
		let g = (self.g * 255.0).clamp(0.0, 255.0) as u8;
		let b = (self.b * 255.0).clamp(0.0, 255.0) as u8;
		format!("\x1b[38;2;{r};{g};{b}m")
	}

	// pretty-print ansi escape code color
	pub fn debug_ansi_block(&self) -> String {
		format!("{}███{}", self.debug_ansi_format(), ANSI_RESET_CODE)
	}

	#[must_use]
	pub const fn with_alpha(&self, n: f32) -> Self {
		Self {
			r: self.r,
			g: self.g,
			b: self.b,
			a: n,
		}
	}

	#[must_use]
	pub fn to_hex(&self) -> String {
		let r = (self.r.clamp(0.0, 1.0) * 255.0).round() as u8;
		let g = (self.g.clamp(0.0, 1.0) * 255.0).round() as u8;
		let b = (self.b.clamp(0.0, 1.0) * 255.0).round() as u8;
		let a = (self.a.clamp(0.0, 1.0) * 255.0).round() as u8;
		format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
	}

	#[must_use]
	pub fn as_arr(&self) -> [f32; 4] {
		[self.r, self.b, self.g, self.a]
	}
}

impl Default for Color {
	fn default() -> Self {
		// opaque black
		Self::new(0.0, 0.0, 0.0, 1.0)
	}
}

#[repr(u8)]
#[derive(Debug, Default, Clone, Copy)]
pub enum GradientMode {
	#[default]
	None,
	Horizontal,
	Vertical,
	Radial,
}

#[derive(Default, Clone, Copy)]
pub struct Rectangle {
	pub color: Color,
	pub color2: Color,
	pub gradient: GradientMode,

	pub border: f32, // width in pixels
	pub border_color: Color,

	pub round_units: u8,
}

#[derive(Clone)]
pub struct ImagePrimitive {
	pub content: CustomGlyphData,
	pub content_key: usize,

	pub border: f32, // width in pixels
	pub border_color: Color,

	pub round_units: u8,
}

#[derive(Clone)]
pub struct PrimitiveExtent {
	pub(super) boundary: Boundary,
	pub(super) transform: Mat4,
}

#[derive(Clone)]
pub enum RenderPrimitive {
	NewPass,
	Rectangle(PrimitiveExtent, Rectangle),
	Text(PrimitiveExtent, Rc<RefCell<Buffer>>, Option<TextShadow>),
	Sprite(PrimitiveExtent, Option<CustomGlyph>), //option because we want as_slice
	Image(PrimitiveExtent, ImagePrimitive),
	ScissorSet(ScissorBoundary),
}

pub struct DrawParams<'a> {
	pub globals: &'a Globals,
	pub layout: &'a mut Layout,
	pub debug_draw: bool,
	pub timestep_alpha: f32, // timestep alpha, 0.0 - 1.0, used for motion interpolation if rendering above tick rate: smoother animations or scrolling
}

pub fn has_overflow_clip(style: &taffy::Style) -> bool {
	style.overflow.x != taffy::Overflow::Visible || style.overflow.y != taffy::Overflow::Visible
}

fn primitive_debug_rect(
	boundary: &Boundary,
	transform: &Mat4,
	color: drawing::Color,
	border: f32,
) -> drawing::RenderPrimitive {
	drawing::RenderPrimitive::Rectangle(
		PrimitiveExtent {
			boundary: *boundary,
			transform: *transform,
		},
		Rectangle {
			border,
			border_color: color,
			color: Color::new(0.0, 0.0, 0.0, 0.0),
			..Default::default()
		},
	)
}

pub fn push_transform_stack(
	transform_stack: &mut TransformStack,
	l: &taffy::Layout,
	scroll_shift: Vec2,
	widget_state: &WidgetState,
) {
	let raw_dim = Vec2::new(l.size.width, l.size.height);
	let visual_dim = raw_dim + scroll_shift;
	let content_dim = Vec2::new(l.content_box_width(), l.content_box_height());
	let content_rel_pos = Vec2::new(l.content_box_x(), l.content_box_y());

	transform_stack.push(stack::Transform {
		rel_pos: Vec2::new(l.location.x, l.location.y) - scroll_shift,
		transform: widget_state.data.transform,
		raw_dim,
		visual_dim,
		content_dim,
		content_rel_pos,
		abs_pos: Default::default(),
		transform_rel: Default::default(),
	});
}

#[derive(Eq, PartialEq)]
pub enum PushScissorStackResult {
	VisibleDontClip, // scissor calculated, but don't clip anything
	VisibleAndClip,  // scissor should be applied at this stage (ScissorSet primitive needs to be called)
	OutOfBounds,     // scissor rectangle is out of bounds (negative boundary dimensions)
}

impl PushScissorStackResult {
	pub fn should_display(&self) -> bool {
		*self != Self::OutOfBounds
	}
}

/// Returns Some() if scissor has been pushed.
pub fn push_scissor_stack(
	transform_stack: &mut TransformStack,
	scissor_stack: &mut ScissorStack,
	scroll_shift: Vec2,
	info: &Option<ScrollbarInfo>,
	style: &taffy::Style,
) -> Option<PushScissorStackResult> {
	let mut boundary_absolute = drawing::Boundary::construct_absolute(transform_stack);
	boundary_absolute.pos += scroll_shift;

	let do_clip = info.is_some() && has_overflow_clip(style);

	if !do_clip {
		return None; // Don't care
	}

	scissor_stack.push(ScissorBoundary(boundary_absolute));

	if scissor_stack.is_out_of_bounds() {
		return Some(PushScissorStackResult::OutOfBounds);
	}

	if do_clip {
		Some(PushScissorStackResult::VisibleAndClip)
	} else {
		Some(PushScissorStackResult::VisibleDontClip)
	}
}

struct DrawWidgetInternal {
	// how many times ScissorSet render primitives has been called?
	scissor_set_count: u32,
}

fn draw_widget(
	params: &DrawParams,
	state: &mut DrawState,
	node_id: taffy::NodeId,
	style: &taffy::Style,
	internal: &mut DrawWidgetInternal,
	widget: &Widget,
) {
	let Ok(l) = params.layout.state.tree.layout(node_id) else {
		debug_assert!(false);
		return;
	};

	let mut widget_state = widget.state();

	if widget_state.flags.new_pass {
		state.primitives.push(RenderPrimitive::NewPass);
	}

	let (scroll_shift, wants_redraw, info) = match widget::get_scrollbar_info(l) {
		Some(info) => {
			let (scrolling, wants_redraw) = widget_state.get_scroll_shift_smooth(&info, l, params.timestep_alpha);
			(scrolling, wants_redraw, Some(info))
		}
		None => (Vec2::default(), false, None),
	};

	// see layout.rs push_event_widget too
	push_transform_stack(state.transform_stack, l, scroll_shift, &widget_state);

	if params.debug_draw {
		let boundary = drawing::Boundary::construct_relative(state.transform_stack);
		state.primitives.push(primitive_debug_rect(
			&boundary,
			&state.transform_stack.get().transform,
			Color::new(0.0, 1.0, 1.0, 0.5),
			1.0,
		));
	}

	let starting_scissor_set_count = internal.scissor_set_count;
	let scissor_result = push_scissor_stack(state.transform_stack, state.scissor_stack, scroll_shift, &info, style);

	if let Some(scissor_result) = &scissor_result
		&& *scissor_result == PushScissorStackResult::VisibleAndClip
	{
		if params.debug_draw {
			let mut boundary_relative = drawing::Boundary::construct_relative(state.transform_stack);
			boundary_relative.pos += scroll_shift;
			state.primitives.push(primitive_debug_rect(
				&boundary_relative,
				&state.transform_stack.get().transform,
				Color::new(1.0, 0.0, 1.0, 1.0),
				3.0,
			));
		}
		state
			.primitives
			.push(drawing::RenderPrimitive::ScissorSet(*state.scissor_stack.get()));
		internal.scissor_set_count += 1;
	}

	let draw_params = widget::DrawParams {
		node_id,
		taffy_layout: l,
		style,
	};

	if scissor_result
		.as_ref()
		.is_none_or(PushScissorStackResult::should_display)
	{
		widget_state.draw_all(state, &draw_params);
		draw_children(params, state, node_id, internal, false);
	}

	if scissor_result.is_some() {
		state.scissor_stack.pop();
	}

	let current_scissor_set_count = internal.scissor_set_count;

	if current_scissor_set_count > starting_scissor_set_count {
		state
			.primitives
			.push(drawing::RenderPrimitive::ScissorSet(*state.scissor_stack.get()));
	}

	if let Some(info) = &info {
		widget_state.draw_scrollbars(state, &draw_params, info);
	}

	state.transform_stack.pop();

	if wants_redraw {
		state.alterables.mark_redraw();
	}
}

fn draw_children(
	params: &DrawParams,
	state: &mut DrawState,
	parent_node_id: taffy::NodeId,
	internal: &mut DrawWidgetInternal,
	is_topmost: bool,
) {
	let layout = &params.layout;

	for node_id in layout.state.tree.child_ids(parent_node_id) {
		let Ok(style) = layout.state.tree.style(node_id) else {
			debug_assert!(false);
			continue;
		};

		if style.display == taffy::Display::None {
			continue;
		}

		let Some(widget_id) = layout.state.tree.get_node_context(node_id).copied() else {
			debug_assert!(false);
			continue;
		};

		let Some(widget) = layout.state.widgets.get(widget_id) else {
			debug_assert!(false);
			continue;
		};

		draw_widget(params, state, node_id, style, internal, widget);

		if is_topmost {
			state.primitives.push(RenderPrimitive::NewPass);
		}
	}
}

pub fn draw(params: &mut DrawParams) -> anyhow::Result<Vec<RenderPrimitive>> {
	let mut primitives = Vec::<RenderPrimitive>::new();
	let mut transform_stack = TransformStack::new();
	let mut scissor_stack = ScissorStack::new();

	let mut alterables = EventAlterables::default();

	let mut state = DrawState {
		globals: params.globals,
		primitives: &mut primitives,
		transform_stack: &mut transform_stack,
		scissor_stack: &mut scissor_stack,
		layout: params.layout,
		alterables: &mut alterables,
	};

	let mut internal = DrawWidgetInternal { scissor_set_count: 0 };

	draw_children(params, &mut state, params.layout.tree_root_node, &mut internal, true);

	params.layout.process_alterables(alterables)?;

	Ok(primitives)
}
