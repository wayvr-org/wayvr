use std::{cell::RefCell, rc::Rc};

use glam::Vec2;
use taffy::prelude::{length, percent};

use crate::{
	animation::{Animation, AnimationEasing},
	assets::AssetPath,
	components::button::ComponentButton,
	drawing,
	event::{EventListenerKind, StyleSetRequest},
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, LayoutTask, LayoutTasks, WidgetPair},
	parser::{self, Fetchable, ParserState},
	widget::{EventResult, div::WidgetDiv, label::WidgetLabel, rectangle::WidgetRectangle},
};

struct OpenedWindow {
	layout_tasks: LayoutTasks,
	widget: WidgetPair,
	input_grabber: Option<WidgetPair>,
	content: WidgetPair,

	#[allow(dead_code)]
	state: Option<ParserState>,
}

impl Drop for OpenedWindow {
	fn drop(&mut self) {
		self.layout_tasks.push(LayoutTask::RemoveWidget(self.widget.id));
		if let Some(grabber) = &self.input_grabber {
			self.layout_tasks.push(LayoutTask::RemoveWidget(grabber.id));
		}
	}
}

struct State {
	opened_window: Option<OpenedWindow>,
}

#[derive(Clone)]
pub struct WguiWindow(Rc<RefCell<State>>);

pub struct OnContentData {
	pub widget: WidgetPair,
}

#[derive(Default)]
pub enum WguiWindowPlacement {
	#[default]
	TopLeft,
	BottomLeft,
	TopRight,
	BottomRight,
}

pub struct WguiWindowParamsExtra {
	pub fixed_width: Option<f32>,
	pub fixed_height: Option<f32>,
	pub placement: WguiWindowPlacement,
	pub with_decorations: bool,
	pub no_decoration_padding: f32,
	pub close_if_clicked_outside: bool,
	pub title: Option<Translation>,
}

impl Default for WguiWindowParamsExtra {
	fn default() -> Self {
		Self {
			fixed_width: None,
			fixed_height: None,
			title: None,
			placement: WguiWindowPlacement::TopLeft,
			no_decoration_padding: 0.0,
			close_if_clicked_outside: false,
			with_decorations: true,
		}
	}
}

pub struct WguiWindowParams<'a> {
	pub position: Vec2,
	pub layout: &'a mut Layout,
	pub extra: WguiWindowParamsExtra,
}

impl Default for WguiWindow {
	fn default() -> Self {
		Self(Rc::new(RefCell::new(State { opened_window: None })))
	}
}

const WINDOW_DECORATION_PADDING: f32 = 2.0;
const WINDOW_DECORATION_HEADER_HEIGHT: f32 = 32.0;

impl WguiWindow {
	pub fn close(&self) {
		self.0.borrow_mut().opened_window = None;
	}

