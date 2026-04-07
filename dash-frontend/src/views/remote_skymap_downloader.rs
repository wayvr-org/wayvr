use std::{collections::HashMap, rc::Rc};

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::{
		networking::{self, skymap_catalog::SkymapResolution},
		popup_manager::{MountPopupOnceParams, PopupHolder},
	},
};
use wgui::{
	assets::AssetPath,
	components::button::ComponentButton,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	renderer_vk::text::custom_glyph::CustomGlyphData,
	task::Tasks,
	widget::{image::WidgetImage, label::WidgetLabel},
};
use wlx_common::async_executor::AsyncExecutor;

pub struct Params<'a> {
	pub globals: &'a WguiGlobals,
	pub layout: &'a mut Layout,
	pub executor: &'a AsyncExecutor,
	pub parent_id: WidgetID,
	pub entry: networking::skymap_catalog::SkymapCatalogEntry,
	pub on_close_request: Box<dyn Fn()>,
	pub preview_image: Option<CustomGlyphData>,
}

#[derive(Clone)]
enum Task {
	ResolutionClicked(networking::skymap_catalog::SkymapResolution),
}

pub struct View {
	id_parent: WidgetID,
	entry: networking::skymap_catalog::SkymapCatalogEntry,
	globals: WguiGlobals,
	tasks: Tasks<Task>,
	executor: AsyncExecutor,

	#[allow(dead_code)]
	parser_state: ParserState,
}

fn mount_resolution_button(
	parser_state: &mut ParserState,
	doc_params: &ParseDocumentParams,
	layout: &mut Layout,
	parent_id: WidgetID,
	res: SkymapResolution,
	tasks: &Tasks<Task>,
) -> anyhow::Result<()> {
	let mut t = HashMap::<Rc<str>, Rc<str>>::new();
	t.insert(Rc::from("text"), Rc::from(res.get_display_str()));
	let data = parser_state.realize_template(doc_params, "ResolutionButton", layout, parent_id, t)?;
	let button = data.fetch_component_as::<ComponentButton>("button")?;
	tasks.handle_button(&button, Task::ResolutionClicked(res));
	Ok(())
}

impl View {
	pub fn new(par: Params) -> anyhow::Result<Self> {
		let tasks = Tasks::<Task>::new();

		let doc_params = ParseDocumentParams {
			globals: par.globals.clone(),
			path: AssetPath::BuiltIn("gui/view/remote_skymap_downloader.xml"),
			extra: Default::default(),
		};

		let mut parser_state = wgui::parser::parse_from_assets(&doc_params, par.layout, par.parent_id)?;
		let id_resolution_buttons = parser_state.get_widget_id("resolution_buttons")?;

		let str_version = par.globals.i18n().translate("VERSION");
		let str_creation_date = par.globals.i18n().translate("CREATION_DATE");
		let str_modification_date = par.globals.i18n().translate("MODIFICATION_DATE");

		let image = parser_state.fetch_widget(&par.layout.state, "image")?.widget;
		let mut image = image.cast::<WidgetImage>()?;
		image.set_content(&mut par.layout.alterables, par.preview_image);

		// Set author label
		parser_state
			.fetch_widget_as::<WidgetLabel>(&par.layout.state, "label_author")?
			.set_text_simple(
				&mut par.globals.get(),
				Translation::from_raw_text_string(format!("by {}", par.entry.author)),
			);

		// Set description label
		parser_state
			.fetch_widget_as::<WidgetLabel>(&par.layout.state, "label_description")?
			.set_text_simple(
				&mut par.globals.get(),
				Translation::from_raw_text(&par.entry.description),
			);

		// Set version label
		parser_state
			.fetch_widget_as::<WidgetLabel>(&par.layout.state, "label_version")?
			.set_text_simple(
				&mut par.globals.get(),
				Translation::from_raw_text_string(format!("{}: {}", str_version, par.entry.version)),
			);

		// Set creation date label
		parser_state
			.fetch_widget_as::<WidgetLabel>(&par.layout.state, "label_creation_date")?
			.set_text_simple(
				&mut par.globals.get(),
				Translation::from_raw_text_string(format!("{}: {}", str_creation_date, par.entry.created_at,)),
			);

		// Set modification date label
		parser_state
			.fetch_widget_as::<WidgetLabel>(&par.layout.state, "label_modification_date")?
			.set_text_simple(
				&mut par.globals.get(),
				Translation::from_raw_text_string(format!("{}: {}", str_modification_date, par.entry.created_at,)),
			);

		let files = &par.entry.files;
		let mut mount_res = |res: SkymapResolution| -> anyhow::Result<()> {
			mount_resolution_button(
				&mut parser_state,
				&doc_params,
				par.layout,
				id_resolution_buttons,
				res,
				&tasks,
			)
		};

		mount_res(SkymapResolution::Res2k)?;
		if files.size_4k.is_some() {
			mount_res(SkymapResolution::Res4k)?;
		}
		if files.size_8k.is_some() {
			mount_res(SkymapResolution::Res8k)?;
		}

		Ok(Self {
			id_parent: par.parent_id,
			tasks,
			globals: par.globals.clone(),
			executor: par.executor.clone(),
			entry: par.entry,
			parser_state,
		})
	}

	pub fn update(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		for task in self.tasks.drain() {
			match task {
				Task::ResolutionClicked(skymap_resolution) => todo!(),
			}
		}
		Ok(())
	}
}

pub fn mount_popup(
	frontend_tasks: FrontendTasks,
	executor: AsyncExecutor,
	globals: WguiGlobals,
	entry: networking::skymap_catalog::SkymapCatalogEntry,
	preview_image: Option<CustomGlyphData>,
	on_close_request: Box<dyn Fn()>,
	set_holder: Box<dyn FnOnce(PopupHolder<View>)>,
) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_raw_text(&entry.name),
			Box::new(move |data| {
				let view = View::new(Params {
					globals: &globals,
					layout: data.layout,
					executor: &executor,
					parent_id: data.id_content,
					entry,
					on_close_request,
					preview_image,
				})?;

				set_holder((data.handle, view));
				Ok(())
			}),
		)));
}
