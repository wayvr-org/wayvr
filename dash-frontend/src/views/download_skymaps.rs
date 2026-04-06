use wgui::{
	assets::AssetPath,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams},
	task::Tasks,
};
use wlx_common::async_executor::AsyncExecutor;

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::{
		networking::{self},
		popup_manager::{MountPopupOnceParams, PopupHolder},
		wgui_simple,
	},
	views,
};

pub struct Params<'a> {
	pub globals: &'a WguiGlobals,
	pub layout: &'a mut Layout,
	pub executor: &'a AsyncExecutor,
	pub parent_id: WidgetID,
	pub on_close_request: Box<dyn Fn()>,
}

enum Task {
	SetSkymapCatalog(anyhow::Result<networking::skymap_catalog::SkymapCatalog>),
}

pub struct View {
	id_parent: WidgetID,
	id_list: WidgetID,
	id_loading: WidgetID,
	globals: WguiGlobals,
	tasks: Tasks<Task>,
	mounted_cells: Vec<views::skymap_list_cell::View>,
}

impl View {
	async fn skymap_catalog_request_wrapper(tasks: Tasks<Task>, executor: AsyncExecutor) {
		let res = networking::skymap_catalog::request_catalog(&executor).await;
		tasks.push(Task::SetSkymapCatalog(res))
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
		})
	}

	fn mount_catalog(
		&mut self,
		layout: &mut Layout,
		catalog: networking::skymap_catalog::SkymapCatalog,
	) -> anyhow::Result<()> {
		let doc_params = &ParseDocumentParams {
			globals: self.globals.clone(),
			path: AssetPath::BuiltIn("gui/view/download_skymaps.xml"),
			extra: Default::default(),
		};

		let parser_state = wgui::parser::parse_from_assets(&doc_params, layout, self.id_parent)?;

		let id_list = parser_state.fetch_widget(&layout.state, "list")?.id;

		for entry in catalog.entries {
			let view_cell = views::skymap_list_cell::View::new(views::skymap_list_cell::Params {
				id_parent: id_list,
				layout,
				entry,
			})?;
			self.mounted_cells.push(view_cell);
		}

		Ok(())
	}

	pub fn update(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		for task in self.tasks.drain() {
			match task {
				Task::SetSkymapCatalog(skymap_catalog) => {
					layout.remove_widget(self.id_loading);
					match skymap_catalog {
						Ok(skymap_catalog) => {
							self.mount_catalog(layout, skymap_catalog)?;
						}
						Err(e) => wgui_simple::create_label_error(
							layout,
							self.id_parent,
							format!("Failed to fetch skymap catalog: {:?}", e),
						)?,
					}
				}
			}
		}
		Ok(())
	}
}

pub fn mount_popup(
	frontend_tasks: FrontendTasks,
	executor: AsyncExecutor,
	globals: WguiGlobals,
	on_close_request: Box<dyn Fn()>,
	set_holder: Box<dyn FnOnce(PopupHolder<View>)>,
) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_translation_key("APP_SETTINGS.DOWNLOAD_SKYMAPS"),
			Box::new(move |data| {
				let view = View::new(Params {
					globals: &globals,
					layout: data.layout,
					executor: &executor,
					parent_id: data.id_content,
					on_close_request,
				})?;

				set_holder((data.handle, view));
				Ok(())
			}),
		)));
}
