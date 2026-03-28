use slotmap::Key;

use crate::{
	drawing::{self},
	layout::WidgetID,
	stack,
	widget::WidgetStateFlags,
};

use super::{WidgetObj, WidgetState};

pub struct CustomDrawArgs<'a> {
	pub boundary: &'a drawing::Boundary,
	pub transform: &'a stack::Transform,
	pub primitives: &'a mut Vec<drawing::RenderPrimitive>,
}

pub struct WidgetCustomDrawParams {
	pub func: Box<dyn Fn(CustomDrawArgs)>,
}

// FIXME: bring up a better name for this
pub struct WidgetCustomDraw {
	pub params: WidgetCustomDrawParams,
	id: WidgetID,
}

impl WidgetCustomDraw {
	pub fn create(params: WidgetCustomDrawParams) -> WidgetState {
		WidgetState::new(
			WidgetStateFlags::default(),
			Box::new(Self {
				params,
				id: WidgetID::null(),
			}),
		)
	}
}

impl WidgetObj for WidgetCustomDraw {
	fn draw(&mut self, state: &mut super::DrawState, _params: &super::DrawParams) {
		let boundary = drawing::Boundary::construct_relative(state.transform_stack);
		(*self.params.func)(CustomDrawArgs {
			primitives: state.primitives,
			boundary: &boundary,
			transform: state.transform_stack.get(),
		});
	}

	fn get_id(&self) -> WidgetID {
		self.id
	}

	fn set_id(&mut self, id: WidgetID) {
		self.id = id;
	}

	fn get_type(&self) -> super::WidgetType {
		super::WidgetType::Rectangle
	}

	fn debug_print(&self) -> String {
		String::default()
	}
}
