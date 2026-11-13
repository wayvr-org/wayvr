use std::{cell::RefCell, rc::Rc};

use glam::Vec2;
use taffy::{
	LengthPercentage,
	prelude::{length, percent},
};

use crate::{
	animation::{Animation, AnimationDuration, AnimationEasing},
	assets::AssetPathRef,
	components::button::ComponentButton,
	drawing,
	event::{EventListenerKind, StyleSetRequest},
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
	placement: WguiWindowPlacement,

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

#[derive(Clone, Copy, Default)]
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

fn shift_padding(
	placement: WguiWindowPlacement,
	rect: taffy::Rect<LengthPercentage>,
	shift: Vec2,
) -> taffy::Rect<LengthPercentage> {
	let padding_left = rect.left.into_raw().value();
	let padding_right = rect.right.into_raw().value();
	let padding_top = rect.top.into_raw().value();
	let padding_bottom = rect.bottom.into_raw().value();

	let (inv_x, inv_y) = match placement {
		WguiWindowPlacement::TopLeft => (false, false),
		WguiWindowPlacement::BottomLeft => (false, true),
		WguiWindowPlacement::TopRight => (true, false),
		WguiWindowPlacement::BottomRight => (true, true),
	};

	taffy::Rect {
		left: length(padding_left + if inv_x { 0.0 } else { shift.x }),
		right: length(padding_right + if inv_x { shift.x } else { 0.0 }),
		top: length(padding_top + if inv_y { 0.0 } else { shift.y }),
		bottom: length(padding_bottom + if inv_y { shift.y } else { 0.0 }),
	}
}

impl WguiWindow {
	pub fn close(&self) {
		self.0.borrow_mut().opened_window = None;
	}

	// before                     after
	// ┌───────────────────────┐  ┌───────────────────────┐
	// │                  root │  │                  root │
	// │                       │  │  ┌───────┐            │
	// │                       │->│  │   win │            │
	// │  ┌───────┐            │  │  │       │            │
	// │  │   win │            │  │  │       │            │
	// │  │       │            │  │  │       │            │
	// └──│───────│────────────┘  └──└───────┘────────────┘
	//    │       │
	//    └───────┘
	pub fn reposition_if_outside(&self, layout: &mut Layout) {
		let mut state = self.0.borrow_mut();
		let Some(window) = &mut state.opened_window else {
			return;
		};

		let _ = layout.recompute_layout(None); // required

		let Some(style) = layout.state.get_widget_style(window.widget.id) else {
			unreachable!(); // shouldn't happen
		};

		let content_size = layout.content_size;
		let boundary = layout.state.get_widget_boundary(window.content.id);

		let mut shift_x = 0.0;
		let mut shift_y = 0.0;

		if boundary.left() < 0.0 {
			shift_x += boundary.left();
		}

		if boundary.top() < 0.0 {
			shift_y += boundary.top();
		}

		if boundary.right() >= content_size.x {
			shift_x -= boundary.right() - content_size.x;
		}

		if boundary.bottom() >= content_size.y {
			shift_y -= boundary.bottom() - content_size.y;
		}

		layout.tasks.push(LayoutTask::SetWidgetStyle(
			window.widget.id,
			StyleSetRequest::Padding(shift_padding(
				window.placement,
				style.padding,
				Vec2::new(shift_x, shift_y),
			)),
		));
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
					bottom: length(0.0_f32),
					right: length(0.0_f32),
				},
				taffy::JustifyContent::START, // x start
				taffy::AlignItems::START,     // y start
			),
			WguiWindowPlacement::BottomLeft => (
				taffy::Rect {
					left: length(params.position.x - window_padding),
					top: length(0.0_f32),
					bottom: length(params.position.y - window_padding),
					right: length(0.0_f32),
				},
				taffy::JustifyContent::START, // x start
				taffy::AlignItems::END,       // y end
			),
			WguiWindowPlacement::TopRight => (
				taffy::Rect {
					left: length(0.0_f32),
					top: length(params.position.y - header_height - window_padding),
					bottom: length(0.0_f32),
					right: length(params.position.x - window_padding),
				},
				taffy::JustifyContent::END, // x end
				taffy::AlignItems::START,   // y start
			),
			WguiWindowPlacement::BottomRight => (
				taffy::Rect {
					left: length(0.0_f32),
					top: length(0.0_f32),
					bottom: length(params.position.y - window_padding),
					right: length(params.position.x - window_padding),
				},
				taffy::JustifyContent::END, // x end
				taffy::AlignItems::END,     // y end
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
						width: percent(1.0_f32),
						height: percent(1.0_f32),
					},
					..Default::default()
				},
			)?;

			// Fade animation
			Animation::new(
				widget.id,
				AnimationDuration::Seconds(0.333),
				AnimationEasing::OutQuad,
				Box::new(|common, data| {
					let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap() /* should always succeed */;
					rect.params.color = drawing::Color::new(0.0, 0.0, 0.0, data.pos * 0.3).into();
					common.alterables.mark_redraw();
				}),
			)
			.submit_l(params.layout);

			Some(widget)
		} else {
			None
		};

		let (topmost_widget, _) = params.layout.add_topmost_child(
			WidgetDiv::create(),
			taffy::Style {
				position: taffy::Position::Absolute,
				align_items: Some(align_items),
				justify_content: Some(justify_content),
				padding,
				size: taffy::Size {
					width: percent(1.0_f32),
					height: percent(1.0_f32),
				},
				..Default::default()
			},
		)?;

		let globals = params.layout.state.globals.clone();

		// window slide animation
		Animation::effect_slide(
			topmost_widget.id,
			AnimationDuration::Seconds(0.5),
			AnimationEasing::OutQuint,
			Vec2::new(0.0, 16.0),
		)
		.submit_l(params.layout);

		let content_id = if params.extra.with_decorations {
			let xml_path: AssetPathRef = AssetPathRef::WguiInternal("wgui/window_frame.xml");

			let state = parser::parse_from_assets(
				&parser::ParseDocumentParams {
					path: xml_path,
					extra: None,
				},
				params.layout,
				topmost_widget.id,
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
				widget: topmost_widget,
				state: Some(state),
				layout_tasks: params.layout.tasks.clone(),
				content: content.clone(),
				input_grabber,
				placement: params.extra.placement,
			});

			content.id
		} else {
			// without decorations
			let (content, _) = params
				.layout
				.add_child(topmost_widget.id, WidgetDiv::create(), Default::default())?;

			self.0.borrow_mut().opened_window = Some(OpenedWindow {
				widget: topmost_widget,
				state: None,
				layout_tasks: params.layout.tasks.clone(),
				content: content.clone(),
				input_grabber,
				placement: params.extra.placement,
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
