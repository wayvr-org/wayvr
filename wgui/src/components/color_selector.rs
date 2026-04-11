use crate::{
	assets::AssetPath,
	components::{
		Component, ComponentBase, ComponentTrait, RefreshData,
		button::{self, ComponentButton},
		slider::{ComponentSlider, SliderValueChangedCallback},
	},
	drawing::{self},
	event::CallbackDataCommon,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID, WidgetPair},
	parser::{self, Fetchable, ParseDocumentParams, ParserState},
	widget::{ConstructEssentials, rectangle::WidgetRectangle, util::WLength},
	windowing::window::{WguiWindow, WguiWindowParams, WguiWindowParamsExtra},
};
use glam::Vec2;
use std::{
	cell::RefCell,
	rc::{Rc, Weak},
};
use taffy::prelude::length;

pub struct Params {
	pub color: drawing::Color,
	pub style: taffy::Style,
}

impl Default for Params {
	fn default() -> Self {
		Self {
			color: drawing::Color::new(1.0, 1.0, 1.0, 1.0),
			style: Default::default(),
		}
	}
}

struct WantsOpen {
	position: Vec2,
}

#[allow(dead_code)]
struct PopupState {
	slider_r: Rc<ComponentSlider>,
	slider_g: Rc<ComponentSlider>,
	slider_b: Rc<ComponentSlider>,
	id_rect_color: WidgetID,
}

struct State {
	color: drawing::Color,
	self_ref: Weak<ComponentColorSelector>,
	wants_open: Option<WantsOpen>,
	on_changed: Option<ColorSelectorChangedCallback>,
	popup_state: Option<PopupState>,
}

struct Data {
	button: Rc<ComponentButton>,
}

pub struct ColorSelectorChangedEvent {
	pub color: drawing::Color,
}

pub type ColorSelectorChangedCallback = Box<dyn Fn(&mut CallbackDataCommon, ColorSelectorChangedEvent)>;

pub struct ComponentColorSelector {
	base: ComponentBase,
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
	window: WguiWindow,
}

impl ComponentTrait for ComponentColorSelector {
	fn base(&self) -> &ComponentBase {
		&self.base
	}

	fn base_mut(&mut self) -> &mut ComponentBase {
		&mut self.base
	}

	fn refresh(&self, data: &mut RefreshData) {
		let mut state = self.state.borrow_mut();

		if let Some(wants_open) = state.wants_open.take() {
			if let Err(e) = self.open(data.layout, &mut state, wants_open.position) {
				log::error!("{:?}", e);
				debug_assert!(false);
			}
		}

		self.data.button.set_text(
			&mut data.layout.common(),
			Translation::from_raw_text_string(format!("{}", state.color.to_hex_rgb())),
		);

		self.data.button.set_color(&mut data.layout.common(), state.color);
	}
}

enum ColorIndex {
	Red,
	Green,
	Blue,
}

fn set_color_internal(state: &mut State, common: &mut CallbackDataCommon, color: drawing::Color) {
	if state.color == color {
		return;
	}

	if let Some(on_changed) = &state.on_changed {
		on_changed(common, ColorSelectorChangedEvent { color })
	}

	state.color = color;
	common.alterables.refresh_component_once(&state.self_ref);
	common.alterables.mark_redraw();
}

impl ComponentColorSelector {
	pub fn on_changed(&self, func: ColorSelectorChangedCallback) {
		self.state.borrow_mut().on_changed = Some(func);
	}

	pub fn set_color(&self, common: &mut CallbackDataCommon, color: drawing::Color) {
		let mut state = self.state.borrow_mut();
		set_color_internal(&mut state, common, color);
	}

	pub fn get_color(&self) -> drawing::Color {
		self.state.borrow().color
	}

