use crate::{
	layout::WidgetID,
	parser::{
		AttribPair, ParserContext, ParserFile, get_asset_path_from_kv, parse_children, parse_widget_universal,
		style::{parse_color, parse_round, parse_style},
	},
	renderer_vk::text::custom_glyph::CustomGlyphData,
	widget::image::{WidgetImage, WidgetImageParams},
};

pub fn parse_widget_image<'a>(
	file: &ParserFile,
	ctx: &mut ParserContext,
	node: roxmltree::Node<'a, 'a>,
	parent_id: WidgetID,
	attribs: &[AttribPair],
	tag_name: &str,
) -> anyhow::Result<WidgetID> {
	let mut params = WidgetImageParams::default();
	let style = parse_style(ctx, attribs, tag_name);
	let mut glyph = None;

	for pair in attribs {
		let (key, value) = (pair.attrib.as_ref(), pair.value.as_ref());
		match key {
			"src" | "src_ext" | "src_builtin" | "src_internal" => {
				if !value.is_empty() {
					glyph = match CustomGlyphData::from_assets(&ctx.layout.state.globals, get_asset_path_from_kv("", key, value))
					{
						Ok(glyph) => Some(glyph),
						Err(e) => {
							log::warn!("failed to load {value}: {e}");
							None
						}
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

	if let Some(glyph) = glyph {
		params.glyph_data = Some(glyph);
	} else {
		ctx.print_missing_attrib(tag_name, "src");
	}

	let (widget, _) = ctx.layout.add_child(parent_id, WidgetImage::create(params), style)?;

	parse_widget_universal(ctx, &widget, attribs, tag_name);
	parse_children(file, ctx, node, widget.id)?;

	Ok(widget.id)
}
