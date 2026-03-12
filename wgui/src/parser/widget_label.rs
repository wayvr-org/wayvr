use crate::{
	i18n::Translation,
	layout::WidgetID,
	parser::{
		AttribPair, ParserContext, ParserFile, parse_children, parse_i32, parse_widget_universal,
		style::{parse_style, parse_text_style},
	},
	widget::label::{WidgetLabel, WidgetLabelParams},
};

pub fn parse_widget_label<'a>(
	file: &'a ParserFile,
	ctx: &mut ParserContext,
	node: roxmltree::Node<'a, 'a>,
	parent_id: WidgetID,
	attribs: &[AttribPair],
	tag_name: &str,
) -> anyhow::Result<WidgetID> {
	let mut params = WidgetLabelParams::default();

	let style = parse_style(ctx, attribs, tag_name);
	params.style = parse_text_style(ctx, attribs, tag_name);

	for pair in attribs {
		let (key, value) = (pair.attrib.as_ref(), pair.value.as_ref());
		match key {
			"wrap" => {
				if let Some(num) = parse_i32(value) {
					params.style.wrap = num == 1;
				} else {
					ctx.print_invalid_attrib(tag_name, key, value);
				}
			}
			"text" => {
				if !value.is_empty() {
					params.content = Translation::from_raw_text(value);
				}
			}
			"translation" => {
				if !value.is_empty() {
					params.content = Translation::from_translation_key(value);
				}
			}
			_ => {}
		}
	}

	let label = WidgetLabel::create(&mut ctx.layout.state, params);
	let (widget, _) = ctx.layout.add_child(parent_id, label, style)?;

	parse_widget_universal(ctx, &widget, attribs, tag_name);
	parse_children(file, ctx, node, widget.id)?;

	Ok(widget.id)
}
