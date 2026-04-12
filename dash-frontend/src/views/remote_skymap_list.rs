use std::rc::Rc;

use wgui::{
	assets::AssetPath,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams},
	renderer_vk::text::custom_glyph::CustomGlyphData,
	task::Tasks,
};
use wlx_common::async_executor::AsyncExecutor;

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::{
		networking::{
			self,
			skymap_catalog::{SkymapCatalogEntry, SkymapUuid},
		},
		popup_manager::{MountPopupOnceParams, PopupHolder},
		wgui_simple,
	},
	views::{self, ViewTrait, ViewUpdateParams},
};

pub struct Params<'a> {
	pub globals: &'a WguiGlobals,
	pub layout: &'a mut Layout,
	pub executor: &'a AsyncExecutor,
	pub parent_id: WidgetID,
	pub on_close_request: Box<dyn FnOnce()>,
	pub frontend_tasks: FrontendTasks,
}

#[derive(Clone)]
enum Task {
	SetSkymapCatalog(Rc<anyhow::Result<networking::skymap_catalog::SkymapCatalog>>),
	SetSkymapPreview((SkymapUuid, Option<CustomGlyphData>)),
	ShowRemoteSkymapDownloader(SkymapUuid),
}

struct MountedCell {
	skymap_uuid: SkymapUuid,
	view: views::skymap_list_cell::View,
}

pub struct View {
	id_parent: WidgetID,
	id_list: WidgetID,
	id_loading: WidgetID,
	globals: WguiGlobals,
	tasks: Tasks<Task>,
	mounted_cells: Vec<MountedCell>,
	executor: AsyncExecutor,
	frontend_tasks: FrontendTasks,
	catalog: Option<networking::skymap_catalog::SkymapCatalog>,
	popup_remote_skymap_downloader: PopupHolder<views::remote_skymap_downloader::View>,
}

impl ViewTrait for View {
	fn update(&mut self, par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		self.popup_remote_skymap_downloader.update(par)?;

		for task in self.tasks.drain() {
			match task {
				Task::SetSkymapCatalog(skymap_catalog) => {
					par.layout.remove_widget(self.id_loading);
					match &*skymap_catalog {
						Ok(skymap_catalog) => {
							self.mount_catalog(par.layout, skymap_catalog)?;
						}
						Err(e) => wgui_simple::create_label_error(
							par.layout,
							self.id_parent,
							format!("Failed to fetch skymap catalog: {:?}", e),
						)?,
					}
				}
				Task::SetSkymapPreview((skymap_uuid, glyph_data)) => {
					if let Some(cell) = &mut self
						.mounted_cells
						.iter_mut()
						.find(|cell| cell.skymap_uuid == skymap_uuid)
					{
						cell.view.set_image(par.layout, glyph_data)?;
					}
				}
				Task::ShowRemoteSkymapDownloader(skymap_uuid) => {
					let preview_image = self.get_image_preview(skymap_uuid);
					self.show_remote_skymap_downloader(skymap_uuid, preview_image)?;
				}
			}
		}
		Ok(())
	}
}

impl View {
	async fn skymap_catalog_request_wrapper(tasks: Tasks<Task>, executor: AsyncExecutor) {
		let res = networking::skymap_catalog::request_catalog(&executor).await;
		tasks.push(Task::SetSkymapCatalog(Rc::new(res)))
	}

	pub fn new(par: Params) -> anyhow::Result<Self> {
		let id_loading = wgui_simple::create_loading(wgui_simple::CreateLoadingParams {
			layout: par.layout,
			parent_id: par.parent_id,
			with_text: true,
		})?;
		let tasks = Tasks::<Task>::new();
		let fut = View::skymap_catalog_request_wrapper(tasks.clone(), par.executor.clone());
		par.executor.spawn(fut).detach();
		Ok(Self {
			id_parent: par.parent_id,
			id_list: WidgetID::default(),
			id_loading,
			tasks,
			globals: par.globals.clone(),
			mounted_cells: Vec::new(),
			executor: par.executor.clone(),
			frontend_tasks: par.frontend_tasks,
			catalog: None,
			popup_remote_skymap_downloader: Default::default(),
		})
	}

	async fn request_skymap_preview(
		globals: WguiGlobals,
		executor: AsyncExecutor,
		entry: SkymapCatalogEntry,
		tasks: Tasks<Task>,
	) {
		let glyph_data = networking::image_fetch::fetch_to_glyph_data(&globals, &executor, &entry.files.get_url_preview())
			.await
			.ok();
		tasks.push(Task::SetSkymapPreview((entry.uuid, glyph_data)));
	}

	fn mount_catalog(
		&mut self,
		layout: &mut Layout,
		catalog: &networking::skymap_catalog::SkymapCatalog,
	) -> anyhow::Result<()> {
		let doc_params = &ParseDocumentParams {
			globals: self.globals.clone(),
			path: AssetPath::BuiltIn("gui/view/remote_skymap_list.xml"),
			extra: Default::default(),
		};

		let parser_state = wgui::parser::parse_from_assets(&doc_params, layout, self.id_parent)?;

		let id_list = parser_state.fetch_widget(&layout.state, "list")?.id;

		for entry in &catalog.entries {
			let task = View::request_skymap_preview(
				self.globals.clone(),
				self.executor.clone(),
				entry.clone(),
				self.tasks.clone(),
			);

			let skymap_uuid = entry.uuid.clone();

			self.mounted_cells.push(MountedCell {
				skymap_uuid: entry.uuid.clone(),
				view: views::skymap_list_cell::View::new(views::skymap_list_cell::Params {
					id_parent: id_list,
					layout,
					entry: entry.clone(),
					on_click: self
						.tasks
						.get_button_click_callback(Task::ShowRemoteSkymapDownloader(skymap_uuid)),
				})?,
			});

			self.executor.spawn(task).detach();
		}

		self.catalog = Some(catalog.clone());
		Ok(())
	}

	fn show_remote_skymap_downloader(
		&mut self,
		uuid: SkymapUuid,
		preview_image: Option<CustomGlyphData>,
	) -> anyhow::Result<()> {
		let Some(catalog) = &self.catalog else {
			debug_assert!(false); // impossible
			return Ok(());
		};

		let Some(entry) = catalog.entries.iter().find(|entry| entry.uuid == uuid) else {
			debug_assert!(false); // impossible
			return Ok(());
		};

		views::remote_skymap_downloader::mount_popup(
			self.frontend_tasks.clone(),
			self.executor.clone(),
			self.globals.clone(),
			entry.clone(),
			preview_image,
			self.popup_remote_skymap_downloader.clone(),
		);

		Ok(())
	}

	fn get_image_preview(&self, skymap_uuid: SkymapUuid) -> Option<CustomGlyphData> {
		if let Some(cell) = &self.mounted_cells.iter().find(|mc| mc.skymap_uuid == skymap_uuid) {
			return cell.view.get_image();
		}
		None
	}
}

pub fn mount_popup(
	frontend_tasks: FrontendTasks,
	executor: AsyncExecutor,
	globals: WguiGlobals,
	popup: PopupHolder<View>,
) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_translation_key("APP_SETTINGS.DOWNLOAD_SKYMAPS"),
			Box::new(move |data| {
				let on_close_request = popup.get_close_callback(data.layout);
				let view = View::new(Params {
					globals: &globals,
					layout: data.layout,
					executor: &executor,
					parent_id: data.id_content,
					on_close_request,
					frontend_tasks,
				})?;

				popup.set_view(data.handle, view);
				Ok(popup.get_close_callback(data.layout))
			}),
		)));
}
