use wgui::{
	assets::AssetPath,
	components::button::{ButtonClickCallback, ComponentButton},
	event::EventAlterables,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	renderer_vk::text::custom_glyph::CustomGlyphData,
	widget::{image::WidgetImage, label::WidgetLabel},
};

use crate::util::{networking, wgui_simple};

pub struct Params<'a> {
	pub id_parent: WidgetID,
	pub layout: &'a mut Layout,
	pub entry: networking::skymap_catalog::SkymapCatalogEntry,
	pub on_click: ButtonClickCallback,
}

pub struct View {
	#[allow(dead_code)]
	parser_state: ParserState,
	id_loading: WidgetID,
	id_image_preview: WidgetID,
	image: Option<CustomGlyphData>,
}

impl View {
	pub fn new(par: Params) -> anyhow::Result<Self> {
		let globals = par.layout.state.globals.clone();
		let doc_params = &ParseDocumentParams {
			globals: globals.clone(),
			path: AssetPath::BuiltIn("gui/view/skymap_list_cell.xml"),
			extra: Default::default(),
		};

		let mut parser_state = wgui::parser::parse_from_assets(&doc_params, par.layout, par.id_parent)?;

		let data = parser_state.realize_template(&doc_params, "Cell", par.layout, par.id_parent, Default::default())?;

		let id_image_preview = data.get_widget_id("image_preview")?;

		data
			.fetch_component_as::<ComponentButton>("button")?
			.on_click(par.on_click);

		{
			let mut label_title = data.fetch_widget_as::<WidgetLabel>(&par.layout.state, "label_title")?;
			let mut label_author = data.fetch_widget_as::<WidgetLabel>(&par.layout.state, "label_author")?;

			label_title.set_text_simple(&mut globals.get(), Translation::from_raw_text_string(par.entry.name));
			label_author.set_text_simple(
				&mut globals.get(),
				Translation::from_raw_text_string(format!("by {}", par.entry.author)),
			);
		}

		let id_loading = wgui_simple::create_loading(wgui_simple::CreateLoadingParams {
			layout: par.layout,
			parent_id: id_image_preview,
			with_text: false,
		})?;

		Ok(Self {
			parser_state,
			id_loading,
			id_image_preview,
			image: None,
		})
	}

	pub fn set_image(&mut self, layout: &mut Layout, content: Option<CustomGlyphData>) -> anyhow::Result<()> {
		layout.remove_widget(self.id_loading);
		let mut alt = EventAlterables::default();
		{
			let mut image_preview = layout.state.widgets.cast_as::<WidgetImage>(self.id_image_preview)?;
			image_preview.set_content(&mut alt, content.clone());
		}
		layout.process_alterables(alt)?;
		self.image = content;
		Ok(())
	}

	pub fn get_image(&self) -> Option<CustomGlyphData> {
		return self.image.clone();
	}
}
