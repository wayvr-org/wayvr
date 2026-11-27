use crate::{
	assets::AssetPathRc,
	color::WguiColorName,
	components::{
		Component, ComponentBase, ComponentTrait, RefreshData,
		button::{self, ComponentButton},
	},
	event::CallbackDataCommon,
	i18n::Translation,
	layout::WidgetPair,
	widget::{
		ConstructEssentials,
		div::WidgetDiv,
		rectangle::{WidgetRectangle, WidgetRectangleParams},
		util::WLength,
	},
};
use std::{cell::RefCell, rc::Rc};
use taffy::{
	AlignItems,
	prelude::{length, percent},
};

pub struct Cell {
	pub id: Rc<str>,
	pub text: Translation,
	pub sprite_src: Option<AssetPathRc>,
}

pub struct ListClickEvent {
	pub id: Rc<str>,
}

pub type ListClickCallback = Rc<dyn Fn(&mut CallbackDataCommon, ListClickEvent) -> anyhow::Result<()>>;

#[derive(Default)]
pub struct Params {
	pub style: taffy::Style,
	pub cells: Vec<Cell>,
	pub on_click: Option<ListClickCallback>,
	pub selected_cell: Option<Rc<str>>,
}

struct MountedCell {
	id: Rc<str>,
	button: Rc<ComponentButton>,
}

struct State {
	mounted_cells: Vec<MountedCell>,
	on_click: Option<ListClickCallback>,
	selected_cell: Option<Rc<str>>,
}

pub struct ComponentList {
	base: ComponentBase,
	state: Rc<RefCell<State>>,
}

impl ComponentTrait for ComponentList {
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

impl State {
	fn select_cell(&mut self, common: &mut CallbackDataCommon, id: &Rc<str>) {
		for cell in &self.mounted_cells {
			cell.button.set_sticky_state(common, *cell.id == **id);
		}
		self.selected_cell = Some(id.clone());

		if let Some(on_click) = self.on_click.clone() {
			let evt = ListClickEvent { id: id.clone() };
			common.alterables.dispatch(Box::new(move |common| {
				(*on_click)(common, evt)?;
				Ok(())
			}));
		}
	}
}

impl ComponentList {
	pub fn on_click(&self, callback: ListClickCallback) {
		self.state.borrow_mut().on_click = Some(callback);
	}
}

pub fn construct(ess: &mut ConstructEssentials, params: Params) -> anyhow::Result<(WidgetPair, Rc<ComponentList>)> {
	let mut style = params.style;

	// force-override style
	style.overflow.y = taffy::Overflow::Scroll;
	style.flex_direction = taffy::FlexDirection::Column;
	style.flex_wrap = taffy::FlexWrap::NoWrap;
	style.align_items = Some(AlignItems::STRETCH);
	style.gap = length(0.0_f32);

	if style.size.width.is_auto() {
		style.size.width = percent(1.0_f32);
	}

	if style.size.height.is_auto() {
		style.size.height = percent(1.0_f32);
	}

	let (root, _) = ess.layout.add_child(ess.parent, WidgetDiv::create(), style)?;

	let mut mounted_cells = Vec::<MountedCell>::new();

	let count = params.cells.len();

	// Mount cells
	for (idx, cell) in params.cells.into_iter().enumerate() {
		let sprite_src = cell.sprite_src.as_ref().map(|s| s.as_ref());

		let (_, button) = button::construct(
			&mut ConstructEssentials {
				layout: ess.layout,
				parent: root.id,
			},
			button::Params {
				text: Some(cell.text.clone()),
				sprite_src,
				border: 0.0,
				color: Some(WguiColorName::BackgroundVariant.to_wgui_color().with_alpha(0.0)),
				round: WLength::Units(0.0),
				hover_effect: button::HoverEffect::None,
				style: taffy::Style {
					min_size: taffy::Size {
						width: percent(1.0_f32),
						height: length(32.0_f32),
					},
					justify_content: Some(taffy::JustifyContent::START),
					..Default::default()
				},
				..Default::default()
			},
		)?;

		// horizontal separator after each button
		if idx < count - 1 {
			ess.layout.add_child(
				root.id,
				WidgetRectangle::create(WidgetRectangleParams {
					color: WguiColorName::Background.to_wgui_color(),
					..Default::default()
				}),
				taffy::Style {
					min_size: taffy::Size {
						width: percent(1.0_f32),
						height: length(2.0_f32),
					},
					..Default::default()
				},
			)?;
		}

		mounted_cells.push(MountedCell { id: cell.id, button });
	}

	let state = Rc::new(RefCell::new(State {
		selected_cell: params.selected_cell.clone(),
		mounted_cells,
		on_click: params.on_click,
	}));

	// handle button clicks
	for cell in &state.borrow().mounted_cells {
		cell.button.on_click({
			let cell_id = cell.id.clone();
			let state = state.clone();
			Rc::new(move |common, _| {
				state.borrow_mut().select_cell(common, &cell_id);
				Ok(())
			})
		});
	}

	let base = ComponentBase {
		id: root.id,
		lhandles: Default::default(),
	};

	let tabs = Rc::new(ComponentList { base, state });

	ess.layout.defer_component_refresh(Component(tabs.clone()));
	Ok((root, tabs))
}
