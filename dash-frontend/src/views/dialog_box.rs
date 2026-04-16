use std::{collections::HashMap, rc::Rc};

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::popup_manager::{MountPopupOnceParams, PopupHolder},
	views::{ViewTrait, ViewUpdateParams},
};
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

pub struct ButtonEntry {
	pub content: Translation, // button text
	pub icon: &'static str,   // sprite_src_builtin
	pub action: &'static str, // action name (will be passed into on_action_click)
}

pub struct Params {
	pub globals: WguiGlobals,
	pub entries: Vec<ButtonEntry>,
	pub message: Translation,
	pub on_action_click: Box<dyn FnOnce(&'static str)>,
}

#[derive(Clone)]
enum Task {
	ActionClicked(&'static str),
}

pub struct View {
	tasks: Tasks<Task>,

	#[allow(dead_code)]
	parser_state: ParserState,

	on_action_click: Option<Box<dyn FnOnce(&'static str)>>,
	on_close_request: Option<Box<dyn FnOnce()>>,
}

fn doc_params(globals: &WguiGlobals) -> ParseDocumentParams<'_> {
	ParseDocumentParams {
		globals: globals.clone(),
		path: AssetPath::BuiltIn("gui/view/dialog_box.xml"),
		extra: Default::default(),
	}
}

impl ViewTrait for View {
	fn update(&mut self, _par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		for task in self.tasks.drain() {
			match task {
				Task::ActionClicked(action) => {
					if let Some(func) = self.on_action_click.take() {
						func(action);
					}

					if let Some(on_close) = self.on_close_request.take() {
						on_close();
					}
				}
			}
		}
		Ok(())
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

		let mut parser_state = wgui::parser::parse_from_assets(&doc_params(&par.globals), layout, id_parent)?;
		let id_buttons = parser_state.get_widget_id("buttons")?;

		{
			let label_message = parser_state.fetch_widget(&layout.state, "label_message")?.widget;
			label_message
				.cast::<WidgetLabel>()?
				.set_text(&mut layout.common(), par.message);
		}

		for entry in par.entries {
			let mut t_par = HashMap::<Rc<str>, Rc<str>>::new();
			t_par.insert(Rc::from("icon"), Rc::from(entry.icon));

			let data =
				parser_state.realize_template(&doc_params(&par.globals), "DialogBoxButton", layout, id_buttons, t_par)?;

			let button = data.fetch_component_as::<ComponentButton>("btn")?;
			button.set_text(&mut layout.common(), entry.content.clone());
			button.on_click(tasks.get_button_click_callback(Task::ActionClicked(entry.action)));
		}

		Ok(Self {
			tasks,
			parser_state,
			on_action_click: Some(par.on_action_click),
			on_close_request: Some(on_close_request),
		})
	}
}

pub fn mount_popup(popup: PopupHolder<View>, frontend_tasks: FrontendTasks, params: Params) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_raw_text("Info"),
			Box::new(move |data| {
				let on_close_request = popup.get_close_callback(data.layout);
				let view = View::new(data.layout, data.id_content, on_close_request, params)?;

				popup.set_view(data.handle, view, None);
				Ok(popup.get_close_callback(data.layout))
			}),
		)));
}
