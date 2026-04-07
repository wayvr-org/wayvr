use crate::components::button::{ButtonClickCallback, ComponentButton};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

pub struct Tasks<TaskType>(Rc<RefCell<VecDeque<TaskType>>>);

impl<T> Clone for Tasks<T> {
	fn clone(&self) -> Self {
		Self(self.0.clone())
	}
}

impl<TaskType: 'static> Tasks<TaskType> {
	pub fn new() -> Self {
		Self(Rc::new(RefCell::new(VecDeque::new())))
	}

	pub fn push(&self, task: TaskType) {
		self.0.borrow_mut().push_back(task);
	}

	pub fn drain(&mut self) -> VecDeque<TaskType> {
		let mut tasks = self.0.borrow_mut();
		std::mem::take(&mut *tasks)
	}
}

impl<TaskType: 'static> Default for Tasks<TaskType> {
	fn default() -> Self {
		Self::new()
	}
}

// copyable tasks only!
impl<TaskType: Clone + 'static> Tasks<TaskType> {
	pub fn get_button_click_callback(&self, task: TaskType) -> ButtonClickCallback {
		let this = self.clone();
		Rc::new(move |_, _| {
			this.push(task.clone());
			Ok(())
		})
	}

	pub fn handle_button(&self, button: &Rc<ComponentButton>, task: TaskType) {
		button.on_click(self.get_button_click_callback(task));
	}

	pub fn make_callback_rc(&self, task: TaskType) -> Rc<dyn Fn()> {
		let this = self.clone();
		Rc::new(move || {
			this.push(task.clone());
		})
	}

	pub fn make_callback_box(&self, task: TaskType) -> Box<dyn Fn()> {
		let this = self.clone();
		Box::new(move || {
			this.push(task.clone());
		})
	}
}
