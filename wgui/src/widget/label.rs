use std::{cell::RefCell, rc::Rc};

use cosmic_text::{Attrs, AttrsList, BorrowedWithFontSystem, Buffer, Metrics, Shaping, Wrap};
use slotmap::Key;
use taffy::AvailableSpace;

use crate::{
	color::{ParentColor, WguiColor, WguiColorName},
	drawing::{self, PrimitiveExtent},
	event::CallbackDataCommon,
	globals::Globals,
	i18n::Translation,
	layout::{LayoutState, WidgetID},
	palette::WguiColorPalette,
	renderer_vk::text::TextStyle,
	widget::WidgetStateFlags,
};

use super::{WidgetObj, WidgetState};

#[derive(Debug, Clone, Default)]
pub struct WidgetLabelParams {
	pub content: Translation,
	pub style: TextStyle,
	pub parent_color: ParentColor,
}

const DEFAULT_COLOR: WguiColor = WguiColorName::OnBackground.to_wgui_color();

pub struct WidgetLabel {
	id: WidgetID,

	params: WidgetLabelParams,
	buffer: Rc<RefCell<Buffer>>,
}

impl WidgetLabel {
	pub fn create(state: &mut LayoutState, params: WidgetLabelParams) -> WidgetState {
		WidgetLabel::create_ex(&mut state.globals.get(), params)
	}

	pub fn create_ex(globals: &mut Globals, mut params: WidgetLabelParams) -> WidgetState {
		if params.style.color.is_none() {
			params.style.color = Some(DEFAULT_COLOR);
		}

		let metrics = Metrics::from(&params.style);
		let attrs = params.style.to_attrs(&globals.palette);
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
				id: WidgetID::null(),
			}),
		)
	}

	fn with_buffer<F, R>(buffer: &Rc<RefCell<Buffer>>, globals: &Globals, func: F) -> R
	where
		F: FnOnce(BorrowedWithFontSystem<'_, Buffer>) -> R,
	{
		let mut font_system = globals.font_system.system.lock();
		let mut buffer = buffer.borrow_mut();
		let buffer = buffer.borrow_with(&mut font_system);
		func(buffer)
	}

	// set text without layout/re-render update.
	// Not recommended unless the widget wasn't rendered yet (first init).
	pub fn set_text_simple(&mut self, globals: &mut Globals, translation: Translation) -> bool {
		if self.params.content == translation {
			return false;
		}

		self.params.content = translation;
		let attrs = self.params.style.to_attrs(&globals.palette);

		let text = self.params.content.generate(&mut globals.i18n_builtin);

		WidgetLabel::with_buffer(&self.buffer, globals, |mut buffer| {
			buffer.set_rich_text(
				[(text.as_ref(), attrs)],
				&Attrs::new(),
				Shaping::Advanced,
				self.params.style.align.map(Into::into),
			);
		});
		true
	}

	fn update_attrs(&mut self, palette: &WguiColorPalette) {
		let attrs = self.params.style.to_attrs(palette);
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

	pub fn set_color(&mut self, common: &mut CallbackDataCommon, color: WguiColor, apply_to_existing_text: bool) {
		self.params.style.color = Some(color);
		if apply_to_existing_text {
			self.update_attrs(&common.globals().palette);
			common.mark_widget_dirty(self.id);
		}
	}

	pub const fn get_color(&self) -> WguiColor {
		self.params.style.color.unwrap() // create_ex populates this
	}

	pub const fn parent_color(&self) -> ParentColor {
		self.params.parent_color
	}
}

impl WidgetObj for WidgetLabel {
	fn draw(&mut self, state: &mut super::DrawState, _params: &super::DrawParams) {
		let boundary = drawing::Boundary::construct_relative(state.transform_stack);

		WidgetLabel::with_buffer(&self.buffer, state.globals, |mut buffer| {
			buffer.set_size(Some(boundary.size.x), Some(boundary.size.y));
		});

		state.primitives.push(drawing::RenderPrimitive::Text(
			PrimitiveExtent {
				boundary,
				transform: state.transform_stack.get().transform,
			},
			self.buffer.clone(),
			self
				.params
				.style
				.shadow
				.clone()
				.map(|s| s.to_text_shadow(&state.globals.palette)),
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

		WidgetLabel::with_buffer(&self.buffer, globals, |mut buffer| {
			buffer.set_size(width_constraint, None);
			// Determine measured size of text
			let (width, total_lines) = buffer.layout_runs().fold((0.0, 0usize), |(width, total_lines), run| {
				(run.line_w.max(width), total_lines + 1)
			});
			let height = total_lines as f32 * buffer.metrics().line_height;
			taffy::Size {
				width: width + 1.0, /* f32 aliasing fix */
				height,
			}
		})
	}

	fn palette_updated(&mut self, common: &mut CallbackDataCommon) {
		let cur_color = self.get_color();
		self.set_color(common, cur_color, true);
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

	fn debug_print(&self, globals: &Globals) -> String {
		let color = if let Some(color) = self.params.style.color {
			format!("[color: {}]", color.resolve(&globals.palette).debug_ansi_block())
		} else {
			String::default()
		};

		format!("[text: \"{}\"]{}", self.params.content.text, color)
	}
}
