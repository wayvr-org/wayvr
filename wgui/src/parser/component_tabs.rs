use std::rc::Rc;

use crate::{
	assets::AssetPathRc,
	color::WguiColor,
	components::{Component, tabs},
	i18n::Translation,
	layout::WidgetID,
	parser::{
		AttribPair, ParserContext, ParserFile, get_asset_path_from_kv, process_attribs, process_component,
		style::{parse_color_opt, parse_round, parse_style},
	},
	widget::util::WLength,
};

pub fn parse_component_tabs<'a>(
	file: &'a ParserFile,
	ctx: &mut ParserContext,
	node: roxmltree::Node<'a, 'a>,
	parent_id: WidgetID,
	attribs: &[AttribPair],
	tag_name: &str,
) -> anyhow::Result<WidgetID> {
	let style = parse_style(ctx, attribs, tag_name);

	let mut entries = Vec::<tabs::Entry>::new();

	let mut border = 2.0;
	let mut color: Option<WguiColor> = None;
	let mut border_color: Option<WguiColor> = None;
	let mut hover_color: Option<WguiColor> = None;
	let mut hover_border_color: Option<WguiColor> = None;
	let mut sticky_color: Option<WguiColor> = None;
	let mut sticky_border_color: Option<WguiColor> = None;
	let mut round = WLength::Units(4.0);

	for child in node.children() {
		match child.tag_name().name() {
			"" => { /* ignore */ }
			"Tab" => {
				let mut name: Option<Rc<str>> = None;
				let mut text: Option<Translation> = None;
				let mut sprite_src: Option<AssetPathRc> = None;

				let attribs = process_attribs(file, ctx, &child, false);

				for attrib in &attribs {
					match &*attrib.attrib {
						"name" => name = Some(attrib.value.clone()),
						"text" => text = Some(Translation::from_raw_text(&attrib.value)),
						"translation" => text = Some(Translation::from_translation_key(&attrib.value)),
						"sprite_src" | "sprite_src_ext" | "sprite_src_builtin" | "sprite_src_internal" => {
							sprite_src = Some(get_asset_path_from_kv(file, "sprite_", &attrib.attrib, &attrib.value));
						}
						"round" => {
							parse_round(
								ctx,
								tag_name,
								&attrib.attrib,
								&attrib.value,
								&mut round,
								ctx.layout.state.theme.rounding_mult,
							);
						}
						"color" => {
							parse_color_opt(ctx, tag_name, &attrib.attrib, &attrib.value, &mut color);
						}
						"border" => {
							ctx.parse_check_f32(tag_name, &attrib.attrib, &attrib.value, &mut border);
						}
						"border_color" => {
							parse_color_opt(ctx, tag_name, &attrib.attrib, &attrib.value, &mut border_color);
						}
						"hover_color" => {
							parse_color_opt(ctx, tag_name, &attrib.attrib, &attrib.value, &mut hover_color);
						}
						"hover_border_color" => {
							parse_color_opt(ctx, tag_name, &attrib.attrib, &attrib.value, &mut hover_border_color);
						}
						"sticky_color" => {
							parse_color_opt(ctx, tag_name, &attrib.attrib, &attrib.value, &mut sticky_color);
						}
						"sticky_border_color" => {
							parse_color_opt(ctx, tag_name, &attrib.attrib, &attrib.value, &mut sticky_border_color);
						}
						other_key => {
							ctx.print_invalid_attrib("Tab", other_key, &attrib.value);
						}
					}
				}

				if let Some(name) = name
					&& let Some(text) = text
				{
					entries.push(tabs::Entry { sprite_src, text, name });
				}
			}
			other_tag_name => {
				ctx.print_invalid_tag(tag_name, other_tag_name);
			}
		}
	}

	let (widget, component) = tabs::construct(
		&mut ctx.get_construct_essentials(parent_id),
		tabs::Params {
			style,
			selected_entry_name: "first",
			entries,
			on_select: None,
			border,
			color,
			border_color,
			hover_color,
			hover_border_color,
			sticky_color,
			sticky_border_color,
			round,
		},
	)?;

	process_component(ctx, Component(component), widget.id, attribs);

	Ok(widget.id)
}
