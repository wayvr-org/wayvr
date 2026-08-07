use glam::{Mat4, Vec2, Vec3};
use std::{cell::RefCell, rc::Rc};
use taffy::prelude::length;

use crate::{
	animation::{Animation, AnimationEasing},
	color::{WguiColor, WguiColorName},
	components::{self, Component, ComponentBase, ComponentTrait, RefreshData},
	event::CallbackDataCommon,
	i18n::Translation,
	layout::{self, LayoutTask, LayoutTasks, WidgetID, WidgetPair},
	renderer_vk::text::{FontWeight, TextStyle},
	widget::{
		ConstructEssentials,
		div::WidgetDiv,
		label::{WidgetLabel, WidgetLabelParams},
		rectangle::{WidgetRectangle, WidgetRectangleParams},
		util::WLength,
	},
};

pub trait TooltipTrait {
	fn get(&mut self) -> &mut Option<Rc<ComponentTooltip>>;
}

impl ComponentTooltip {
	pub fn register_hover_in(
		common: &mut CallbackDataCommon,
		tooltip_info: &Option<TooltipInfo>,
		widget_to_watch: WidgetID,
		state: Rc<RefCell<dyn TooltipTrait>>,
	) {
		let Some(info) = tooltip_info.clone() else {
			return;
		};
		common.alterables.tasks.push(LayoutTask::ModifyLayoutState({
			Box::new(move |m| {
				let mut state = state.borrow_mut();
				*state.get() = Some(components::tooltip::show(m.layout, widget_to_watch, info.clone())?);
				Ok(())
			})
		}));
	}
}

#[derive(Clone, Default)]
pub enum TooltipSide {
	Left,
	Right,
	Top,
	#[default]
	Bottom,
}

#[derive(Clone)]
pub struct TooltipInfo {
	pub text: Translation,
	pub side: TooltipSide,
}

pub struct Params {
	pub info: TooltipInfo,
	pub widget_to_watch: WidgetID,
}

impl Default for Params {
	fn default() -> Self {
		Self {
			info: TooltipInfo {
				text: Translation::from_raw_text(""),
				side: TooltipSide::Bottom,
			},
			widget_to_watch: WidgetID::default(),
		}
	}
}

struct State {
	position_shift: Vec2, // in case if the tooltip goes out of bounds
}

#[allow(clippy::struct_field_names)]
struct Data {
	id_root: WidgetID, // div
	id_rect: WidgetID, // rectangle
	side: TooltipSide,
}

pub struct ComponentTooltip {
	base: ComponentBase,
	data: Rc<Data>,
	#[allow(dead_code)]
	state: Rc<RefCell<State>>,
	tasks: LayoutTasks,
}

impl ComponentTrait for ComponentTooltip {
	fn base_mut(&mut self) -> &mut ComponentBase {
		&mut self.base
	}

	fn base(&self) -> &ComponentBase {
		&self.base
	}

	fn refresh(&self, data: &mut RefreshData) {
		let layout_size = data.layout.content_size;
		let pin_pos = data.layout.state.get_widget_boundary(self.data.id_root).pos;
		let rect_size = data.layout.state.get_widget_boundary(self.data.id_rect).size;

		let rect_pos_topleft: Vec2 = match self.data.side {
			TooltipSide::Left => pin_pos + Vec2::new(-rect_size.x, -rect_size.y / 2.0),
			TooltipSide::Right => pin_pos + Vec2::new(0.0, -rect_size.y / 2.0),
			TooltipSide::Top => pin_pos + Vec2::new(-rect_size.x / 2.0, -rect_size.y),
			TooltipSide::Bottom => pin_pos + Vec2::new(-rect_size.x / 2.0, 0.0),
		};

		let mut state = self.state.borrow_mut();

		let padding = 8.0;
		let rect_right = rect_pos_topleft.x + rect_size.x + padding;
		let rect_bottom = rect_pos_topleft.y + rect_size.y + padding;

		let diff_right = rect_right - layout_size.x;
		let diff_bottom = rect_bottom - layout_size.y;

		// limit x < 0 or rect_right > layout_size.x
		if rect_pos_topleft.x < 0.0 {
			state.position_shift.x = -rect_pos_topleft.x;
		} else if diff_right > 0.0 {
			state.position_shift.x = -diff_right;
		}

		// limit y < 0 or rect_bottom > layout_size.y
		if rect_pos_topleft.y < 0.0 {
			state.position_shift.y = -rect_pos_topleft.y;
		} else if diff_bottom > 0.0 {
			state.position_shift.y = -diff_bottom;
		}

		/*log::info!(
			"layout size {:?}, pin pos {:?}, rect size {:?}",
			layout_size,
			pin_pos,
			rect_size
		);*/
	}
}

impl Drop for ComponentTooltip {
	fn drop(&mut self) {
		self.tasks.push(LayoutTask::RemoveWidget(self.data.id_root));
	}
}

pub const TOOLTIP_COLOR: WguiColorName = WguiColorName::BackgroundContrast;
pub const TOOLTIP_BORDER_COLOR: WguiColorName = WguiColorName::Outline;

