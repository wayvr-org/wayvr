use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::popup_manager::{MountPopupOnceParams, PopupHolder},
	views::{ViewTrait, ViewUpdateParams},
};
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
enum Task {}

pub struct View {
	id_parent: WidgetID,
	globals: WguiGlobals,
	tasks: Tasks<Task>,
	executor: AsyncExecutor,

	#[allow(dead_code)]
	parser_state: ParserState,
}

impl ViewTrait for View {
	fn update(&mut self, _par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		for task in self.tasks.drain() {
			match task {}
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

		let mut parser_state = wgui::parser::parse_from_assets(&doc_params, par.layout, par.parent_id)?;

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

		Ok(Self {
			id_parent: par.parent_id,
			tasks,
			globals: par.globals.clone(),
			executor: par.executor.clone(),
			parser_state,
		})
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
