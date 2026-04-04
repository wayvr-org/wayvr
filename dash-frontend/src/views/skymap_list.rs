use wgui::{
	assets::AssetPath,
	components::button::ComponentButton,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	task::Tasks,
};
use wlx_common::{async_executor::AsyncExecutor, config_io};

use crate::util::{networking, wgui_simple};

#[derive(Clone)]
enum Task {
	DownloadSkymaps,
	SetSkymapCatalog(Result<networking::skymap_catalog::SkymapCatalog, String>),
	Refresh,
}

pub struct Params<'a> {
	pub globals: WguiGlobals,
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
}

pub struct View {
	#[allow(dead_code)]
	parser_state: ParserState,
	tasks: Tasks<Task>,
	list_parent: WidgetID,
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
		})
	}

	pub fn update(&mut self, layout: &mut Layout, executor: &AsyncExecutor) -> anyhow::Result<()> {
		loop {
			let tasks = self.tasks.drain();
			if tasks.is_empty() {
				break;
			}
			for task in tasks {
				match task {
					Task::DownloadSkymaps => {
						self.download_skymaps(executor)?;
					}
					Task::Refresh => {
						self.refresh(layout)?;
					}
					Task::SetSkymapCatalog(skymap_catalog) => {
						log::info!("{:?}", skymap_catalog);
					}
				}
			}
		}

		Ok(())
	}

	async fn skymap_catalog_request_wrapper(tasks: Tasks<Task>, executor: AsyncExecutor) {
		let res = networking::skymap_catalog::request_catalog(&executor).await;
		tasks.push(Task::SetSkymapCatalog(res.map_err(|e| format!("{}", e))));
	}

	fn download_skymaps(&mut self, executor: &AsyncExecutor) -> anyhow::Result<()> {
		let fut = View::skymap_catalog_request_wrapper(self.tasks.clone(), executor.clone());
		executor.spawn(fut).detach();
		Ok(())
	}

	fn refresh(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		let skymaps_uuids = config_io::get_skymaps_uuids().unwrap_or_default();
		log::info!("skymap uuids {:?}", skymaps_uuids);

		layout.remove_children(self.list_parent);

		if skymaps_uuids.is_empty() {
			wgui_simple::create_label(
				layout,
				self.list_parent,
				Translation::from_translation_key("APP_SETTINGS.NO_SKYMAPS_FOUND"),
			)?;
			return Ok(());
		}

		Ok(())
	}
}
