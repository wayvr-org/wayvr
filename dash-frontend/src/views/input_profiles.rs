use std::{collections::HashMap, rc::Rc};

use anyhow::Context;
use wgui::{
	assets::AssetPath,
	components::button::ComponentButton,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams},
	task::Tasks,
};

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::{
		openxr_bindings_schema,
		popup_manager::{MountPopupOnceParams, PopupHolder},
	},
	views::{self, ViewTrait, ViewUpdateParams, bindings},
};

#[derive(Clone)]
enum Task {
	SelectProfile(Rc<str>),
	Cancel,
}

pub struct Params<'a> {
	pub globals: WguiGlobals,
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub frontend_tasks: &'a FrontendTasks,
	pub close_callback: Box<dyn FnOnce()>,
}

pub struct View {
	tasks: Tasks<Task>,
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	close_callback: Option<Box<dyn FnOnce()>>,
	bindings_popup: PopupHolder<bindings::View>,
	bindings_file: openxr_bindings_schema::BindingsFile,
}

impl ViewTrait for View {
	fn update(&mut self, par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		self.bindings_popup.update(par)?;

		for task in self.tasks.drain() {
			match task {
				Task::SelectProfile(profile_id) => {
					let profile = self
						.bindings_file
						.profiles
						.get(&*profile_id)
						.context("Selected non-existing profile. UI bug?")?;

					views::bindings::mount_popup(
						self.frontend_tasks.clone(),
						self.globals.clone(),
						self.bindings_popup.clone(),
						profile_id.clone(),
						profile.clone(),
					);
				}
				Task::Cancel => {
					if let Some(close_callback) = self.close_callback.take() {
						close_callback();
					}
				}
			}
		}
		Ok(())
	}
}

impl View {
	pub fn new(params: Params) -> anyhow::Result<Self> {
		let doc_params = &ParseDocumentParams {
			globals: params.globals.clone(),
			path: AssetPath::BuiltIn("gui/view/input_profiles.xml"),
			extra: Default::default(),
		};

		let mut parser_state = wgui::parser::parse_from_assets(doc_params, params.layout, params.parent_id)?;

		let list_parent = parser_state.fetch_widget(&params.layout.state, "list_parent")?.id;

		let tasks = Tasks::new();

		tasks.handle_button(
			&parser_state.fetch_component_as::<ComponentButton>("btn_cancel")?,
			Task::Cancel,
		);

		let bindings_file = openxr_bindings_schema::BindingsFile::load_embedded();

		for (idx, (profile_id, profile)) in bindings_file.profiles.iter().enumerate() {
			let id = format!("profile_btn_{idx}");
			let profile_name: Rc<str> = profile.title.clone();

			let mut cell_params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
			cell_params.insert(Rc::from("id"), Rc::from(id.clone()));
			cell_params.insert(Rc::from("text"), Rc::from(profile_name));

			parser_state.instantiate_template(doc_params, "ProfileButton", params.layout, list_parent, cell_params)?;

			let btn = parser_state.fetch_component_as::<ComponentButton>(&id)?;
			let tasks_clone = tasks.clone();
			btn.on_click(Rc::new({
				let profile_id: Rc<str> = profile_id.clone().into();
				move |_common, _e| {
					tasks_clone.push(Task::SelectProfile(profile_id.clone()));
					Ok(())
				}
			}));
		}

		Ok(Self {
			tasks,
			frontend_tasks: params.frontend_tasks.clone(),
			globals: params.globals.clone(),
			close_callback: Some(params.close_callback),
			bindings_popup: Default::default(),
			bindings_file,
		})
	}
}

pub fn mount_popup(frontend_tasks: FrontendTasks, globals: WguiGlobals, popup: PopupHolder<View>) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_translation_key("APP_SETTINGS.INPUT_PROFILES"),
			Box::new(move |data| {
				let close_callback = popup.get_close_callback(data.layout);
				let view = View::new(Params {
					globals: globals.clone(),
					layout: data.layout,
					parent_id: data.id_content,
					frontend_tasks: &frontend_tasks,
					close_callback,
				})?;

				popup.set_view(data.handle, view, None);
				Ok(popup.get_close_callback(data.layout))
			}),
		)));
}
