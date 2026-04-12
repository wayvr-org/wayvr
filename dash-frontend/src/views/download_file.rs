use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::{
		networking::http_client::{self, ProgressFuncData},
		popup_manager::{MountPopupOnceParams, PopupHolder},
		wgui_simple,
	},
	views::{ViewTrait, ViewUpdateParams},
};
use glam::Vec2;
use std::path::PathBuf;
use wgui::{
	assets::AssetPath,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	task::Tasks,
	widget::label::WidgetLabel,
};
use wlx_common::async_executor::AsyncExecutor;

pub struct Params<'a> {
	pub globals: &'a WguiGlobals,
	pub layout: &'a mut Layout,
	pub executor: &'a AsyncExecutor,
	pub parent_id: WidgetID,
	pub target_path: PathBuf,
	pub url: String,
	pub on_close_request: Box<dyn FnOnce()>,
}

#[derive(Clone)]
enum Task {
	StartDownload(/*url*/ String),
	SetStatusText(String),
	ShowIconSuccess,
	ShowIconError,
}

pub struct View {
	id_parent: WidgetID,
	globals: WguiGlobals,
	tasks: Tasks<Task>,
	executor: AsyncExecutor,

	#[allow(dead_code)]
	parser_state: ParserState,

	id_label_status: WidgetID,
	id_loading_parent: WidgetID,
}

impl ViewTrait for View {
	fn update(&mut self, par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		for task in self.tasks.drain() {
			match task {
				Task::StartDownload(url) => {
					self
						.executor
						.spawn(View::download(self.tasks.clone(), self.executor.clone(), url))
						.detach();
				}
				Task::SetStatusText(text) => {
					let widgets = &mut par.layout.state.widgets;
					widgets
						.fetch(self.id_label_status)?
						.cast::<WidgetLabel>()?
						.set_text(&mut par.layout.common(), Translation::from_raw_text_string(text));
				}
				Task::ShowIconSuccess => {
					par.layout.remove_children(self.id_loading_parent);
					wgui_simple::create_icon(
						par.layout,
						self.id_loading_parent,
						Vec2::splat(32.0),
						AssetPath::BuiltIn("dashboard/check.svg"),
					)?;
				}
				Task::ShowIconError => {
					par.layout.remove_children(self.id_loading_parent);
					wgui_simple::create_icon(
						par.layout,
						self.id_loading_parent,
						Vec2::splat(32.0),
						AssetPath::BuiltIn("dashboard/error.svg"),
					)?;
				}
			}
		}
		Ok(())
	}
}

impl View {
	pub fn new(par: Params) -> anyhow::Result<Self> {
		let tasks = Tasks::<Task>::new();

		let doc_params = ParseDocumentParams {
			globals: par.globals.clone(),
			path: AssetPath::BuiltIn("gui/view/download_file.xml"),
			extra: Default::default(),
		};

		let parser_state = wgui::parser::parse_from_assets(&doc_params, par.layout, par.parent_id)?;
		let id_label_status = parser_state.get_widget_id("label_status")?;
		let id_loading_parent = parser_state.get_widget_id("loading_parent")?;

		wgui_simple::create_loading(wgui_simple::CreateLoadingParams {
			parent_id: id_loading_parent,
			layout: par.layout,
			with_text: false,
		})?;

		let str_target_path = par.globals.i18n().translate("TARGET_PATH");

		{
			let label_target_path = parser_state
				.fetch_widget(&par.layout.state, "label_target_path")?
				.widget;
			label_target_path.cast::<WidgetLabel>()?.set_text(
				&mut par.layout.common(),
				Translation::from_raw_text_string(format!("{}: {}", str_target_path, par.target_path.display())),
			);
		}

		tasks.push(Task::StartDownload(par.url.clone()));

		Ok(Self {
			id_parent: par.parent_id,
			tasks,
			globals: par.globals.clone(),
			executor: par.executor.clone(),
			parser_state,
			id_label_status,
			id_loading_parent,
		})
	}

	async fn download(tasks: Tasks<Task>, executor: AsyncExecutor, url: String) {
		tasks.push(Task::SetStatusText(String::from("Connecting to the server...")));

		let res = http_client::get(http_client::GetParams {
			executor: &executor,
			url: &url,
			on_progress: Some(Box::new({
				let tasks = tasks.clone();
				move |data: ProgressFuncData| {
					tasks.push(Task::SetStatusText(format!(
						"{}/{} KiB ({}%)",
						data.bytes_downloaded / 1024,
						data.file_size / 1024,
						(data.bytes_downloaded as f32 / data.file_size as f32 * 100.0).round()
					)))
				}
			})),
		})
		.await;

		match res {
			Ok(_response) => {
				tasks.push(Task::SetStatusText(String::from("Download finished")));
				tasks.push(Task::ShowIconSuccess);
			}
			Err(e) => {
				tasks.push(Task::ShowIconError);
				tasks.push(Task::SetStatusText(format!("Download failed: {:?}", e)))
			}
		}
	}
}

pub fn mount_popup(
	frontend_tasks: FrontendTasks,
	executor: AsyncExecutor,
	globals: WguiGlobals,
	popup: PopupHolder<View>,
	target_path: PathBuf,
	url: String,
) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_translation_key("DOWNLOADER"),
			Box::new(move |data| {
				let on_close_request = popup.get_close_callback(data.layout);
				let view = View::new(Params {
					globals: &globals,
					layout: data.layout,
					executor: &executor,
					parent_id: data.id_content,
					on_close_request,
					target_path,
					url,
				})?;

				popup.set_view(data.handle, view);
				Ok(popup.get_close_callback(data.layout))
			}),
		)));
}
