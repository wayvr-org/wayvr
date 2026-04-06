use std::{cell::RefCell, rc::Rc};

use cosmic_text::{Attrs, Buffer, Color, Shaping, Weight};
use slotmap::Key;

use crate::{
	drawing::{self, PrimitiveExtent},
	event::{CallbackDataCommon, EventAlterables},
	globals::Globals,
	layout::WidgetID,
	renderer_vk::text::{
		DEFAULT_METRICS,
		custom_glyph::{CustomGlyph, CustomGlyphData},
	},
	widget::WidgetStateFlags,
};

use super::{WidgetObj, WidgetState};

#[derive(Debug, Default)]
pub struct WidgetSpriteParams {
	pub glyph_data: Option<CustomGlyphData>,
	pub color: Option<drawing::Color>,
}

#[derive(Debug, Default)]
pub struct WidgetSprite {
	params: WidgetSpriteParams,
	id: WidgetID,
}

impl WidgetSprite {
	pub fn create(params: WidgetSpriteParams) -> WidgetState {
		WidgetState::new(
			WidgetStateFlags::default(),
			Box::new(Self {
				params,
				id: WidgetID::null(),
			}),
		)
	}

	pub fn set_color(&mut self, common: &mut CallbackDataCommon, color: drawing::Color) {
		self.params.color = Some(color);
		common.mark_widget_dirty(self.id);
	}

	pub const fn get_color(&self) -> Option<drawing::Color> {
		self.params.color
	}

	pub fn set_content(&mut self, alterables: &mut EventAlterables, content: Option<CustomGlyphData>) {
		if self.params.glyph_data == content {
			return;
		}

		self.params.glyph_data = content;
		alterables.mark_dirty_and_redraw(self.id);
	}

	pub fn get_content(&self) -> Option<CustomGlyphData> {
		self.params.glyph_data.clone()
	}
}

impl WidgetObj for WidgetSprite {
	fn draw(&mut self, state: &mut super::DrawState, _params: &super::DrawParams) {
		let boundary = drawing::Boundary::construct_relative(state.transform_stack);

		if let Some(glyph_data) = self.params.glyph_data.as_ref() {
			let glyph = CustomGlyph {
				data: glyph_data.clone(),
				left: 0.0,
				top: 0.0,
				width: boundary.size.x,
				height: boundary.size.y,
				color: Some(
					self
						.params
						.color
						.map_or(cosmic_text::Color::rgb(255, 255, 255), Into::into),
				),
				snap_to_physical_pixel: true,
			};

			state.primitives.push(drawing::RenderPrimitive::Sprite(
				PrimitiveExtent {
					boundary,
					transform: state.transform_stack.get().transform,
				},
				Some(glyph),
			));
		} else {
			// Source not set or not available, display error text
			let mut buffer = Buffer::new_empty(DEFAULT_METRICS);

			{
				let mut font_system = state.globals.font_system.system.lock();
				let mut buffer = buffer.borrow_with(&mut font_system);
				let attrs = Attrs::new().color(Color::rgb(255, 0, 255)).weight(Weight::BOLD);

				// set text last in order to avoid expensive re-shaping
				buffer.set_text("Error", &attrs, Shaping::Basic, None);
			}

			state.primitives.push(drawing::RenderPrimitive::Text(
				PrimitiveExtent {
					boundary,
					transform: state.transform_stack.get().transform,
				},
				Rc::new(RefCell::new(buffer)),
				None,
			));
		}
	}

	fn measure(
		&mut self,
		_globals: &Globals,
		_known_dimensions: taffy::Size<Option<f32>>,
		_available_space: taffy::Size<taffy::AvailableSpace>,
	) -> taffy::Size<f32> {
		taffy::Size::ZERO
	}

	fn get_id(&self) -> WidgetID {
		self.id
	}

	fn set_id(&mut self, id: WidgetID) {
		self.id = id;
	}

	fn get_type(&self) -> super::WidgetType {
		super::WidgetType::Sprite
	}

	fn debug_print(&self) -> String {
		String::default()
	}
}
