use crate::{
	components::tooltip,
	i18n::Translation,
	parser::{AttribPair, ParserContext},
};

#[derive(Default)]
pub struct TooltipAttribs {
	tooltip: Option<Translation>,
	tooltip_side: Option<tooltip::TooltipSide>,
}

impl TooltipAttribs {
	pub fn get_info(self) -> Option<tooltip::TooltipInfo> {
		self.tooltip.map(|text| tooltip::TooltipInfo {
			text,
			side: self.tooltip_side.unwrap_or(tooltip::TooltipSide::Top),
		})
	}
}

pub fn parse_attrib_tooltip(ctx: &mut ParserContext, tag_name: &str, pair: &AttribPair, tooltip: &mut TooltipAttribs) {
	match pair.attrib.as_ref() {
		"tooltip" if !pair.value.is_empty() => tooltip.tooltip = Some(Translation::from_translation_key(&pair.value)),
		"tooltip_str" if !pair.value.is_empty() => tooltip.tooltip = Some(Translation::from_raw_text(&pair.value)),
		"tooltip_side" => {
			tooltip.tooltip_side = match pair.value.as_ref() {
				"left" => Some(tooltip::TooltipSide::Left),
				"right" => Some(tooltip::TooltipSide::Right),
				"top" => Some(tooltip::TooltipSide::Top),
				"bottom" => Some(tooltip::TooltipSide::Bottom),
				_ => {
					ctx.print_invalid_attrib(tag_name, &pair.attrib, &pair.value);
					None
				}
			}
		}
		_ => {}
	}
}
