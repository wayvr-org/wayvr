use crate::{
	drawing::GradientMode,
	layout::WidgetID,
	parser::{
		AttribPair, ParserContext, ParserFile, parse_children, parse_widget_universal,
		style::{parse_color, parse_round, parse_style},
	},
	widget::rectangle::{WidgetRectangle, WidgetRectangleParams},
};

pub fn parse_widget_rectangle<'a>(
	file: &ParserFile,
	ctx: &mut ParserContext,
	node: roxmltree::Node<'a, 'a>,
	parent_id: WidgetID,
	attribs: &[AttribPair],
	tag_name: &str,
) -> anyhow::Result<WidgetID> {
	let mut params = WidgetRectangleParams::default();
	let style = parse_style(ctx, attribs, tag_name);

	for pair in attribs {
		let (key, value) = (pair.attrib.as_ref(), pair.value.as_ref());
		match key {
			"color" => {
				parse_color(ctx, tag_name, key, value, &mut params.color);
			}
			"color2" => {
				parse_color(ctx, tag_name, key, value, &mut params.color2);
			}
			"gradient" => {
				params.gradient = match value {
					"horizontal" => GradientMode::Horizontal,
					"vertical" => GradientMode::Vertical,
					"radial" => GradientMode::Radial,
					"none" => GradientMode::None,
					_ => {
						ctx.print_invalid_attrib(tag_name, key, value);
						GradientMode::None
					}
				}
			}
			"round" => {
				parse_round(
					ctx,
					tag_name,
					key,
					value,
					&mut params.round,
					ctx.layout.state.theme.rounding_mult,
				);
			}
			"border" => {
				params.border = value.parse().unwrap_or_else(|_| {
					ctx.print_invalid_attrib(tag_name, key, value);
					0.0
				});
			}
			"border_color" => {
				parse_color(ctx, tag_name, key, value, &mut params.border_color);
			}
			_ => {}
		}
	}

	let (widget, _) = ctx
		.layout
		.add_child(parent_id, WidgetRectangle::create(params), style)?;

	parse_widget_universal(ctx, &widget, attribs, tag_name);
	parse_children(file, ctx, node, widget.id)?;

	Ok(widget.id)
}