pub fn construct(ess: &mut ConstructEssentials, params: Params) -> anyhow::Result<(WidgetPair, Rc<ComponentTooltip>)> {
	let absolute_boundary = {
		let widget_to_watch = ess
			.layout
			.state
			.widgets
			.get(params.widget_to_watch)
			.ok_or_else(|| anyhow::anyhow!("widget_to_watch is invalid"))?;
		let widget_to_watch_state = widget_to_watch.state();
		widget_to_watch_state.data.cached_absolute_boundary
	};

	let spacing = 8.0;

	let transform = Mat4::from_translation(Vec3::new(0.0, 0.0, 0.0));

	// this value needs to be bigger than rectangle padding sizes due to the
	// transform stack & scissoring design. Needs investigation, zero-size objects
	// would result in PushScissorStackResult::OutOfBounds otherwise preventing us
	// to render the label. Didn't find the best solution for this edge-case yet,
	// so here it is.
	let pin_size = 32.0;

	let (pin_left, pin_top, pin_align_items, pin_justify_content) = match params.info.side {
		TooltipSide::Left => (
			absolute_boundary.left() - spacing - pin_size,
			absolute_boundary.top() + absolute_boundary.size.y / 2.0 - pin_size / 2.0,
			taffy::AlignItems::CENTER,
			taffy::JustifyContent::END,
		),
		TooltipSide::Right => (
			absolute_boundary.left() + absolute_boundary.size.x + spacing,
			absolute_boundary.top() + absolute_boundary.size.y / 2.0 - pin_size / 2.0,
			taffy::AlignItems::CENTER,
			taffy::JustifyContent::START,
		),
		TooltipSide::Top => (
			absolute_boundary.left() + absolute_boundary.size.x / 2.0 - pin_size / 2.0,
			absolute_boundary.top() - spacing - pin_size,
			taffy::AlignItems::END,
			taffy::JustifyContent::CENTER,
		),
		TooltipSide::Bottom => (
			absolute_boundary.left() + absolute_boundary.size.x / 2.0 - pin_size / 2.0,
			absolute_boundary.top() + absolute_boundary.size.y + spacing,
			taffy::AlignItems::BASELINE,
			taffy::JustifyContent::CENTER,
		),
	};

	let (div, _) = ess.layout.add_child(
		ess.parent,
		WidgetDiv::create(),
		taffy::Style {
			align_items: Some(pin_align_items),
			justify_content: Some(pin_justify_content),
			position: taffy::Position::Absolute,
			margin: taffy::Rect {
				left: length(pin_left),
				top: length(pin_top),
				bottom: length(0.0_f32),
				right: length(0.0_f32),
			},
			/* important, to make it centered! */
			size: taffy::Size {
				width: length(pin_size),
				height: length(pin_size),
			},
			..Default::default()
		},
	)?;

	div.widget.state().data.transform = transform;

	let (rect, _) = ess.layout.add_child(
		div.id,
		WidgetRectangle::create(WidgetRectangleParams {
			color: TOOLTIP_COLOR.into(),
			border_color: TOOLTIP_BORDER_COLOR.into(),
			border: 2.0,
			round: WLength::Units(24.0),
			..Default::default()
		}),
		taffy::Style {
			position: taffy::Position::Relative,
			gap: length(4.0_f32),
			padding: taffy::Rect {
				left: length(16.0_f32),
				right: length(16.0_f32),
				top: length(8.0_f32),
				bottom: length(8.0_f32),
			},
			..Default::default()
		},
	)?;

	let label = WidgetLabel::create(
		&mut ess.layout.state,
		WidgetLabelParams {
			content: params.info.text,
			style: TextStyle {
				weight: Some(FontWeight::Bold),
				..Default::default()
			},
			..Default::default()
		},
	);

	let (label, _) = ess.layout.add_child(rect.id, label, Default::default())?;

	let data = Rc::new(Data {
		id_root: div.id,
		id_rect: rect.id,
		side: params.info.side.clone(),
	});

	let state = Rc::new(RefCell::new(State {
		position_shift: Vec2::default(),
	}));

	let base = ComponentBase::default();

	let tooltip = Rc::new(ComponentTooltip {
		base,
		data,
		state: state.clone(),
		tasks: ess.layout.tasks.clone(),
	});

	let direction = match params.info.side {
		TooltipSide::Left => Vec2::new(-1.0, 0.0),
		TooltipSide::Right => Vec2::new(1.0, 0.0),
		TooltipSide::Top => Vec2::new(0.0, -1.0),
		TooltipSide::Bottom => Vec2::new(0.0, 1.0),
	};

	let anim_mult = ess.layout.state.theme.animation_mult;
	ess.layout.animations.add(Animation::new(
		rect.id,
		(10.0 * anim_mult) as u32,
		AnimationEasing::OutQuad,
		{
			Box::new(move |common, data| {
				let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap(); /* safe */
				let alpha = data.pos;
				rect.params.color = rect.params.color.with_alpha(alpha);
				rect.params.border_color = rect.params.border_color.with_alpha(alpha);

				let position_shift = state.borrow().position_shift;

				let dir_mult = (1.0 - data.pos) * 5.0;
				data.data.transform = Mat4::from_translation(Vec3::new(
					direction.x * dir_mult + position_shift.x,
					direction.y * dir_mult + position_shift.y,
					0.0,
				));

				if let Some(mut label) = common.state.widgets.get_as::<WidgetLabel>(label.id) {
					label.set_color(
						common,
						WguiColor::from(WguiColorName::OnBackground).with_alpha(alpha),
						true,
					);
				}

				common.alterables.mark_redraw();
			})
		},
	));

	ess.layout.defer_component_refresh(Component(tooltip.clone()));
	Ok((div, tooltip))
}

pub fn show(
	layout: &mut layout::Layout,
	widget_to_watch: WidgetID,
	info: TooltipInfo,
) -> anyhow::Result<Rc<ComponentTooltip>> {
	let parent = layout.tree_root_widget;
	let (_, tooltip) = components::tooltip::construct(
		&mut ConstructEssentials { layout, parent },
		components::tooltip::Params { info, widget_to_watch },
	)?;
	Ok(tooltip)
}