	fn open(&self, layout: &mut Layout, state: &mut State, position: Vec2) -> anyhow::Result<()> {
		self.window.open(&mut WguiWindowParams {
			position,
			layout,
			extra: WguiWindowParamsExtra {
				close_if_clicked_outside: true,
				// TODO: translation text in wgui too?
				title: Some(Translation::from_raw_text("Select color")),
				..Default::default()
			},
		})?;

		let id_content = self.window.get_content().id;

		let parser_state = parser::parse_from_assets(
			&mut ParseDocumentParams {
				globals: layout.state.globals.clone(),
				path: AssetPath::WguiInternal("wgui/color_selector.xml"),
				extra: Default::default(),
			},
			layout,
			id_content,
		)?;

		let slider_r = parser_state.fetch_component_as::<ComponentSlider>("slider_r")?;
		let slider_g = parser_state.fetch_component_as::<ComponentSlider>("slider_g")?;
		let slider_b = parser_state.fetch_component_as::<ComponentSlider>("slider_b")?;

		{
			let mut common = layout.common();
			slider_r.set_value(&mut common, state.color.r * 255.0);
			slider_g.set_value(&mut common, state.color.g * 255.0);
			slider_b.set_value(&mut common, state.color.b * 255.0);
		}

		slider_r.on_value_changed(self.gen_slider_callback(ColorIndex::Red));
		slider_g.on_value_changed(self.gen_slider_callback(ColorIndex::Green));
		slider_b.on_value_changed(self.gen_slider_callback(ColorIndex::Blue));

		let id_rect_color = parser_state.get_widget_id("rect_color")?;

		state.popup_state = Some(PopupState {
			slider_r,
			slider_g,
			slider_b,
			id_rect_color,
		});
		Ok(())
	}

	fn gen_slider_callback(&self, idx: ColorIndex) -> SliderValueChangedCallback {
		let state = Rc::downgrade(&self.state);
		Box::new(move |common, evt| {
			let Some(state) = state.upgrade() else {
				return;
			};

			let mut state = state.borrow_mut();
			let Some(popup_state) = &state.popup_state else {
				return;
			};

			let norm = evt.value / 255.0;

			let mut new_color = state.color;
			match idx {
				ColorIndex::Red => new_color.r = norm,
				ColorIndex::Green => new_color.g = norm,
				ColorIndex::Blue => new_color.b = norm,
			}

			if let Some(mut rect) = common
				.state
				.widgets
				.get_as::<WidgetRectangle>(popup_state.id_rect_color)
			{
				rect.set_color(common, new_color);
			}
			set_color_internal(&mut state, common, new_color);
		})
	}
}

const DEFAULT_WIDTH: f32 = 96.0;
const DEFAULT_HEIGHT: f32 = 32.0;

pub fn construct(
	ess: &mut ConstructEssentials,
	params: Params,
) -> anyhow::Result<(WidgetPair, Rc<ComponentColorSelector>)> {
	let mut style = params.style;

	if style.size.width.is_auto() {
		style.size.width = length(DEFAULT_WIDTH);
	}

	if style.size.height.is_auto() {
		style.size.height = length(DEFAULT_HEIGHT);
	}

	style.min_size = style.size;
	style.max_size = style.size;

	let (widget_button, button) = button::construct(
		ess,
		button::Params {
			color: Some(params.color),
			round: WLength::Percent(1.0),
			border: 2.0,
			border_color: Some(drawing::Color::new(0.0, 0.0, 0.0, 1.0)),
			style,
			..Default::default()
		},
	)?;

	let data = Rc::new(Data { button: button.clone() });

	let state = Rc::new(RefCell::new(State {
		color: params.color,
		self_ref: Default::default(),
		wants_open: None,
		popup_state: None,
		on_changed: None,
	}));

	let base = ComponentBase {
		id: widget_button.id,
		lhandles: Default::default(),
	};

	let color_selector = Rc::new(ComponentColorSelector {
		base,
		data,
		state: state.clone(),
		window: WguiWindow::default(),
	});

	let self_ref = Rc::downgrade(&color_selector);
	state.borrow_mut().self_ref = self_ref.clone();

	button.on_click(Rc::new({
		let color_selector = color_selector.clone();
		move |common, evt| {
			let mut state = color_selector.state.borrow_mut();
			state.wants_open = Some(WantsOpen {
				position: evt.mouse_pos_absolute.unwrap_or_default(),
			});
			common.alterables.refresh_component_once(&self_ref);
			Ok(())
		}
	}));

	ess.layout.defer_component_refresh(Component(color_selector.clone()));
	Ok((widget_button, color_selector))
}
