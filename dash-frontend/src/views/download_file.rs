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
	components::button::ComponentButton,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	task::Tasks,
	widget::label::WidgetLabel,
};
use wlx_common::async_executor::AsyncExecutor;

pub struct Params {
	pub globals: WguiGlobals,
	pub executor: AsyncExecutor,
	pub target_path: PathBuf,
	pub url: String,
	pub on_downloaded: Box<dyn FnOnce()>,
}

#[derive(Clone)]
enum Task {
	StartDownload(/*url*/ String, /*target path*/ PathBuf),
	SetStatusText(String),
	ShowIconSuccess,
	ShowIconError,
	Close,
}

pub struct View {
	globals: WguiGlobals,
	tasks: Tasks<Task>,
	executor: AsyncExecutor,

	#[allow(dead_code)]
	parser_state: ParserState,

	id_label_status: WidgetID,
	id_loading_parent: WidgetID,
	id_content: WidgetID,
	on_close_request: Option<Box<dyn FnOnce()>>,
	on_downloaded: Option<Box<dyn FnOnce()>>,
}

fn doc_params(globals: &WguiGlobals) -> ParseDocumentParams<'_> {
	ParseDocumentParams {
		globals: globals.clone(),
		path: AssetPath::BuiltIn("gui/view/download_file.xml"),
		extra: Default::default(),
	}
}

impl ViewTrait for View {
	fn update(&mut self, par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		for task in self.tasks.drain() {
			match task {
				Task::StartDownload(url, path) => {
					if let Some(on_downloaded) = self.on_downloaded.take() {
						self
							.executor
							.spawn(View::download(
								self.tasks.clone(),
								self.executor.clone(),
								url,
								path,
								on_downloaded,
							))
							.detach();
					}
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

					// "Close window" button
					self
						.parser_state
						.realize_template(
							&doc_params(&self.globals),
							"btn_close",
							par.layout,
							self.id_content,
							Default::default(),
						)?
						.fetch_component_as::<ComponentButton>("btn")?
						.on_click(self.tasks.get_button_click_callback(Task::Close));
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
				Task::Close => {
					if let Some(on_close) = self.on_close_request.take() {
						on_close();
					}
				}
			}
		}
		Ok(())
	}
}

fn handle_async_result<T, E>(error_reason: &'static str, tasks: &Tasks<Task>, result: anyhow::Result<T, E>) -> Option<T>
where
	E: std::fmt::Debug,
{
	match result {
		Ok(res) => Some(res),
		Err(e) => {
			tasks.push(Task::ShowIconError);
			tasks.push(Task::SetStatusText(format!("{}: {:?}", error_reason, e)));
			None
		}
	}
}

impl View {
	pub fn new(
		layout: &mut Layout,
		id_parent: WidgetID,
		on_close_request: Box<dyn FnOnce()>,
		par: Params,
	) -> anyhow::Result<Self> {
		let tasks = Tasks::<Task>::new();

		let parser_state = wgui::parser::parse_from_assets(&doc_params(&par.globals), layout, id_parent)?;
		let id_label_status = parser_state.get_widget_id("label_status")?;
		let id_content = parser_state.get_widget_id("content")?;
		let id_loading_parent = parser_state.get_widget_id("loading_parent")?;

		wgui_simple::create_loading(wgui_simple::CreateLoadingParams {
			parent_id: id_loading_parent,
			layout: layout,
			with_text: false,
		})?;

		let str_target_path = par.globals.i18n().translate("TARGET_PATH");

		{
			let label_target_path = parser_state.fetch_widget(&layout.state, "label_target_path")?.widget;
			label_target_path.cast::<WidgetLabel>()?.set_text(
				&mut layout.common(),
				Translation::from_raw_text_string(format!("{}: {}", str_target_path, par.target_path.display())),
			);
		}

		tasks.push(Task::StartDownload(par.url, par.target_path));

		Ok(Self {
			tasks,
			globals: par.globals.clone(),
			executor: par.executor.clone(),
			parser_state,
			id_label_status,
			id_loading_parent,
			id_content,
			on_close_request: Some(on_close_request),
			on_downloaded: Some(par.on_downloaded),
		})
	}

	async fn download(
		tasks: Tasks<Task>,
		executor: AsyncExecutor,
		url: String,
		target_path: PathBuf,
		on_downloaded: Box<dyn FnOnce()>,
	) -> Option<()> {
		tasks.push(Task::SetStatusText(String::from("Connecting to the server...")));

		// start downloading from the server with progress reporting
		let res = handle_async_result(
			"Download failed",
			&tasks,
			http_client::get(http_client::GetParams {
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
			.await,
		)?;

		tasks.push(Task::SetStatusText(String::from("Writing to file...")));

		// create skymaps directory if it doesn't exist yet
		if let Some(parent) = target_path.parent() {
			handle_async_result(
				"Directory creation failed",
				&tasks,
				smol::fs::create_dir_all(parent).await,
			)?;
		}

		handle_async_result(
			"File write failed",
			&tasks,
			smol::fs::write(target_path, res.data).await,
		)?;

		tasks.push(Task::SetStatusText(String::from("Download finished")));
		tasks.push(Task::ShowIconSuccess);

		on_downloaded();

		None
	}
}

pub fn mount_popup(
	popup: PopupHolder<View>,
	frontend_tasks: FrontendTasks,
	on_view_close: Box<dyn FnOnce()>,
	params: Params,
) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_translation_key("DOWNLOADER"),
			Box::new(move |data| {
				let on_close_request = popup.get_close_callback(data.layout);
				let view = View::new(data.layout, data.id_content, on_close_request, params)?;

				popup.set_view(data.handle, view, Some(on_view_close));
				Ok(popup.get_close_callback(data.layout))
			}),
		)));
}
