use crate::{
	assets::AssetPathRef,
	components::{ComponentTrait, button::ComponentButton},
	drawing::Boundary,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{self, Fetchable, ParseDocumentParams, ParserState, TemplateParams},
	task::Tasks,
	windowing::window::{WguiWindow, WguiWindowParams, WguiWindowParamsExtra, WguiWindowPlacement},
};
use glam::Vec2;
use std::rc::Rc;

#[derive(Default, Clone)]
pub struct Button {
	pub title: Translation,
	pub tooltip: Option<Translation>,
	pub action_name: Option<Rc<str>>,
	pub sprite_src_builtin: Option<Rc<str>>,
	pub attribs: Vec<parser::AttribPair>,
}

#[derive(Clone)]
pub enum Cell {
	Button(Button),
	Separator,
}

pub enum Blueprint {
	Cells(Vec<Cell>),
	Template {
		template_name: Rc<str>,
		template_params: TemplateParams,
	},
}

#[derive(Clone)]
pub enum Position {
	Unspecified,
	Absolute(Vec2),
	Widget(WidgetID),
	Boundary(Boundary),
}

// absolute position
impl From<Vec2> for Position {
	fn from(value: Vec2) -> Self {
		Self::Absolute(value)
	}
}

impl From<Option<Vec2>> for Position {
	fn from(value: Option<Vec2>) -> Self {
		match value {
			Some(v) => Self::Absolute(v),
			None => Self::Unspecified,
		}
	}
}

// relative to widget center
impl From<WidgetID> for Position {
	fn from(value: WidgetID) -> Self {
		Self::Widget(value)
	}
}

// relative to absolute rectangle center
impl From<Boundary> for Position {
	fn from(value: Boundary) -> Self {
		Self::Boundary(value)
	}
}

pub struct OpenParams {
	pub on_custom_attribs: Option<parser::OnCustomAttribsFunc>,
	pub position: Position,
	pub blueprint: Blueprint,
}

#[derive(Clone)]
enum Task {
	ActionClicked(Option<Rc<str>>),
}

#[derive(Default)]
pub struct ContextMenu {
	window: WguiWindow,
	pending_open: Option<OpenParams>,
	tasks: Tasks<Task>,
}

const DOC_PARAMS: ParseDocumentParams = ParseDocumentParams {
	path: AssetPathRef::WguiInternal("wgui/context_menu.xml"),
	extra: None,
};

#[derive(Default)]
pub enum TickResult {
	/// Nothing happened
	#[default]
	None,
	/// The context menu was opened.
	Opened,
	/// User has selected an action.
	Action(Rc<str>),
	/// The context menu was closed without an action.
	Closed,
}

impl ContextMenu {
	pub fn open(&mut self, params: OpenParams) {
		self.pending_open = Some(params);
	}

	pub fn close(&self) {
		self.window.close();
	}

	fn process_open(
		&mut self,
		params: OpenParams,
		layout: &mut Layout,
		parser_state: &mut ParserState,
	) -> anyhow::Result<()> {
		let cells = match params.blueprint {
			Blueprint::Template {
				template_name,
				template_params,
			} => parser_state.context_menu_parse_cells(&template_name, &template_params)?,
			Blueprint::Cells(cells) => cells,
		};

		let absolute_pos = match params.position {
			Position::Unspecified => Vec2::new(0.0, 0.0),
			Position::Absolute(pos) => pos,
			Position::Boundary(boundary) => boundary.center(),
			Position::Widget(widget_id) => {
				let widget = layout.state.widgets.fetch(widget_id)?;
				widget.state().data.cached_absolute_boundary.center()
			}
		};

		let root_size = layout.state.get_widget_size(layout.content_root_widget);

		let (window_pos, placement) = if absolute_pos.y < root_size.y / 2.0 {
			(absolute_pos, WguiWindowPlacement::TopLeft)
		} else {
			// invert y axis (position.y is counted from the bottom)
			(
				Vec2::new(absolute_pos.x, root_size.y - absolute_pos.y),
				WguiWindowPlacement::BottomLeft,
			)
		};

		let globals = layout.state.globals.clone();

		self.window.open(&mut WguiWindowParams {
			layout,
			position: window_pos,
			extra: WguiWindowParamsExtra {
				with_decorations: false,
				close_if_clicked_outside: true,
				placement,
				..Default::default()
			},
		})?;

		let content = self.window.get_content();

		let mut inner_parser = parser::parse_from_assets(&DOC_PARAMS, layout, content.id)?;

		let id_buttons = inner_parser.get_widget_id("buttons")?;

		let mut has_sprites = false;

		for cell in &cells {
			if let Cell::Button(button) = cell
				&& button.sprite_src_builtin.is_some()
			{
				has_sprites = true;
			}
		}

		for cell in &cells {
			match cell {
				Cell::Button(cell) => {
					let mut par = TemplateParams::new();
					par.insert_rc("text", cell.title.generate(&mut globals.i18n()));

					if has_sprites {
						par.insert_rc(
							"sprite_src_builtin",
							cell
								.sprite_src_builtin
								.clone()
								.unwrap_or_else(|| Rc::from("@/dashboard/dot.svg")),
						);
					}
					if let Some(tooltip) = &cell.tooltip {
						par.insert_rc("tooltip_str", tooltip.generate(&mut globals.i18n()));
					}

					let mut data_cell = inner_parser.parse_template_only(
						&DOC_PARAMS,
						if has_sprites { "CellButtonSprite" } else { "CellButton" },
						layout,
						id_buttons,
						par,
					)?;

					let button = data_cell.fetch_component_as::<ComponentButton>("button")?;
					let button_id = button.base().get_id();
					parser_state.data.take_results_from(&mut data_cell);
					self
						.tasks
						.handle_button(&button, Task::ActionClicked(cell.action_name.clone()));

					if let Some(c) = params.on_custom_attribs.as_ref() {
						(*c)(parser::CustomAttribsInfo {
							pairs: &cell.attribs,
							parent_id: id_buttons,
							widget_id: button_id,
							widgets: &layout.state.widgets,
						});
					}
				}
				Cell::Separator => {
					inner_parser.realize_template(&DOC_PARAMS, "Separator", layout, id_buttons, Default::default())?;
				}
			}
		}
		Ok(())
	}

	pub fn tick(&mut self, layout: &mut Layout, parser_state: &mut ParserState) -> anyhow::Result<TickResult> {
		if let Some(p) = self.pending_open.take() {
			self.process_open(p, layout, parser_state)?;
			self.window.reposition_if_outside(layout);
			let _ = self.tasks.drain();
			return Ok(TickResult::Opened);
		}

		let mut result = TickResult::default();

		for task in self.tasks.drain() {
			match task {
				Task::ActionClicked(Some(action_name)) => {
					result = TickResult::Action(action_name);
					self.close();
				}
				Task::ActionClicked(None) => {
					result = TickResult::Closed;
					self.close();
				}
			}
		}

		Ok(result)
	}
}
