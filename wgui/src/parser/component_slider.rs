use crate::{
	components::{Component, slider},
	layout::WidgetID,
	parser::{
		AttribPair, ParserContext,
		helpers::{TooltipAttribs, parse_attrib_tooltip},
		process_component,
		style::parse_style,
	},
	widget::ConstructEssentials,
};

pub fn parse_component_slider(
	ctx: &mut ParserContext,
	parent_id: WidgetID,
	attribs: &[AttribPair],
	tag_name: &str,
) -> anyhow::Result<WidgetID> {
	let mut min_value = 0.0;
	let mut max_value = 1.0;
	let mut initial_value1 = 0.5;
	let mut initial_value2: Option<f32> = None;
	let mut step = 1.0;
	let mut show_value = 1;
	let mut tooltip = TooltipAttribs::default();

	let style = parse_style(ctx, attribs, tag_name);

	for pair in attribs {
		let (key, value) = (pair.attrib.as_ref(), pair.value.as_ref());
		match key {
			"min_value" => {
				ctx.parse_check_f32(tag_name, key, value, &mut min_value);
			}
			"max_value" => {
				ctx.parse_check_f32(tag_name, key, value, &mut max_value);
			}
			"value" => {
				ctx.parse_check_f32(tag_name, key, value, &mut initial_value1);
			}
			"value2" => {
				let mut val = 0.0;
				if ctx.parse_check_f32(tag_name, key, value, &mut val) {
					initial_value2 = Some(val);
				}
			}
			"step" => {
				ctx.parse_check_f32(tag_name, key, value, &mut step);
			}
			"show_value" => {
				ctx.parse_check_i32(tag_name, key, value, &mut show_value);
			}
			_ => {
				parse_attrib_tooltip(ctx, tag_name, pair, &mut tooltip);
			}
		}
	}

	let (widget, component) = slider::construct(
		&mut ConstructEssentials {
			layout: ctx.layout,
			parent: parent_id,
		},
		slider::Params {
			style,
			limits: slider::Limits {
				min_value,
				max_value,
				step,
			},
			value1: slider::Value(initial_value1),
			value2: initial_value2.map(|v| slider::Value(v)),
			show_value: show_value != 0,
			tooltip: tooltip.get_info(),
		},
	)?;

	process_component(ctx, Component(component), widget.id, attribs);

	Ok(widget.id)
}
