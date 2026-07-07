use std::{cell::RefCell, rc::Rc};

use crate::{
	components::{Component, ComponentBase, ComponentTrait, RefreshData, checkbox::ComponentCheckbox},
	event::CallbackDataCommon,
	layout::WidgetPair,
	widget::{ConstructEssentials, div::WidgetDiv},
};

pub struct RadioValueChangeEvent {
	pub value: Option<Rc<str>>,
}

pub type RadioValueChangeCallback = Box<dyn Fn(&mut CallbackDataCommon, RadioValueChangeEvent) -> anyhow::Result<()>>;

#[derive(Default)]
struct State {
	radio_boxes: Vec<Rc<ComponentCheckbox>>,
	selected: Option<Rc<ComponentCheckbox>>,
	on_value_changed: Option<RadioValueChangeCallback>,
}

pub struct ComponentRadioGroup {
	base: ComponentBase,
	state: Rc<RefCell<State>>,
}

impl ComponentRadioGroup {
	pub(super) fn register_child(&self, child: Rc<ComponentCheckbox>, checked: bool) {
		let mut state = self.state.borrow_mut();
		if checked {
			state.selected = Some(child.clone());
			for radio_box in &state.radio_boxes {
				radio_box.set_checked_internal(false);
			}
		}
		state.radio_boxes.push(child);
	}

	// This doesn't `set_checked` on `selected` in order to avoid double borrow.
	pub(super) fn set_selected_internal(
		&self,
		common: &mut CallbackDataCommon,
		selected: &Rc<ComponentCheckbox>,
	) -> anyhow::Result<()> {
		let mut state = self.state.borrow_mut();
		if state.selected.as_ref().is_some_and(|b| Rc::ptr_eq(b, selected)) {
			return Ok(());
		}

		let mut selected_found = false;
		for radio_box in &state.radio_boxes {
			if Rc::ptr_eq(radio_box, selected) {
				selected_found = true;
			} else {
				radio_box.set_checked(common, false);
			}
		}
		if !selected_found {
			anyhow::bail!("RadioGroup set_active called with a non-child ComponentCheckbox!");
		}
		state.selected = Some(selected.clone());

		if let Some(on_value_changed) = state.on_value_changed.as_ref() {
			on_value_changed(
				common,
				RadioValueChangeEvent {
					value: selected.get_value(),
				},
			)?;
		}

		Ok(())
	}

	pub fn set_selected(&self, common: &mut CallbackDataCommon, selected: &Rc<ComponentCheckbox>) -> anyhow::Result<()> {
		self.set_selected_internal(common, selected)?;
		if let Some(selected) = self.state.borrow().selected.as_ref() {
			selected.set_checked(common, true);
		}
		Ok(())
	}

	#[must_use]
	pub fn get_value(&self) -> Option<Rc<str>> {
		self.state.borrow().selected.as_ref().and_then(|b| b.get_value())
	}

	pub fn set_value(&self, common: &mut CallbackDataCommon, value: &str) -> anyhow::Result<()> {
		let mut state = self.state.borrow_mut();
		let mut selected = None;
		for radio_box in &state.radio_boxes {
			if radio_box.get_value().is_some_and(|box_val| &*box_val == value) {
				selected = Some(radio_box.clone());
				radio_box.set_checked(common, true);
			} else {
				radio_box.set_checked(common, false);
			}
		}

		if selected.is_some() {
			state.selected = selected;
			Ok(())
		} else {
			anyhow::bail!("No RadioBox found with value '{value}'")
		}
	}

	pub fn on_value_changed(&self, callback: RadioValueChangeCallback) {
		self.state.borrow_mut().on_value_changed = Some(callback);
	}
}

impl ComponentTrait for ComponentRadioGroup {
	fn base(&self) -> &ComponentBase {
		&self.base
	}

	fn base_mut(&mut self) -> &mut ComponentBase {
		&mut self.base
	}

	fn refresh(&self, _data: &mut RefreshData) {
		// nothing to do
	}
}

pub fn construct(
	ess: &mut ConstructEssentials,
	style: taffy::Style,
) -> anyhow::Result<(WidgetPair, Rc<ComponentRadioGroup>)> {
	let (root, _) = ess.layout.add_child(ess.parent, WidgetDiv::create(), style)?;

	let base = ComponentBase {
		id: root.id,
		..Default::default()
	};

	let state = Rc::new(RefCell::new(State::default()));

	let checkbox = Rc::new(ComponentRadioGroup { base, state });

	ess.layout.defer_component_refresh(Component(checkbox.clone()));
	Ok((root, checkbox))
}
