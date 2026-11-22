use std::sync::Arc;

use wgui::{
	components::button::ComponentButton,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState, TemplateParams},
	task::Tasks,
};
use wlx_common::{dash_interface::ConfigChangeKind, keyboard::KEYBOARD_LAYOUTS};

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::popup_manager::{MountPopupOnceParams, PopupHolder},
	views::{ViewTrait, ViewUpdateParams},
};

const DOC_PARAMS: ParseDocumentParams = ParseDocumentParams::builtin("gui/view/keyboard_layout_editor.xml");

#[derive(Clone)]
enum Task {
	ToggleLayout(Arc<str>),
}

pub struct Params<'a> {
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub keyboard_layouts: &'a [Arc<str>],
}

pub struct View {
	tasks: Tasks<Task>,
	parser_state: ParserState,
	list_active: WidgetID,
	list_available: WidgetID,
}

fn get_display_name(layout_name: &str) -> &str {
	for &(display_name, code, _) in KEYBOARD_LAYOUTS {
		if code == layout_name {
			return display_name;
		}
	}

	layout_name
}

fn get_lang_code(layout_name: &str) -> &str {
	for &(_, layout_code, lang_code) in KEYBOARD_LAYOUTS {
		if layout_name == layout_code {
			return lang_code;
		}
	}

	layout_name
}

impl ViewTrait for View {
	fn update(&mut self, par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		let mut changed = false;

		for task in self.tasks.drain() {
			match task {
				Task::ToggleLayout(layout_name) => {
					let layouts = &mut par.general_config.keyboard_layouts;
					match layouts.iter().position(|existing| existing == &layout_name) {
						Some(idx) => {
							layouts.remove(idx);
						}
						None => {
							layouts.push(layout_name);
						}
					}
					changed = true;
				}
			}
		}

		if !changed {
			return Ok(());
		}

		*par.config_change_kind = Some(ConfigChangeKind::KeyboardLayouts);
		self.refresh_lists(par.layout, &par.general_config.keyboard_layouts)?;

		Ok(())
	}
}

impl View {
	pub fn new(params: Params) -> anyhow::Result<Self> {
		let parser_state = wgui::parser::parse_from_assets(&DOC_PARAMS, params.layout, params.parent_id)?;

		let list_active = parser_state.fetch_widget(&params.layout.state, "list_active")?.id;
		let list_available = parser_state.fetch_widget(&params.layout.state, "list_available")?.id;

		let mut view = Self {
			tasks: Tasks::new(),
			parser_state,
			list_active,
			list_available,
		};

		view.refresh_lists(params.layout, params.keyboard_layouts)?;

		Ok(view)
	}

	fn refresh_lists(&mut self, layout: &mut Layout, active: &[Arc<str>]) -> anyhow::Result<()> {
		layout.remove_children(self.list_active);
		layout.remove_children(self.list_available);

		for name in active {
			self.mount_button(layout, self.list_active, name, Task::ToggleLayout(Arc::clone(name)))?;
		}

		for &(_, layout_code, _) in KEYBOARD_LAYOUTS {
			if active.iter().any(|name| &**name == layout_code) {
				continue;
			}

			self.mount_button(
				layout,
				self.list_available,
				layout_code,
				Task::ToggleLayout(Arc::from(layout_code)),
			)?;
		}

		Ok(())
	}

	fn mount_button(
		&mut self,
		layout: &mut Layout,
		list_id: WidgetID,
		layout_code: &str,
		task: Task,
	) -> anyhow::Result<()> {
		let mut cell_params = TemplateParams::new();
		cell_params.insert("name", get_display_name(layout_code));
		cell_params.insert("layout", layout_code);
		cell_params.insert("flag", get_lang_code(layout_code));

		let data = self
			.parser_state
			.realize_template(&DOC_PARAMS, "KeyboardLayoutButton", layout, list_id, cell_params)?;
		let button = data.fetch_component_as::<ComponentButton>("button")?;
		self.tasks.handle_button(&button, task);

		Ok(())
	}
}

pub fn mount_popup(popup: PopupHolder<View>, frontend_tasks: FrontendTasks) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_translation_key("APP_SETTINGS.KEYBOARD_LAYOUT"),
			Box::new(move |data| {
				let view = View::new(Params {
					layout: data.layout,
					parent_id: data.id_content,
					keyboard_layouts: &data.config.keyboard_layouts,
				})?;

				popup.set_view(data.handle, view, None);
				Ok(popup.get_close_callback(data.layout))
			}),
			Default::default(),
		)));
}
