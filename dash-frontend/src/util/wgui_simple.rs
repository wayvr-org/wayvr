use glam::{Mat4, Vec2};
use wgui::{
	animation::{Animation, AnimationEasing},
	assets::AssetPath,
	drawing,
	i18n::Translation,
	layout::{Layout, LayoutTask, WidgetID},
	parser::{Fetchable, ParseDocumentParams},
	renderer_vk::{
		text::{FontWeight, TextStyle, custom_glyph::CustomGlyphData},
		util::centered_matrix,
	},
	taffy::{self, prelude::length},
	widget::{
		label::{WidgetLabel, WidgetLabelParams},
		sprite::{WidgetSprite, WidgetSpriteParams},
	},
};

pub fn create_label(layout: &mut Layout, parent: WidgetID, content: Translation) -> anyhow::Result<()> {
	let label = WidgetLabel::create(
		&mut layout.state,
		WidgetLabelParams {
			content,
			style: TextStyle {
				wrap: true,
				..Default::default()
			},
		},
	);

	layout.add_child(parent, label, Default::default())?;

	Ok(())
}

pub fn create_label_error(layout: &mut Layout, parent: WidgetID, content: String) -> anyhow::Result<()> {
	let label = WidgetLabel::create(
		&mut layout.state,
		WidgetLabelParams {
			content: Translation::from_raw_text_string(content),
			style: TextStyle {
				wrap: true,
				color: Some(drawing::Color::new(1.0, 0.5, 0.0, 1.0)),
				weight: Some(FontWeight::Bold),
				..Default::default()
			},
		},
	);

	layout.add_child(parent, label, Default::default())?;

	Ok(())
}

pub fn create_icon(layout: &mut Layout, id_parent: WidgetID, size: Vec2, path: AssetPath) -> anyhow::Result<WidgetID> {
	let widget_sprite = WidgetSprite::create(WidgetSpriteParams {
		color: None,
		glyph_data: Some(CustomGlyphData::from_assets(&layout.state.globals, path)?),
	});

	let size = taffy::Size {
		width: length(size.x),
		height: length(size.y),
	};

	let (widget, _) = layout.add_child(
		id_parent,
		widget_sprite,
		taffy::Style {
			min_size: size.clone(),
			max_size: size.clone(),
			size: size.clone(),
			..Default::default()
		},
	)?;

	Ok(widget.id)
}

pub struct CreateLoadingParams<'a> {
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub with_text: bool,
}

pub fn create_loading(par: CreateLoadingParams) -> anyhow::Result<WidgetID> {
	let doc_params = ParseDocumentParams {
		globals: par.layout.state.globals.clone(),
		path: AssetPath::BuiltIn("gui/t_loading.xml"),
		extra: Default::default(),
	};

	let mut parser_state = wgui::parser::parse_from_assets(&doc_params, par.layout, par.parent_id)?;

	let data = parser_state.realize_template(
		&doc_params,
		if par.with_text {
			"LoadingWithText"
		} else {
			"LoadingWithoutText"
		},
		par.layout,
		par.parent_id,
		Default::default(),
	)?;

	let id_root = data.get_widget_id("root")?;
	let id_sprite_loading = data.get_widget_id("sprite_loading")?;

	par.layout.animations.add(Animation::new(
		id_sprite_loading,
		60 * 30, /* spin it for 30 seconds at most */
		AnimationEasing::Linear,
		Box::new(move |common, data| {
			// spin it
			data.data.transform = centered_matrix(data.widget_boundary.size, &Mat4::from_rotation_z(data.pos * 400.0));
			if data.pos == 1.0 {
				// remove the spinner, do not waste energy
				common
					.alterables
					.tasks
					.push(LayoutTask::RemoveWidget(id_sprite_loading));
			}
			common.alterables.mark_redraw();
		}),
	));

	Ok(id_root)
}
