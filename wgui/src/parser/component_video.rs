use crate::{
	assets::AssetPathRc,
	components::{self, Component},
	layout::WidgetID,
	parser::{
		AttribPair, ParserContext, ParserFile, get_asset_path_from_kv, parse_children, parse_f32, parse_i32,
		process_component, style::parse_style,
	},
};

pub fn parse_component_video<'a>(
	file: &'a ParserFile,
	ctx: &mut ParserContext,
	node: roxmltree::Node<'a, 'a>,
	parent_id: WidgetID,
	attribs: &[AttribPair],
	tag_name: &str,
) -> anyhow::Result<WidgetID> {
	let mut src: Option<AssetPathRc> = None;
	let mut looping: bool = false;
	let mut speed: f32 = 1.0;

	let style = parse_style(ctx, attribs, tag_name);

	for pair in attribs {
		let (key, value) = (pair.attrib.as_ref(), pair.value.as_ref());
		match key {
			"speed" => {
				speed = parse_f32(value).unwrap_or(speed);
			}
			"looping" => {
				if let Some(v) = parse_i32(value)
					&& v != 0
				{
					looping = true;
				}
			}
			"src" | "src_ext" | "src_builtin" | "src_internal" => {
				let asset_path = get_asset_path_from_kv(file, "", key, value);

				if !value.is_empty() {
					src = Some(asset_path);
				}
			}
			_ => {}
		}
	}

	let (widget, video) = components::video::construct(
		&mut ctx.get_construct_essentials(parent_id),
		components::video::Params {
			style,
			src: src.as_ref().map(|s| s.as_ref()),
			looping,
			speed,
		},
	)?;

	process_component(ctx, Component(video), widget.id, attribs);
	parse_children(file, ctx, node, widget.id)?;

	Ok(widget.id)
}
