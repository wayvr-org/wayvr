use anyhow::Context;
use wgui::{
	assets::AssetPath,
	components::button::ComponentButton,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	renderer_vk::text::custom_glyph::CustomGlyphData,
	task::Tasks,
};
use wlx_common::{async_executor::AsyncExecutor, config_io};

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::{
		networking::skymap_catalog::{self, SkymapCatalogEntry, SkymapResolution},
		popup_manager::{MountPopupOnceParams, PopupHolder},
		wgui_simple,
	},
	views::{self, ViewTrait, ViewUpdateParams},
};

#[derive(Clone)]
enum Task {
	DownloadSkymaps,
	Refresh,
	SetSkymap(SkymapCatalogEntry),
}

pub struct Params<'a> {
	pub globals: WguiGlobals,
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub frontend_tasks: &'a FrontendTasks,
}

struct Cell {
	#[allow(dead_code)]
	view: views::skymap_list_cell::View,
}

pub struct View {
	#[allow(dead_code)]
	parser_state: ParserState,
	tasks: Tasks<Task>,
	list_parent: WidgetID,
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	popup_remote_skymap_list: PopupHolder<views::remote_skymap_list::View>,
	cells: Vec<Cell>,
}

impl ViewTrait for View {
	fn update(&mut self, par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		self.popup_remote_skymap_list.update(par)?;

		loop {
			let tasks = self.tasks.drain();
			if tasks.is_empty() {
				break;
			}
			for task in tasks {
				match task {
					Task::DownloadSkymaps => {
						self.download_skymaps(&par.executor)?;
					}
					Task::Refresh => {
						self.refresh(&mut par.layout)?;
					}
					Task::SetSkymap(entry) => {
						self.set_skymap(entry)?;
					}
				}
			}
		}

		Ok(())
	}
}

impl View {
	pub fn new(params: Params) -> anyhow::Result<Self> {
		let doc_params = &ParseDocumentParams {
			globals: params.globals.clone(),
			path: AssetPath::BuiltIn("gui/view/skymap_list.xml"),
			extra: Default::default(),
		};

		let parser_state = wgui::parser::parse_from_assets(doc_params, params.layout, params.parent_id)?;
		let list_parent = parser_state.fetch_widget(&params.layout.state, "list_parent")?.id;
		let tasks = Tasks::new();

		tasks.push(Task::Refresh);

		tasks.handle_button(
			&parser_state.fetch_component_as::<ComponentButton>("btn_download_skymaps")?,
			Task::DownloadSkymaps,
		);

		tasks.handle_button(
			&parser_state.fetch_component_as::<ComponentButton>("btn_refresh")?,
			Task::Refresh,
		);

		Ok(Self {
			parser_state,
			tasks,
			list_parent,
			frontend_tasks: params.frontend_tasks.clone(),
			globals: params.globals.clone(),
			popup_remote_skymap_list: Default::default(),
			cells: Vec::new(),
		})
	}

	fn download_skymaps(&mut self, executor: &AsyncExecutor) -> anyhow::Result<()> {
		views::remote_skymap_list::mount_popup(
			self.frontend_tasks.clone(),
			executor.clone(),
			self.globals.clone(),
			self.tasks.make_callback_rc(Task::Refresh), /* on_updated_library */
			self.popup_remote_skymap_list.clone(),
		);
		Ok(())
	}

	fn set_skymap(&mut self, entry: SkymapCatalogEntry) -> anyhow::Result<()> {
		let skymap_file_path = entry
			.get_destination_path(SkymapResolution::Res2k /* todo: resolution selector */)
			.context("Skymap not found" /* you shouldn't really see this, like ever. */)?;

		log::error!(
			"not implemented (skymap path to be loaded: {})",
			skymap_file_path.to_string_lossy()
		);

		Ok(())
	}

	fn refresh(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		let entries = match skymap_catalog::get_entries_from_disk() {
			Ok(entries) => entries,
			Err(e) => {
				log::error!("failed to get skymap entries: {}", e);
				Default::default()
			}
		};

		layout.remove_children(self.list_parent);
		self.cells.clear();

		if entries.is_empty() {
			wgui_simple::create_label(
				layout,
				self.list_parent,
				Translation::from_translation_key("APP_SETTINGS.NO_SKYMAPS_FOUND"),
			)?;
			return Ok(());
		}

		let skymaps_root = config_io::get_skymaps_root();

		for entry in &entries {
			let mut view = views::skymap_list_cell::View::new(views::skymap_list_cell::Params {
				id_parent: self.list_parent,
				layout,
				entry: entry.clone(),
				on_click: self.tasks.get_button_click_callback(Task::SetSkymap(entry.clone())),
			})?;

			// load preview image
			if let Ok(data) = std::fs::read(skymaps_root.join(&entry.files.preview)) {
				if let Ok(glyph_data) = CustomGlyphData::from_bytes_raster(&self.globals, &entry.files.preview, &data) {
					view.set_image(layout, Some(glyph_data))?;
				}
			}

			self.cells.push(Cell { view });
		}

		Ok(())
	}
}

pub fn mount_popup(frontend_tasks: FrontendTasks, globals: WguiGlobals, popup: PopupHolder<View>) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_translation_key("APP_SETTINGS.BROWSE_SKYMAPS"),
			Box::new(move |data| {
				let view = View::new(Params {
					globals: globals.clone(),
					layout: data.layout,
					parent_id: data.id_content,
					frontend_tasks: &frontend_tasks,
				})?;

				popup.set_view(data.handle, view, None);
				Ok(popup.get_close_callback(data.layout))
			}),
		)));
}
