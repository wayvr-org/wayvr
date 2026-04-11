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

use crate::{
	frontend::FrontendTasks,
	util::{popup_manager::PopupHolder, wgui_simple},
	views,
};

#[derive(Clone)]
enum Task {
	DownloadSkymaps,
	Refresh,
}

pub struct Params<'a> {
	pub globals: WguiGlobals,
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub frontend_tasks: &'a FrontendTasks,
}

pub struct View {
	#[allow(dead_code)]
	parser_state: ParserState,
	tasks: Tasks<Task>,
	list_parent: WidgetID,
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	popup_remote_skymap_list: PopupHolder<views::remote_skymap_list::View>,
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
		})
	}

	pub fn update(&mut self, layout: &mut Layout, executor: &AsyncExecutor) -> anyhow::Result<()> {
		self
			.popup_remote_skymap_list
			.with_view_res(|view| view.update(layout))?;

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
				}
			}
		}

		Ok(())
	}

	fn download_skymaps(&mut self, executor: &AsyncExecutor) -> anyhow::Result<()> {
		views::remote_skymap_list::mount_popup(
			self.frontend_tasks.clone(),
			executor.clone(),
			self.globals.clone(),
			self.popup_remote_skymap_list.clone(),
		);
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
