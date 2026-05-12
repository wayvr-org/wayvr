use std::{cell::RefCell, path::PathBuf, rc::Rc};

use crate::{
	assets,
	testbed::{Testbed, TestbedUpdateParams},
};
use glam::Vec2;
use wgui::{
	assets::AssetPath,
	components::{
		Component,
		button::{ButtonClickCallback, ComponentButton},
		checkbox::ComponentCheckbox,
	},
	drawing::Color,
	event::StyleSetRequest,
	font_config::WguiFontConfig,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, LayoutParams, LayoutUpdateParams, Widget},
	parser::{Fetchable, ParseDocumentExtra, ParseDocumentParams, ParserState},
	taffy::{self, prelude::length},
	task::Tasks,
	theme::WguiTheme,
	widget::{div::WidgetDiv, label::WidgetLabel, rectangle::WidgetRectangle},
	windowing::{
		context_menu::{self, TickResult},
		window::{WguiWindow, WguiWindowParams, WguiWindowParamsExtra},
	},
};
use wlx_common::locale::WayVRLangProvider;

#[derive(Clone)]
pub enum TestbedTask {
	ShowPopup,
	ShowContextMenu(Vec2),
}

struct Data {
	popup_window: WguiWindow,
	context_menu: context_menu::ContextMenu,
}

pub struct TestbedGeneric {
	pub layout: Layout,
	pub parser_state: ParserState,
	tasks: Tasks<TestbedTask>,

	data: Rc<RefCell<Data>>,
}

fn button_click_callback(
	button: Component,
	label: Widget,
	text: &'static str,
) -> ButtonClickCallback {
	Rc::new(move |common, _e| {
		label
			.cast::<WidgetLabel>()?
			.set_text(common, Translation::from_raw_text(text));

		button.try_cast::<ComponentButton>()?.set_text(
			common,
			Translation::from_raw_text("this button has been clicked"),
		);

		Ok(())
	})
}

fn handle_button_click(button: Rc<ComponentButton>, label: Widget, text: &'static str) {
	button.on_click(button_click_callback(
		Component(button.clone()),
		label,
		text,
	));
}

