use std::{collections::HashMap, rc::Rc};

use wgui::{
	assets::AssetPath,
	components::button::ComponentButton,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams},
	task::Tasks,
};
use wlx_common::{openxr_bindings_schema::XrControllerProfile, openxr_controller_profiles::OPENXR_INPUT_PROFILES};

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::popup_manager::{MountPopupOnceParams, PopupHolder},
	views::{self, ViewTrait, ViewUpdateParams, bindings},
};

#[derive(Clone)]
enum Task {
	SelectProfile(&'static XrControllerProfile),
}

pub struct Params<'a> {
	pub globals: WguiGlobals,
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub frontend_tasks: &'a FrontendTasks,
}

pub struct View {
	tasks: Tasks<Task>,
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	bindings_popup: PopupHolder<bindings::View>,
}

impl ViewTrait for View {
	fn update(&mut self, par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		self.bindings_popup.update(par)?;

		for task in self.tasks.drain() {
			match task {
				Task::SelectProfile(profile) => {
					views::bindings::mount_popup(
						self.frontend_tasks.clone(),
						self.globals.clone(),
						self.bindings_popup.clone(),
						profile,
					);
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

		for (idx, profile) in OPENXR_INPUT_PROFILES.iter().enumerate() {
			let id = format!("profile_btn_{idx}");

			let mut cell_params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
			cell_params.insert(Rc::from("id"), Rc::from(id.clone()));
			cell_params.insert(Rc::from("text"), Rc::from(profile.display_name));

			parser_state.instantiate_template(
				doc_params,
				"InputProfileButton",
				params.layout,
				list_parent,
				cell_params,
			)?;

			let btn = parser_state.fetch_component_as::<ComponentButton>(&id)?;
			let tasks_clone = tasks.clone();
			btn.on_click(Rc::new({
				move |_common, _e| {
					tasks_clone.push(Task::SelectProfile(profile));
					Ok(())
				}
			}));
		}

		Ok(Self {
			tasks,
			frontend_tasks: params.frontend_tasks.clone(),
			globals: params.globals.clone(),
			bindings_popup: Default::default(),
		})
	}
}

pub fn mount_popup(frontend_tasks: FrontendTasks, globals: WguiGlobals, popup: PopupHolder<View>) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_translation_key("APP_SETTINGS.INPUT_PROFILES"),
			Box::new(move |data| {
				let view = View::new(Params {
					globals: globals.clone(),
					layout: data.layout,
					parent_id: data.id_content,
					frontend_tasks: &frontend_tasks,
				})?;

				popup.set_view(data.handle, view, None);
				Ok(popup.get_close_callback(data.layout))
			}),
			Default::default(), /* extra */
		)));
}
