use crate::{
	layout::WidgetID,
	parser::{
		AttribPair, ParseChildResult, ParserContext, ParserFile, parse_children, parse_widget_universal, style::parse_style,
	},
	widget::div::WidgetDiv,
};

pub fn parse_widget_div<'a>(
	file: &ParserFile,
	ctx: &mut ParserContext,
	node: roxmltree::Node<'a, 'a>,
	parent_id: WidgetID,
	attribs: &[AttribPair],
	tag_name: &str,
) -> anyhow::Result<(ParseChildResult, WidgetID)> {
	let style = parse_style(ctx, attribs, tag_name);

	let (widget, _) = ctx.layout.add_child(parent_id, WidgetDiv::create(), style)?;

	parse_widget_universal(ctx, &widget, attribs, tag_name);
	Ok((parse_children(file, ctx, node, widget.id)?, widget.id))
}
