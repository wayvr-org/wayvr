use crate::{
	assets::AssetPathRc,
	color::WguiColor,
	components::{Component, button},
	i18n::Translation,
	layout::WidgetID,
	parser::{
		AttribPair, ParseChildResult, ParserContext, ParserFile, get_asset_path_from_kv,
		helpers::{TooltipAttribs, parse_attrib_tooltip},
		parse_children, parse_f32, process_component,
		style::{parse_color_opt, parse_round, parse_style, parse_text_style},
	},
	widget::util::WLength,
};

pub fn parse_component_button<'a>(
	file: &'a ParserFile,
	ctx: &mut ParserContext,
	node: roxmltree::Node<'a, 'a>,
	parent_id: WidgetID,
	attribs: &[AttribPair],
	tag_name: &str,
) -> anyhow::Result<(ParseChildResult, WidgetID)> {
	let mut color: Option<WguiColor> = None;
	let mut sprite_color: Option<WguiColor> = None;
	let mut border = 2.0;
	let mut border_color: Option<WguiColor> = None;
	let mut hover_color: Option<WguiColor> = None;
	let mut hover_border_color: Option<WguiColor> = None;
	let mut sticky_color: Option<WguiColor> = None;
	let mut sticky_border_color: Option<WguiColor> = None;
	let mut round = WLength::Units(4.0);
	let mut tooltip = TooltipAttribs::default();
	let mut sticky: bool = false;
	let mut long_press_time = 0.0;
	let mut sprite_src: Option<AssetPathRc> = None;

	let mut translation: Option<Translation> = None;

	let text_style = parse_text_style(ctx, attribs, tag_name, "text_");
	let style = parse_style(ctx, attribs, tag_name);

	for pair in attribs {
		let (key, value) = (pair.attrib.as_ref(), pair.value.as_ref());
		match key {
			"text" => {
				if !value.is_empty() {
					translation = Some(Translation::from_raw_text(value));
				}
			}
			"translation" => {
				if !value.is_empty() {
					translation = Some(Translation::from_translation_key(value));
				}
			}
			"round" => {
				parse_round(
					ctx,
					tag_name,
					key,
					value,
					&mut round,
					ctx.layout.state.theme.rounding_mult,
				);
			}
			"color" => {
				parse_color_opt(ctx, tag_name, key, value, &mut color);
			}
			"sprite_color" => {
				parse_color_opt(ctx, tag_name, key, value, &mut sprite_color);
			}
			"border" => {
				ctx.parse_check_f32(tag_name, key, value, &mut border);
			}
			"border_color" => {
				parse_color_opt(ctx, tag_name, key, value, &mut border_color);
			}
			"hover_color" => {
				parse_color_opt(ctx, tag_name, key, value, &mut hover_color);
			}
			"hover_border_color" => {
				parse_color_opt(ctx, tag_name, key, value, &mut hover_border_color);
			}
			"sticky_color" => {
				parse_color_opt(ctx, tag_name, key, value, &mut sticky_color);
			}
			"sticky_border_color" => {
				parse_color_opt(ctx, tag_name, key, value, &mut sticky_border_color);
			}
			"sprite_src" | "sprite_src_ext" | "sprite_src_builtin" | "sprite_src_internal" => {
				let asset_path = get_asset_path_from_kv(file, "sprite_", key, value);

				if !value.is_empty() {
					sprite_src = Some(asset_path);
				}
			}
			"sticky" => {
				let mut sticky_i32 = 0;
				sticky = ctx.parse_check_i32(tag_name, key, value, &mut sticky_i32) && sticky_i32 == 1;
			}
			"long_press_time" => {
				long_press_time = parse_f32(value).unwrap_or(long_press_time);
			}
			_ => {
				parse_attrib_tooltip(ctx, tag_name, pair, &mut tooltip);
			}
		}
	}

	let (widget, button) = button::construct(
		&mut ctx.get_construct_essentials(parent_id),
		button::Params {
			sprite_color,
			color,
			border,
			border_color,
			hover_color,
			hover_border_color,
			sticky_color,
			sticky_border_color,
			text: translation,
			style,
			text_style,
			round,
			tooltip: tooltip.get_info(),
			sticky,
			long_press_time,
			sprite_src: sprite_src.as_ref().map(|s| s.as_ref()),
		},
	)?;

	process_component(ctx, Component(button), widget.id, attribs);
	Ok((parse_children(file, ctx, node, widget.id)?, widget.id))
}
