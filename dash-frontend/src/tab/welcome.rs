use std::{marker::PhantomData, rc::Rc};

use wgui::{
	assets::AssetPathRef,
	components::button::ComponentButton,
	globals::WguiGlobals,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState, TemplateParams},
	task::Tasks,
};

use crate::{
	frontend::{Frontend, FrontendTask, FrontendTasks},
	tab::{Tab, TabType},
};

#[derive(Clone)]
#[allow(clippy::enum_variant_names)]
enum Task {
	SetPage(u8),
	SetPageNext,
	SetPagePrev,
}

pub struct TabWelcome<T> {
	#[allow(dead_code)]
	pub state: ParserState,
	marker: PhantomData<T>,
	tasks: Tasks<Task>,
	current_page: u8,
	id_pips: WidgetID,
	id_content: WidgetID,
	frontend_tasks: FrontendTasks,

	state_tab: Option<ParserState>,
}

const PAGE_COUNT: u8 = 6; // 0-5 inclusive

impl<T> Tab<T> for TabWelcome<T> {
	fn get_type(&self) -> TabType {
		TabType::Welcome
	}

	fn update(&mut self, frontend: &mut Frontend<T>, _time_ms: u32, _user_data: &mut T) -> anyhow::Result<()> {
		for task in self.tasks.drain() {
			match task {
				Task::SetPage(page_num) => {
					self.set_page(&mut frontend.layout, page_num)?;
				}
				Task::SetPageNext => {
					if self.current_page < PAGE_COUNT - 1 {
						self.tasks.push(Task::SetPage(self.current_page + 1));
					}
				}
				Task::SetPagePrev => {
					if self.current_page > 0 {
						self.tasks.push(Task::SetPage(self.current_page - 1));
					}
				}
			}
		}

		Ok(())
	}
}

fn doc_params(globals: &WguiGlobals) -> ParseDocumentParams<'_> {
	ParseDocumentParams {
		globals: globals.clone(),
		path: AssetPathRef::BuiltIn("gui/tab/welcome.xml"),
		extra: Default::default(),
	}
}

impl<T> TabWelcome<T> {
	pub fn new(frontend: &mut Frontend<T>, parent_id: WidgetID, _data: &mut T) -> anyhow::Result<Self> {
		let state = wgui::parser::parse_from_assets(&doc_params(&frontend.globals), &mut frontend.layout, parent_id)?;

		let tasks = Tasks::<Task>::new();

		let btn_prev = state.fetch_component_as::<ComponentButton>("btn_prev")?;
		let btn_next = state.fetch_component_as::<ComponentButton>("btn_next")?;

		tasks.handle_button(&btn_prev, Task::SetPagePrev);
		tasks.handle_button(&btn_next, Task::SetPageNext);

		let id_pips = state.get_widget_id("pips")?;
		let id_content = state.get_widget_id("content")?;

		tasks.push(Task::SetPage(0));

		Ok(Self {
			state,
			marker: PhantomData,
			current_page: 0,
			id_pips,
			id_content,
			tasks,
			state_tab: None,
			frontend_tasks: frontend.tasks.clone(),
		})
	}

	fn refresh_pips(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		layout.remove_children(self.id_pips);

		let globals = layout.state.globals.clone();

		for i in 0..PAGE_COUNT {
			let mut params = TemplateParams::new();
			let is_selected = i == self.current_page;
			params.insert(
				"COLOR",
				if is_selected {
					"primary"
				} else {
					"on_background(opacity-0.25)"
				},
			);

			self
				.state
				.instantiate_template(&doc_params(&globals), "Pip", layout, self.id_pips, params)?
		}

		Ok(())
	}

	fn fill_page(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		layout.remove_children(self.id_content);

		let globals = layout.state.globals.clone();

		let state = wgui::parser::parse_from_assets(
			&ParseDocumentParams {
				globals,
				path: AssetPathRef::BuiltIn(&format!("gui/tab/welcome_page_{}.xml", self.current_page)),
				extra: Default::default(),
			},
			layout,
			self.id_content,
		)?;

		if let Ok(btn) = state.fetch_component_as::<ComponentButton>("btn_home_screen") {
			btn.on_click({
				let tasks = self.frontend_tasks.clone();
				Rc::new(move |_, _| {
					tasks.push(FrontendTask::SetTab(TabType::Home));
					tasks.push(FrontendTask::MarkTutorialGraduated);
					Ok(())
				})
			});
		}

		if let Ok(btn) = state.fetch_component_as::<ComponentButton>("btn_wayvr_org") {
			self
				.frontend_tasks
				.handle_button(&btn, FrontendTask::OpenURL(Rc::from("https://wayvr.org")));
		}

		self.state_tab = Some(state);

		Ok(())
	}

	fn set_page(&mut self, layout: &mut Layout, page_num: u8) -> anyhow::Result<()> {
		self.current_page = page_num;

		self.refresh_pips(layout)?;
		self.fill_page(layout)?;

		Ok(())
	}
}
