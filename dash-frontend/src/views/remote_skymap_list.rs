use std::rc::Rc;

use uuid::Uuid;
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
			skymap_catalog::{SkymapCatalog, SkymapCatalogEntry, SkymapUuid},
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
	pub frontend_tasks: FrontendTasks,
	pub on_updated_library: Rc<dyn Fn()>,
}

#[derive(Clone)]
enum Task {
	SetSkymapCatalog(Rc<anyhow::Result<networking::skymap_catalog::SkymapCatalog>>),
	SetSkymapPreview(
		(
			SkymapUuid,
			Option<(
				CustomGlyphData, /* ready-to-use preview image data */
				Rc<Vec<u8>>,     /* compressed preview image data (should weigh about 10-15 KiB) */
			)>,
		),
	),
	ShowRemoteSkymapDownloader(SkymapUuid),
	RefreshCells,
}

struct MountedCell {
	skymap_uuid: SkymapUuid,
	view: views::skymap_list_cell::View,
	preview_image_compressed: Option<Rc<Vec<u8>>>,
}

pub struct View {
	id_parent: WidgetID,
	id_loading: WidgetID,
	globals: WguiGlobals,
	tasks: Tasks<Task>,
	mounted_cells: Vec<MountedCell>,
	executor: AsyncExecutor,
	frontend_tasks: FrontendTasks,
	catalog: Option<SkymapCatalog>,
	popup_remote_skymap_downloader: PopupHolder<views::remote_skymap_downloader::View>,
	on_updated_library: Rc<dyn Fn()>,
}

fn get_entry_by_uuid(catalog: &SkymapCatalog, skymap_uuid: Uuid) -> Option<&SkymapCatalogEntry> {
	let Some(entry) = catalog.entries.iter().find(|entry| entry.uuid == skymap_uuid) else {
		return None;
	};

	Some(entry)
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
				Task::SetSkymapPreview((skymap_uuid, opt_preview_image)) => {
					if let Some(cell) = &mut self
						.mounted_cells
						.iter_mut()
						.find(|cell| cell.skymap_uuid == skymap_uuid)
					{
						if let Some((preview_image, preview_image_compressed)) = opt_preview_image {
							cell.view.set_image(par.layout, Some(preview_image))?;
							cell.preview_image_compressed = Some(preview_image_compressed);
						} else {
							cell.view.set_image(par.layout, None)?;
						}
					}
				}
				Task::ShowRemoteSkymapDownloader(skymap_uuid) => {
					if let Some((preview_image, preview_image_compressed)) = self.get_image_preview(skymap_uuid) {
						self.show_remote_skymap_downloader(skymap_uuid, preview_image, preview_image_compressed)?;
					} else {
						log::error!("preview image not present, ignoring request");
					}
				}
				Task::RefreshCells => {
					self.refresh_cells(par.layout)?;
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
			id_loading,
			tasks,
			globals: par.globals.clone(),
			mounted_cells: Vec::new(),
			executor: par.executor.clone(),
			frontend_tasks: par.frontend_tasks,
			catalog: None,
			popup_remote_skymap_downloader: Default::default(),
			on_updated_library: par.on_updated_library,
		})
	}

	async fn request_skymap_preview(
		globals: WguiGlobals,
		executor: AsyncExecutor,
		entry: SkymapCatalogEntry,
		tasks: Tasks<Task>,
	) {
		tasks.push(Task::SetSkymapPreview((
			entry.uuid,
			networking::image_fetch::fetch_to_glyph_data(&globals, &executor, &entry.files.get_url_preview())
				.await
				.ok(),
		)));
	}

	fn refresh_cells(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		let Some(catalog) = &self.catalog else {
			debug_assert!(false);
			return Ok(());
		};

		for cell in &mut self.mounted_cells {
			if let Some(entry) = get_entry_by_uuid(&catalog, cell.skymap_uuid) {
				cell.view.refresh_resolution_pips(layout, entry)?;
			}
		}

		Ok(())
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
				preview_image_compressed: None,
				skymap_uuid: entry.uuid.clone(),
				view: views::skymap_list_cell::View::new(views::skymap_list_cell::Params {
					id_parent: id_list,
					layout,
					entry: Some(entry.clone()),
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
		preview_image: CustomGlyphData,
		preview_image_compressed: Rc<Vec<u8>>,
	) -> anyhow::Result<()> {
		let Some(catalog) = &self.catalog else {
			debug_assert!(false); // impossible
			return Ok(());
		};

		let Some(entry) = get_entry_by_uuid(&catalog, uuid) else {
			return Ok(());
		};

		// call our task before calling underlying on_updated_library callback
		let on_updated_library = Rc::new({
			let func = self.on_updated_library.clone();
			let tasks = self.tasks.clone();
			move || {
				tasks.push(Task::RefreshCells);
				(*func)();
			}
		});

		views::remote_skymap_downloader::mount_popup(
			self.frontend_tasks.clone(),
			self.executor.clone(),
			self.globals.clone(),
			entry.clone(),
			preview_image,
			preview_image_compressed,
			on_updated_library,
			self.popup_remote_skymap_downloader.clone(),
		);

		Ok(())
	}

	fn get_image_preview(
		&self,
		skymap_uuid: SkymapUuid,
	) -> Option<(CustomGlyphData, Rc<Vec<u8>> /* preview_image_compressed */)> {
		if let Some(cell) = &self.mounted_cells.iter().find(|mc| mc.skymap_uuid == skymap_uuid) {
			let Some(image) = cell.view.get_image() else {
				return None;
			};

			let Some(preview_image_compressed) = cell.preview_image_compressed.clone() else {
				return None;
			};
			return Some((image, preview_image_compressed));
		}
		None
	}
}

pub fn mount_popup(
	frontend_tasks: FrontendTasks,
	executor: AsyncExecutor,
	globals: WguiGlobals,
	on_updated_library: Rc<dyn Fn()>,
	popup: PopupHolder<View>,
) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_translation_key("APP_SETTINGS.BROWSE_ONLINE_CATALOG"),
			Box::new(move |data| {
				let view = View::new(Params {
					globals: &globals,
					layout: data.layout,
					executor: &executor,
					parent_id: data.id_content,
					frontend_tasks,
					on_updated_library,
				})?;

				popup.set_view(data.handle, view, None);
				Ok(popup.get_close_callback(data.layout))
			}),
		)));
}
