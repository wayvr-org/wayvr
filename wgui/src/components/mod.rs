use std::hash::Hash;
use std::{hash::Hasher, rc::Rc};

use crate::{
	any::AnyTrait,
	event::{CallbackDataCommon, EventListenerID},
	layout::WidgetID,
};

pub mod bar_graph;
pub mod button;
pub mod checkbox;
pub mod editbox;
pub mod radio_group;
pub mod slider;
pub mod tabs;
pub mod tooltip;

pub struct RefreshData<'a> {
	pub common: &'a mut CallbackDataCommon<'a>,
}

pub struct FocusChangeData<'a> {
	pub common: &'a mut CallbackDataCommon<'a>,
	pub focused: bool,
}

// common component data
#[derive(Default)]
pub struct ComponentBase {
	#[allow(dead_code)]
	lhandles: Vec<EventListenerID>,
	id: WidgetID,
}

impl ComponentBase {
	pub const fn get_id(&self) -> WidgetID {
		self.id
	}
}

pub trait ComponentTrait: AnyTrait {
	fn base(&self) -> &ComponentBase;
	fn base_mut(&mut self) -> &mut ComponentBase;
	fn refresh(&self, data: &mut RefreshData);
	fn on_focus_change(&self, _data: &mut FocusChangeData) {}
}

#[derive(Clone)]
pub struct Component(pub Rc<dyn ComponentTrait>);

pub type ComponentWeak = std::rc::Weak<dyn ComponentTrait>;

impl Component {
	pub fn weak(&self) -> ComponentWeak {
		Rc::downgrade(&self.0)
	}

	pub fn try_cast<T: 'static>(&self) -> anyhow::Result<Rc<T>> {
		if !(*self.0).as_any().is::<T>() {
			anyhow::bail!("try_cast: type not matching");
		}

		// safety: we already checked it above, should be safe to directly cast it
		unsafe { Ok(Rc::from_raw(Rc::into_raw(self.0.clone()).cast())) }
	}
}

// these hash/eq impls are required in case we want to do something like HashSet<Component> for convenience reasons.

// hash by address
impl Hash for Component {
	fn hash<H: Hasher>(&self, state: &mut H) {
		std::ptr::hash(&raw const self.0, state);
	}
}

// match by address
impl PartialEq for Component {
	fn eq(&self, other: &Self) -> bool {
		std::ptr::eq(&raw const self.0, &raw const other.0)
	}
}

impl Eq for Component {}