	pub fn open(&self, params: &mut WguiWindowParams) -> anyhow::Result<()> {
		// close previous one if it's already open
		self.close();

		let header_height = if params.extra.with_decorations {
			WINDOW_DECORATION_HEADER_HEIGHT
		} else {
			0.0
		};

		let window_padding = if params.extra.with_decorations {
			WINDOW_DECORATION_PADDING
		} else {
			params.extra.no_decoration_padding
		};

		let (padding, justify_content, align_items) = match params.extra.placement {
			WguiWindowPlacement::TopLeft => (
				taffy::Rect {
					left: length(params.position.x - window_padding),
					top: length(params.position.y - header_height - window_padding),
					bottom: length(0.0),
					right: length(0.0),
				},
				taffy::JustifyContent::Start, // x start
				taffy::AlignItems::Start,     // y start
			),
			WguiWindowPlacement::BottomLeft => (
				taffy::Rect {
					left: length(params.position.x - window_padding),
					top: length(0.0),
					bottom: length(params.position.y - window_padding),
					right: length(0.0),
				},
				taffy::JustifyContent::Start, // x start
				taffy::AlignItems::End,       // y end
			),
			WguiWindowPlacement::TopRight => (
				taffy::Rect {
					left: length(0.0),
					top: length(params.position.y - header_height - window_padding),
					bottom: length(0.0),
					right: length(params.position.x - window_padding),
				},
				taffy::JustifyContent::End, // x end
				taffy::AlignItems::Start,   // y start
			),
			WguiWindowPlacement::BottomRight => (
				taffy::Rect {
					left: length(0.0),
					top: length(0.0),
					bottom: length(params.position.y - window_padding),
					right: length(params.position.x - window_padding),
				},
				taffy::JustifyContent::End, // x end
				taffy::AlignItems::End,     // y end
			),
		};

		let input_grabber = if params.extra.close_if_clicked_outside {
			let mut rect = WidgetRectangle::create(Default::default());
			rect.flags.consume_mouse_events = true;

			let this = self.clone();
			rect.event_listeners.register(
				EventListenerKind::MousePress,
				Box::new(move |_, _, (), ()| {
					this.close();
					Ok(EventResult::Consumed)
				}),
			);

			let (widget, _) = params.layout.add_topmost_child(
				rect,
				taffy::Style {
					position: taffy::Position::Absolute,
					size: taffy::Size {
						width: percent(1.0),
						height: percent(1.0),
					},
					..Default::default()
				},
			)?;

			// Fade animation
			params.layout.animations.add(Animation::new(
				widget.id,
				20,
				AnimationEasing::OutQuad,
				Box::new(|common, data| {
					let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap() /* should always succeed */;
					rect.params.color = drawing::Color::new(0.0, 0.0, 0.0, data.pos * 0.3);
					common.alterables.mark_redraw();
				}),
			));

			Some(widget)
		} else {
			None
		};

		let (widget, _) = params.layout.add_topmost_child(
			WidgetDiv::create(),
			taffy::Style {
				position: taffy::Position::Absolute,
				align_items: Some(align_items),
				justify_content: Some(justify_content),
				padding,
				size: taffy::Size {
					width: percent(1.0),
					height: percent(1.0),
				},
				..Default::default()
			},
		)?;

		let globals = params.layout.state.globals.clone();

		let content_id = if params.extra.with_decorations {
			let xml_path: AssetPath = AssetPath::WguiInternal("wgui/window_frame.xml");

			let state = parser::parse_from_assets(
				&parser::ParseDocumentParams {
					globals: globals.clone(),
					path: xml_path,
					extra: Default::default(),
				},
				params.layout,
				widget.id,
			)?;

			let but_close = state.fetch_component_as::<ComponentButton>("but_close").unwrap();
			but_close.on_click({
				let this = self.clone();
				Rc::new(move |_common, _e| {
					this.close();
					Ok(())
				})
			});

			if let Some(title) = &params.extra.title {
				let mut text_title = state.fetch_widget_as::<WidgetLabel>(&params.layout.state, "text_window_title")?;
				text_title.set_text_simple(&mut globals.get(), title.clone());
			}
			let content = state.fetch_widget(&params.layout.state, "content")?;

			self.0.borrow_mut().opened_window = Some(OpenedWindow {
				widget,
				state: Some(state),
				layout_tasks: params.layout.tasks.clone(),
				content: content.clone(),
				input_grabber,
			});

			content.id
		} else {
			// without decorations
			let (content, _) = params
				.layout
				.add_child(widget.id, WidgetDiv::create(), Default::default())?;

			self.0.borrow_mut().opened_window = Some(OpenedWindow {
				widget,
				state: None,
				layout_tasks: params.layout.tasks.clone(),
				content: content.clone(),
				input_grabber,
			});

			content.id
		};

		if let Some(width) = params.extra.fixed_width {
			params
				.layout
				.alterables
				.set_style(content_id, StyleSetRequest::Width(length(width)));
		}

		if let Some(height) = params.extra.fixed_height {
			params
				.layout
				.alterables
				.set_style(content_id, StyleSetRequest::Height(length(height)));
		}

		Ok(())
	}

	pub fn get_content(&self) -> WidgetPair {
		let state = self.0.borrow_mut();
		state.opened_window.as_ref().unwrap().content.clone()
	}
}
