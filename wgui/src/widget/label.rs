use std::{cell::RefCell, rc::Rc};

use cosmic_text::{Attrs, AttrsList, Buffer, Metrics, Shaping, Wrap};
use slotmap::Key;
use taffy::AvailableSpace;

use crate::{
	drawing::{self, Boundary, PrimitiveExtent},
	event::CallbackDataCommon,
	globals::Globals,
	i18n::Translation,
	layout::{LayoutState, WidgetID},
	renderer_vk::text::TextStyle,
	theme::WguiTheme,
	widget::WidgetStateFlags,
};

use super::{WidgetObj, WidgetState};

#[derive(Debug, Default)]
pub struct WidgetLabelParams {
	pub content: Translation,
	pub style: TextStyle,
}

pub struct WidgetLabel {
	id: WidgetID,

	params: WidgetLabelParams,
	buffer: Rc<RefCell<Buffer>>,
	last_boundary: Boundary,
}

impl WidgetLabel {
	pub fn create(state: &mut LayoutState, params: WidgetLabelParams) -> WidgetState {
		WidgetLabel::create_ex(&mut state.globals.get(), &state.theme, params)
	}

	pub fn create_ex(globals: &mut Globals, theme: &WguiTheme, mut params: WidgetLabelParams) -> WidgetState {
		if params.style.color.is_none() {
			params.style.color = Some(theme.text_color);
		}

		let metrics = Metrics::from(&params.style);
		let attrs = Attrs::from(&params.style);
		let wrap = Wrap::from(&params.style);

		let mut buffer = Buffer::new_empty(metrics);
		{
			let mut font_system = globals.font_system.system.lock();
			let mut buffer = buffer.borrow_with(&mut font_system);
			buffer.set_wrap(wrap);

			buffer.set_rich_text(
				[(params.content.generate(&mut globals.i18n_builtin).as_ref(), attrs)],
				&Attrs::new(),
				Shaping::Advanced,
				params.style.align.map(Into::into),
			);
		}

		WidgetState::new(
			WidgetStateFlags::default(),
			Box::new(Self {
				params,
				buffer: Rc::new(RefCell::new(buffer)),
				last_boundary: Boundary::default(),
				id: WidgetID::null(),
			}),
		)
	}

	// set text without layout/re-render update.
	// Not recommended unless the widget wasn't rendered yet (first init).
	pub fn set_text_simple(&mut self, globals: &mut Globals, translation: Translation) -> bool {
		if self.params.content == translation {
			return false;
		}

		self.params.content = translation;
		let attrs = Attrs::from(&self.params.style);
		let mut font_system = globals.font_system.system.lock();

		let mut buffer = self.buffer.borrow_mut();
		buffer.set_rich_text(
			&mut font_system,
			[(self.params.content.generate(&mut globals.i18n_builtin).as_ref(), attrs)],
			&Attrs::new(),
			Shaping::Advanced,
			self.params.style.align.map(Into::into),
		);

		true
	}

	fn update_attrs(&mut self) {
		let attrs = Attrs::from(&self.params.style);
		for line in &mut self.buffer.borrow_mut().lines {
			line.set_attrs_list(AttrsList::new(&attrs));
		}
	}

	// set text and check if it needs to be re-rendered/re-layouted
	pub fn set_text(&mut self, common: &mut CallbackDataCommon, translation: Translation) {
		let mut globals = common.state.globals.get();

		if self.set_text_simple(&mut globals, translation) {
			common.mark_widget_dirty(self.id);
		}
	}

	pub fn set_color(&mut self, common: &mut CallbackDataCommon, color: drawing::Color, apply_to_existing_text: bool) {
		self.params.style.color = Some(color);
		if apply_to_existing_text {
			self.update_attrs();
			common.mark_widget_dirty(self.id);
		}
	}
}

impl WidgetObj for WidgetLabel {
	fn draw(&mut self, state: &mut super::DrawState, _params: &super::DrawParams) {
		let boundary = drawing::Boundary::construct_relative(state.transform_stack);

		if self.last_boundary != boundary {
			self.last_boundary = boundary;
			let mut font_system = state.globals.font_system.system.lock();
			let mut buffer = self.buffer.borrow_mut();
			buffer.set_size(&mut font_system, Some(boundary.size.x), Some(boundary.size.y));
		}

		state.primitives.push(drawing::RenderPrimitive::Text(
			PrimitiveExtent {
				boundary,
				transform: state.transform_stack.get().transform,
			},
			self.buffer.clone(),
			self.params.style.shadow.clone(),
		));
	}

	fn measure(
		&mut self,
		globals: &Globals,
		known_dimensions: taffy::Size<Option<f32>>,
		available_space: taffy::Size<taffy::AvailableSpace>,
	) -> taffy::Size<f32> {
		// Set width constraint
		let width_constraint = known_dimensions.width.or(match available_space.width {
			AvailableSpace::MinContent => Some(0.0),
			AvailableSpace::MaxContent => None,
			AvailableSpace::Definite(width) => Some(width),
		});

		let wgui_font_system = &globals.font_system;
		let mut font_system = wgui_font_system.system.lock();
		let mut buffer = self.buffer.borrow_mut();

		buffer.set_size(&mut font_system, width_constraint, None);

		// Determine measured size of text
		let (width, total_lines) = buffer.layout_runs().fold((0.0, 0usize), |(width, total_lines), run| {
			(run.line_w.max(width), total_lines + 1)
		});
		let height = total_lines as f32 * buffer.metrics().line_height;

		taffy::Size {
			width: width + 1.0, /* f32 aliasing fix */
			height,
		}
	}

	fn get_id(&self) -> WidgetID {
		self.id
	}

	fn set_id(&mut self, id: WidgetID) {
		self.id = id;
	}

	fn get_type(&self) -> super::WidgetType {
		super::WidgetType::Label
	}

	fn debug_print(&self) -> String {
		let color = if let Some(color) = self.params.style.color {
			format!("[color: {}]", color.debug_ansi_block())
		} else {
			String::default()
		};

		format!("[text: \"{}\"]{}", self.params.content.text, color)
	}
}