impl TestbedGeneric {
	fn doc_params(globals: &'_ WguiGlobals, extra: ParseDocumentExtra) -> ParseDocumentParams<'_> {
		ParseDocumentParams {
			globals: globals.clone(),
			path: AssetPath::BuiltIn("gui/various_widgets.xml"),
			extra,
		}
	}

	pub fn new(assets: Box<assets::Asset>) -> anyhow::Result<Self> {
		let lang_provider = WayVRLangProvider::default();

		let globals = WguiGlobals::new(
			assets,
			&lang_provider,
			&WguiFontConfig::default(),
			PathBuf::new(), // cwd
		)?;

		let extra = ParseDocumentExtra {
			on_custom_attribs: Some(Rc::new(move |par| {
				let Some(my_custom_value) = par.get_value("_my_custom") else {
					return;
				};

				let Some(mult_value) = par.get_value("_mult") else {
					return;
				};

				let mult_f32 = mult_value.parse::<f32>().unwrap();

				let mut color = match my_custom_value.as_ref() {
					"red" => Color::new(1.0, 0.0, 0.0, 1.0),
					"green" => Color::new(0.0, 1.0, 0.0, 1.0),
					"blue" => Color::new(0.0, 0.0, 1.0, 1.0),
					_ => Color::new(1.0, 1.0, 1.0, 1.0),
				};

				color = color.mult_rgb(mult_f32);

				let mut rect = par.get_widget_as::<WidgetRectangle>().unwrap();
				rect.params.color = color;
			})),
			dev_mode: false,
		};

		let (layout, parser_state) = wgui::parser::new_layout_from_assets(
			&TestbedGeneric::doc_params(&globals, extra),
			LayoutParams {
				resize_to_parent: true,
				theme: Rc::new(WguiTheme::default()),
			},
		)?;

		let cb_visible = parser_state.fetch_component_as::<ComponentCheckbox>("cb_visible")?;
		let div_visibility = parser_state.fetch_widget(&layout.state, "div_visibility")?;

		cb_visible.on_toggle(Box::new(move |common, evt| {
			common.alterables.set_style(
				div_visibility.id,
				StyleSetRequest::Display(if evt.checked {
					taffy::Display::Flex
				} else {
					taffy::Display::None
				}),
			);
			Ok(())
		}));

		let label_cur_option = parser_state.fetch_widget(&layout.state, "label_current_option")?;

		let button_context_menu =
			parser_state.fetch_component_as::<ComponentButton>("button_context_menu")?;
		let button_click_me = parser_state.fetch_component_as::<ComponentButton>("button_click_me")?;
		let button = button_click_me.clone();
		button_click_me.on_click(Rc::new(move |common, _e| {
			button.set_text(common, Translation::from_raw_text("congrats!"));
			Ok(())
		}));

		let button_popup = parser_state.fetch_component_as::<ComponentButton>("button_popup")?;
		let button_red = parser_state.fetch_component_as::<ComponentButton>("button_red")?;
		let button_aqua = parser_state.fetch_component_as::<ComponentButton>("button_aqua")?;
		let button_yellow = parser_state.fetch_component_as::<ComponentButton>("button_yellow")?;

		handle_button_click(button_red, label_cur_option.widget.clone(), "Clicked red");
		handle_button_click(button_aqua, label_cur_option.widget.clone(), "Clicked aqua");
		handle_button_click(
			button_yellow,
			label_cur_option.widget.clone(),
			"Clicked yellow",
		);

		let cb_first = parser_state.fetch_component_as::<ComponentCheckbox>("cb_first")?;
		let label = label_cur_option.widget.clone();
		cb_first.on_toggle(Box::new(move |common, e| {
			let mut widget = label.cast::<WidgetLabel>()?;
			let text = format!("checkbox toggle: {}", e.checked);
			widget.set_text(common, Translation::from_raw_text(&text));
			Ok(())
		}));

		let testbed = Self {
			layout,
			parser_state,
			tasks: Default::default(),
			data: Rc::new(RefCell::new(Data {
				popup_window: WguiWindow::default(),
				context_menu: context_menu::ContextMenu::default(),
			})),
		};

		button_popup.on_click({
			let tasks = testbed.tasks.clone();
			Rc::new(move |_, _| {
				tasks.push(TestbedTask::ShowPopup);
				Ok(())
			})
		});

		button_context_menu.on_click({
			let tasks = testbed.tasks.clone();
			Rc::new(move |_common, m| {
				tasks.push(TestbedTask::ShowContextMenu(m.boundary.bottom_left()));
				Ok(())
			})
		});

		Ok(testbed)
	}

	fn process_task(
		&mut self,
		task: &TestbedTask,
		params: &mut TestbedUpdateParams,
		data: &mut Data,
	) -> anyhow::Result<()> {
		match task {
			TestbedTask::ShowPopup => self.show_popup(params, data)?,
			TestbedTask::ShowContextMenu(position) => self.show_context_menu(params, data, *position),
		}

		Ok(())
	}

	fn show_popup(
		&mut self,
		_params: &mut TestbedUpdateParams,
		data: &mut Data,
	) -> anyhow::Result<()> {
		data.popup_window.open(&mut WguiWindowParams {
			position: Vec2::new(128.0, 128.0),
			layout: &mut self.layout,
			extra: WguiWindowParamsExtra {
				title: Some(Translation::from_raw_text("foo")),
				..Default::default()
			},
		})?;

		self.layout.add_child(
			data.popup_window.get_content().id,
			WidgetDiv::create(),
			taffy::Style {
				size: taffy::Size {
					width: length(128.0),
					height: length(64.0),
				},
				..Default::default()
			},
		)?;

		Ok(())
	}

	fn show_context_menu(
		&mut self,
		_params: &mut TestbedUpdateParams,
		data: &mut Data,
		position: Vec2,
	) {
		data.context_menu.open(context_menu::OpenParams {
			on_custom_attribs: Some(Rc::new(move |custom_attribs| {
				log::info!("custom attribs {:?}", custom_attribs.pairs);
			})),
			blueprint: context_menu::Blueprint::Template {
				template_name: "my_context_menu".into(),
				template_params: Default::default(),
			},
			position,
		});
	}
}

impl Testbed for TestbedGeneric {
	fn update(&mut self, mut params: TestbedUpdateParams) -> anyhow::Result<()> {
		let data = self.data.clone();
		let mut data = data.borrow_mut();

		let res = self.layout.update(&mut LayoutUpdateParams {
			size: Vec2::new(params.width, params.height),
			timestep_alpha: params.timestep_alpha,
		})?;

		params.process_layout_result(res);

		for task in self.tasks.drain() {
			self.process_task(&task, &mut params, &mut data)?;
		}

		let res = data
			.context_menu
			.tick(&mut self.layout, &mut self.parser_state)?;
		if let TickResult::Action(action_name) = res {
			log::info!("got action: {}", action_name);
		}

		Ok(())
	}

	fn layout(&mut self) -> &mut Layout {
		&mut self.layout
	}
}
