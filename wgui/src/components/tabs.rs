use crate::{
	assets::AssetPathRc,
	color::WguiColor,
	components::{
		Component, ComponentBase, ComponentTrait, DestroyData, RefreshData,
		button::{self, ComponentButton},
	},
	event::CallbackDataCommon,
	i18n::Translation,
	layout::WidgetPair,
	widget::{ConstructEssentials, div::WidgetDiv, util::WLength},
};
use std::{cell::RefCell, rc::Rc};
use taffy::{
	AlignItems,
	prelude::{length, percent},
};

pub struct Entry {
	pub sprite_src: Option<AssetPathRc>,
	pub text: Translation,
	pub name: Rc<str>,
}

pub struct Params<'a> {
	pub style: taffy::Style,
	pub entries: Vec<Entry>,
	pub selected_entry_name: &'a str, // default: ""
	pub on_select: Option<TabSelectCallback>,
	pub round: WLength,
	pub color: Option<WguiColor>,
	pub border: f32,
	pub border_color: Option<WguiColor>,
	pub hover_color: Option<WguiColor>,
	pub hover_border_color: Option<WguiColor>,
	pub sticky_color: Option<WguiColor>,
	pub sticky_border_color: Option<WguiColor>,
}

struct MountedEntry {
	name: Rc<str>,
	button: Rc<ComponentButton>,
}

pub struct TabSelectEvent {
	pub name: Rc<str>,
}

pub type TabSelectCallback = Rc<dyn Fn(&mut CallbackDataCommon, TabSelectEvent) -> anyhow::Result<()>>;

struct State {
	mounted_entries: Vec<MountedEntry>,
	selected_entry_name: Rc<str>,
	on_select: Option<TabSelectCallback>,
}

pub struct ComponentTabs {
	base: ComponentBase,
	state: Rc<RefCell<State>>,
}

impl ComponentTrait for ComponentTabs {
	fn base(&self) -> &ComponentBase {
		&self.base
	}

	fn base_mut(&mut self) -> &mut ComponentBase {
		&mut self.base
	}

	fn refresh(&self, _data: &mut RefreshData) {
		// nothing to do
	}

	fn destroy(&self, data: &mut DestroyData) {
		for e in self.state.borrow_mut().mounted_entries.drain(..) {
			e.button.destroy(data);
			data.destroy_widgets.push(e.button.base().id);
		}
	}
}

impl State {
	fn select_entry(&mut self, common: &mut CallbackDataCommon, name: &Rc<str>) {
		for entry in &self.mounted_entries {
			entry.button.set_sticky_state(common, *entry.name == **name);
		}
		self.selected_entry_name = name.clone();

		if let Some(on_select) = self.on_select.clone() {
			let evt = TabSelectEvent { name: name.clone() };
			common.alterables.dispatch(Box::new(move |common| {
				(*on_select)(common, evt)?;
				Ok(())
			}));
		}
	}
}

impl ComponentTabs {
	pub fn on_select(&self, callback: TabSelectCallback) {
		self.state.borrow_mut().on_select = Some(callback);
	}

	pub fn get_tab_button(&self, name: &str) -> Option<Rc<ComponentButton>> {
		self
			.state
			.borrow_mut()
			.mounted_entries
			.iter()
			.find(|e| name == &*e.name)
			.map(|e| e.button.clone())
	}
}

pub fn construct(ess: &mut ConstructEssentials, params: Params) -> anyhow::Result<(WidgetPair, Rc<ComponentTabs>)> {
	let mut style = params.style;

	// force-override style
	style.overflow.y = taffy::Overflow::Scroll;
	style.flex_direction = taffy::FlexDirection::Column;
	style.flex_wrap = taffy::FlexWrap::NoWrap;
	style.align_items = Some(AlignItems::CENTER);
	style.gap = length(4.0_f32);

	let (root, _) = ess.layout.add_child(ess.parent, WidgetDiv::create(), style)?;

	let mut mounted_entries = Vec::<MountedEntry>::new();

	// Mount entries
	for (idx, entry) in params.entries.into_iter().enumerate() {
		let sprite_src = entry.sprite_src.as_ref().map(AssetPathRc::as_borrowed);

		let (_, button) = button::construct(
			&mut ConstructEssentials {
				layout: ess.layout,
				parent: root.id,
			},
			button::Params {
				text: Some(entry.text),
				sprite_src,
				style: taffy::Style {
					min_size: taffy::Size {
						width: percent(1.0_f32),
						height: length(32.0_f32),
					},
					justify_content: Some(taffy::JustifyContent::START),
					..Default::default()
				},
				round: params.round,
				color: params.color,
				border: params.border,
				border_color: params.border_color,
				hover_color: params.hover_color,
				hover_border_color: params.hover_border_color,
				sticky_color: params.sticky_color,
				sticky_border_color: params.sticky_border_color,
				..Default::default()
			},
		)?;

		// init colors
		button.set_sticky_state(&mut ess.layout.common(), idx == 0);

		mounted_entries.push(MountedEntry {
			name: entry.name,
			button,
		});
	}

	let state = Rc::new(RefCell::new(State {
		selected_entry_name: Rc::from(params.selected_entry_name),
		mounted_entries,
		on_select: params.on_select,
	}));

	// handle button clicks
	for entry in &state.borrow().mounted_entries {
		entry.button.on_click({
			let entry_name = entry.name.clone();
			let state = state.clone();
			Rc::new(move |common, _| {
				state.borrow_mut().select_entry(common, &entry_name);
				Ok(())
			})
		});
	}

	let base = ComponentBase {
		id: root.id,
		lhandles: Default::default(),
	};

	let tabs = Rc::new(ComponentTabs { base, state });

	ess.layout.defer_component_refresh(Component(tabs.clone()));
	Ok((root, tabs))
}
