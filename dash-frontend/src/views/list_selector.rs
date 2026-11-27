use std::rc::Rc;
use wgui::{
	components::list,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	task::Tasks,
	widget::ConstructEssentials,
};

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::popup_manager::{MountPopupOnceParams, PopupHolder},
	views::{ViewTrait, ViewUpdateParams},
};

#[derive(Clone)]
enum Task {
	Select(Rc<str>),
}

pub struct OnFinishResult {
	pub cell: Rc<str>,
}

pub type OnFinishCallback = Box<dyn FnOnce(OnFinishResult)>;

pub struct Params<'a> {
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub cells: Vec<list::Cell>,
	pub on_finish: OnFinishCallback,
	pub close_callback: Box<dyn FnOnce()>,
}

pub struct View {
	tasks: Tasks<Task>,
	_parser_state: ParserState,
	selected_cell: Option<Rc<str>>,
	on_finish: Option<OnFinishCallback>,
	close_callback: Option<Box<dyn FnOnce()>>,
}

impl ViewTrait for View {
	fn update(&mut self, _par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		for task in self.tasks.drain() {
			match task {
				Task::Select(id) => {
					self.selected_cell = Some(id.clone());
					let Some(on_finish) = self.on_finish.take() else {
						continue;
					};
					on_finish(OnFinishResult { cell: id });

					if let Some(c) = self.close_callback.take() {
						c();
					}
				}
			}
		}

		Ok(())
	}
}

impl View {
	pub fn new(params: Params) -> anyhow::Result<Self> {
		let doc_params = ParseDocumentParams::builtin("gui/view/list_selector.xml");

		let parser_state = wgui::parser::parse_from_assets(&doc_params, params.layout, params.parent_id)?;
		let id_list_parent = parser_state.get_widget_id("list_parent")?;

		let tasks = Tasks::new();

		list::construct(
			&mut ConstructEssentials {
				layout: params.layout,
				parent: id_list_parent,
			},
			list::Params {
				cells: params.cells,
				on_click: {
					let tasks = tasks.clone();
					Some(Rc::new(move |_, e| {
						tasks.push(Task::Select(e.id));
						Ok(())
					}))
				},
				..Default::default()
			},
		)?;

		Ok(Self {
			tasks,
			selected_cell: None,
			_parser_state: parser_state,
			on_finish: Some(params.on_finish),
			close_callback: Some(params.close_callback),
		})
	}
}

pub fn mount_popup(
	popup: PopupHolder<View>,
	frontend_tasks: FrontendTasks,
	title: Translation,
	cells: Vec<list::Cell>,
	on_finish: OnFinishCallback,
) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			title,
			Box::new(move |data| {
				let close_callback = popup.get_close_callback(data.layout);
				let view = View::new(Params {
					layout: data.layout,
					parent_id: data.id_content,
					cells,
					close_callback,
					on_finish,
				})?;

				popup.set_view(data.handle, view, None);
				Ok(popup.get_close_callback(data.layout))
			}),
			Default::default(),
		)));
}
