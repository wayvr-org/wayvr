use std::{collections::HashMap, rc::Rc};

use wgui::{
	assets::AssetPath,
	components::button::{ButtonClickCallback, ComponentButton},
	event::EventAlterables,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	renderer_vk::text::custom_glyph::CustomGlyphData,
	widget::{image::WidgetImage, label::WidgetLabel},
};

use crate::util::{
	networking::{
		self,
		skymap_catalog::{SkymapCatalogEntry, SkymapResolution},
	},
	wgui_simple,
};

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
	id_resolution_pips: WidgetID,
	image: Option<CustomGlyphData>,
}

fn doc_params(globals: &'_ WguiGlobals) -> ParseDocumentParams<'_> {
	ParseDocumentParams {
		globals: globals.clone(),
		path: AssetPath::BuiltIn("gui/view/skymap_list_cell.xml"),
		extra: Default::default(),
	}
}

fn populate_res_pips(
	layout: &mut Layout,
	id_parent: WidgetID,
	parser_state: &mut ParserState,
	entry: &SkymapCatalogEntry,
) -> anyhow::Result<()> {
	let globals = layout.state.globals.clone();
	layout.remove_children(id_parent);

	let mut populate_res_pip = |res: SkymapResolution| -> anyhow::Result<()> {
		let mut tpar = HashMap::<Rc<str>, Rc<str>>::new();
		let downloaded = entry.is_downloaded(res).unwrap_or(false);
		tpar.insert(
			Rc::from("color"),
			if downloaded {
				Rc::from("#11aa40")
			} else {
				Rc::from("#444444")
			},
		);
		tpar.insert(Rc::from("text"), res.get_display_str_simple().into());
		parser_state.realize_template(&doc_params(&globals), "ResolutionPip", layout, id_parent, tpar)?;

		Ok(())
	};

	populate_res_pip(SkymapResolution::Res2k)?;
	if entry.files.size_4k.is_some() {
		populate_res_pip(SkymapResolution::Res4k)?;
	}
	if entry.files.size_8k.is_some() {
		populate_res_pip(SkymapResolution::Res8k)?;
	}

	Ok(())
}

impl View {
	pub fn new(par: Params) -> anyhow::Result<Self> {
		let globals = par.layout.state.globals.clone();

		let mut parser_state = wgui::parser::parse_from_assets(&doc_params(&globals), par.layout, par.id_parent)?;

		let data = parser_state.realize_template(
			&doc_params(&globals),
			"Cell",
			par.layout,
			par.id_parent,
			Default::default(),
		)?;

		let id_image_preview = data.get_widget_id("image_preview")?;
		let id_resolution_pips = data.get_widget_id("resolution_pips")?;

		data
			.fetch_component_as::<ComponentButton>("button")?
			.on_click(par.on_click);

		{
			let mut label_title = data.fetch_widget_as::<WidgetLabel>(&par.layout.state, "label_title")?;
			let mut label_author = data.fetch_widget_as::<WidgetLabel>(&par.layout.state, "label_author")?;

			label_title.set_text_simple(
				&mut globals.get(),
				Translation::from_raw_text_string(par.entry.name.clone()),
			);
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

		// Populate resolution pips
		populate_res_pips(par.layout, id_resolution_pips, &mut parser_state, &par.entry)?;

		Ok(Self {
			parser_state,
			id_loading,
			id_image_preview,
			image: None,
			id_resolution_pips,
		})
	}

	pub fn refresh_resolution_pips(&mut self, layout: &mut Layout, entry: &SkymapCatalogEntry) -> anyhow::Result<()> {
		populate_res_pips(layout, self.id_resolution_pips, &mut self.parser_state, &entry)?;
		Ok(())
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
