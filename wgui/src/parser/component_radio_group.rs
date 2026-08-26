use crate::{
	components::{Component, radio_group},
	layout::WidgetID,
	parser::{AttribPair, ParserContext, ParserFile, parse_children, process_component, style::parse_style},
};

pub fn parse_component_radio_group<'a>(
	file: &'a ParserFile,
	ctx: &mut ParserContext,
	node: roxmltree::Node<'a, 'a>,
	parent_id: WidgetID,
	attribs: &[AttribPair],
	tag_name: &str,
) -> anyhow::Result<WidgetID> {
	let style = parse_style(ctx, attribs, tag_name);

	let (widget, component) = radio_group::construct(&mut ctx.get_construct_essentials(parent_id), style)?;

	process_component(ctx, Component(component), widget.id, attribs);
	let _ = parse_children(file, ctx, node, widget.id)?;

	Ok(widget.id)
}
