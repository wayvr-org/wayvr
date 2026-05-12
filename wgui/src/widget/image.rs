use std::sync::atomic::{AtomicUsize, Ordering};

use slotmap::Key;

use crate::{
	drawing::{self, ImagePrimitive, PrimitiveExtent},
	event::EventAlterables,
	globals::Globals,
	layout::WidgetID,
	renderer_vk::text::custom_glyph::CustomGlyphData,
	widget::{WidgetStateFlags, util::WLength},
};

use super::{WidgetObj, WidgetState};

static AUTO_INCREMENT: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Default)]
pub struct WidgetImageParams {
	pub glyph_data: Option<CustomGlyphData>,

	pub border: f32,
	pub border_color: drawing::Color,

	pub round: WLength,
}

#[derive(Debug, Default)]
pub struct WidgetImage {
	params: WidgetImageParams,
	id: WidgetID,
	content_key: usize,
}

impl WidgetImage {
	pub fn create(params: WidgetImageParams) -> WidgetState {
		WidgetState::new(
			WidgetStateFlags::default(),
			Box::new(Self {
				params,
				id: WidgetID::null(),
				content_key: AUTO_INCREMENT.fetch_add(1, Ordering::Relaxed),
			}),
		)
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

impl WidgetObj for WidgetImage {
	fn draw(&mut self, state: &mut super::DrawState, _params: &super::DrawParams) {
		let boundary = drawing::Boundary::construct_relative(state.transform_stack);

		let Some(content) = self.params.glyph_data.clone() else {
			return;
		};

		let round_units = match self.params.round {
			WLength::Units(units) => units as u8,
			WLength::Percent(percent) => (f32::min(boundary.size.x, boundary.size.y) * percent / 2.0) as u8,
		};

		state.primitives.push(drawing::RenderPrimitive::Image(
			PrimitiveExtent {
				boundary,
				transform: state.transform_stack.get().transform,
			},
			ImagePrimitive {
				content,
				content_key: self.content_key,
				border: self.params.border,
				border_color: self.params.border_color,
				round_units,
			},
		));
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
