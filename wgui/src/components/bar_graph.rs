use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use glam::Vec2;
use taffy::{
	FlexDirection, JustifyContent,
	prelude::{auto, length, percent},
};

use crate::{
	components::{Component, ComponentBase, ComponentTrait, RefreshData},
	drawing::{self, GradientMode, PrimitiveExtent, RenderPrimitive},
	event::CallbackDataCommon,
	i18n::Translation,
	layout::{WidgetID, WidgetPair},
	renderer_vk::text::{FontWeight, HorizontalAlign, TextStyle},
	widget::{
		ConstructEssentials,
		custom_draw::{WidgetCustomDraw, WidgetCustomDrawParams},
		div::WidgetDiv,
		label::{WidgetLabel, WidgetLabelParams},
		rectangle::{WidgetRectangle, WidgetRectangleParams},
		util::WLength,
	},
};

#[derive(Default)]
pub struct Params {
	pub style: taffy::Style,
	pub limits: (f32, f32),
	pub unit: String,
	pub capacity: u32,
}

pub struct ValueCell {
	pub value: f32,
	pub color: drawing::Color,
}

struct State {
	limits: (f32, f32), /* min - max */
	values: VecDeque<ValueCell>,
}

#[allow(clippy::struct_field_names)]
struct Data {
	#[allow(dead_code)]
	id_root: WidgetID,

	id_label_val_min: WidgetID,
	id_label_val_max: WidgetID,

	unit: String,
	capacity: u32,
}

pub struct ComponentBarGraph {
	base: ComponentBase,
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
}

impl ComponentTrait for ComponentBarGraph {
	fn base(&self) -> &ComponentBase {
		&self.base
	}

	fn base_mut(&mut self) -> &mut ComponentBase {
		&mut self.base
	}

	fn refresh(&self, data: &mut RefreshData) {
		let state = self.state.borrow();
		self.update_limits_text(&state, data.common);
	}
}

impl ComponentBarGraph {
	fn update_limits_text(&self, state: &State, c: &mut CallbackDataCommon) -> Option<()> {
		let mut label_val_min = c.state.widgets.get_as::<WidgetLabel>(self.data.id_label_val_min)?;
		let mut label_val_max = c.state.widgets.get_as::<WidgetLabel>(self.data.id_label_val_max)?;

		label_val_min.set_text(
			c,
			Translation::from_raw_text_string(format!("{}{}", state.limits.0, self.data.unit)),
		);
		label_val_max.set_text(
			c,
			Translation::from_raw_text_string(format!("{}{}", state.limits.1, self.data.unit)),
		);

		Some(())
	}

	pub fn set_limits(&self, c: &mut CallbackDataCommon, limits: (f32, f32)) {
		let mut state = self.state.borrow_mut();
		state.limits = limits;
		self.update_limits_text(&state, c);
	}

	pub fn push_value(&self, cell: ValueCell) {
		let mut state = self.state.borrow_mut();
		if state.values.len() > self.data.capacity as usize {
			state.values.pop_front();
		}
		state.values.push_back(cell);
	}
}

pub fn construct(
	ess: &mut ConstructEssentials,
	mut params: Params,
) -> anyhow::Result<(WidgetPair, Rc<ComponentBarGraph>)> {
	params.style.flex_direction = FlexDirection::Row;
	params.style.gap = length(4.0);

	// override style
	let (root, _) = ess.layout.add_child(ess.parent, WidgetDiv::create(), params.style)?;

	let (vertical_texts, _) = ess.layout.add_child(
		root.id,
		WidgetDiv::create(),
		taffy::Style {
			justify_content: Some(JustifyContent::SpaceBetween),
			flex_direction: FlexDirection::Column,
			size: taffy::Size {
				width: auto(),
				height: percent(1.0),
			},
			..Default::default()
		},
	)?;

	let (rect, _) = ess.layout.add_child(
		root.id,
		WidgetRectangle::create(WidgetRectangleParams {
			border: 2.0,
			border_color: drawing::Color::new(1.0, 1.0, 1.0, 0.5),
			round: WLength::Units(3.0),
			gradient: GradientMode::Vertical,
			color: drawing::Color::new(0.0, 0.0, 0.0, 0.6),
			..Default::default()
		}),
		taffy::Style {
			position: taffy::Position::Relative,
			size: taffy::Size {
				width: percent(1.0),
				height: percent(1.0),
			},
			..Default::default()
		},
	)?;

	let state = Rc::new(RefCell::new(State {
		limits: params.limits,
		values: VecDeque::new(),
	}));

	let (_, _) = ess.layout.add_child(
		rect.id,
		WidgetCustomDraw::create(WidgetCustomDrawParams {
			func: {
				let state = state.clone();
				Box::new(move |info| {
					let state = state.borrow();
					let (limit_min, limit_max) = state.limits;

					let box_width = info.boundary.width();
					let box_height = info.boundary.height();

					let bar_width = box_width / state.values.len() as f32;

					for (idx, cell) in state.values.iter().enumerate() {
						let norm_value = ((cell.value - limit_min) / (limit_max - limit_min)).clamp(0.0, 1.0);
						let bar_height = norm_value * box_height;
						let bar_x = bar_width * idx as f32;
						let bar_y = box_height - bar_height;

						info.primitives.push(RenderPrimitive::Rectangle(
							PrimitiveExtent {
								boundary: drawing::Boundary {
									pos: Vec2::new(bar_x, bar_y),
									size: Vec2::new(bar_width, bar_height),
								},
								transform: info.transform.transform,
							},
							drawing::Rectangle {
								color: cell.color,
								..Default::default()
							},
						));
					}
				})
			},
		}),
		taffy::Style {
			size: taffy::Size {
				width: percent(1.0),
				height: percent(1.0),
			},
			..Default::default()
		},
	)?;

	let label_params = WidgetLabelParams {
		style: TextStyle {
			align: Some(HorizontalAlign::Right),
			weight: Some(FontWeight::Bold),
			size: Some(11.0),
			..Default::default()
		},
		..Default::default()
	};

	let label = WidgetLabel::create(&mut ess.layout.state, label_params.clone());
	let (label_val_max, _) = ess.layout.add_child(vertical_texts.id, label, Default::default())?;

	let label = WidgetLabel::create(&mut ess.layout.state, label_params);
	let (label_val_min, _) = ess.layout.add_child(vertical_texts.id, label, Default::default())?;

	let data = Rc::new(Data {
		id_root: root.id,
		id_label_val_min: label_val_min.id,
		id_label_val_max: label_val_max.id,
		unit: params.unit,
		capacity: params.capacity,
	});

	let base = ComponentBase {
		id: root.id,
		lhandles: Vec::new(),
	};

	let bar_graph = Rc::new(ComponentBarGraph { base, data, state });

	ess.layout.defer_component_refresh(Component(bar_graph.clone()));
	Ok((root, bar_graph))
}
