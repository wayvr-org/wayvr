use std::{collections::HashMap, rc::Rc};

use crate::tab::settings::{self, SettingType, Task, horiz_cell, mount_requires_restart};
use wgui::{
	components::{
		button::{ButtonClickEvent, ComponentButton},
		checkbox::ComponentCheckbox,
		slider::ComponentSlider,
	},
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	task::Tasks,
	widget::label::WidgetLabel,
	windowing::context_menu,
};
use wlx_common::config::GeneralConfig;

pub fn options_category(
	mp: &mut MacroParams,
	parent: WidgetID,
	translation: &str,
	icon: &str,
) -> anyhow::Result<WidgetID> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("translation"), Rc::from(translation));
	params.insert(Rc::from("icon"), Rc::from(icon));
	params.insert(Rc::from("id"), Rc::from(id.as_ref()));

	mp.parser_state
		.instantiate_template(mp.doc_params, "SettingsGroupBox", mp.layout, parent, params)?;

	mp.parser_state.get_widget_id(&id)
}

pub fn options_checkbox(mp: &mut MacroParams, parent: WidgetID, setting: SettingType) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("id"), Rc::from(id.as_ref()));

	match setting.get_translation() {
		Ok(translation) => params.insert(Rc::from("translation"), translation.into()),
		Err(raw_text) => params.insert(Rc::from("text"), raw_text.into()),
	};

	if let Some(tooltip) = setting.get_tooltip() {
		params.insert(Rc::from("tooltip"), Rc::from(tooltip));
	}

	let checked = if *setting.mut_bool(mp.config) { "1" } else { "0" };
	params.insert(Rc::from("checked"), Rc::from(checked));

	let id_cell = horiz_cell(mp.layout, parent)?;

	mp.parser_state
		.instantiate_template(mp.doc_params, "CheckBoxSetting", mp.layout, id_cell, params)?;

	if setting.requires_restart() {
		mount_requires_restart(mp.layout, id_cell)?;
	}

	let checkbox = mp.parser_state.fetch_component_as::<ComponentCheckbox>(&id)?;
	checkbox.on_toggle(Box::new({
		let tasks = mp.tasks.clone();
		move |_common, e| {
			tasks.push(Task::UpdateBool(setting, e.checked));
			Ok(())
		}
	}));

	Ok(())
}

pub fn options_slider_f32(
	mp: &mut MacroParams,
	parent: WidgetID,
	setting: SettingType,
	min: f32,
	max: f32,
	step: f32,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("id"), Rc::from(id.as_ref()));

	match setting.get_translation() {
		Ok(translation) => params.insert(Rc::from("translation"), translation.into()),
		Err(raw_text) => params.insert(Rc::from("text"), raw_text.into()),
	};

	if let Some(tooltip) = setting.get_tooltip() {
		params.insert(Rc::from("tooltip"), Rc::from(tooltip));
	}

	let value = setting.mut_f32(mp.config).to_string();
	params.insert(Rc::from("value"), Rc::from(value));
	params.insert(Rc::from("min"), Rc::from(min.to_string()));
	params.insert(Rc::from("max"), Rc::from(max.to_string()));
	params.insert(Rc::from("step"), Rc::from(step.to_string()));

	let id_cell = horiz_cell(mp.layout, parent)?;

	mp.parser_state
		.instantiate_template(mp.doc_params, "SliderSetting", mp.layout, id_cell, params)?;

	if setting.requires_restart() {
		mount_requires_restart(mp.layout, id_cell)?;
	}

	let slider = mp.parser_state.fetch_component_as::<ComponentSlider>(&id)?;
	slider.on_value_changed(Box::new({
		let tasks = mp.tasks.clone();
		move |_common, e| {
			tasks.push(Task::UpdateFloat(setting, e.value));
			Ok(())
		}
	}));

	Ok(())
}

pub fn options_slider_i32(
	mp: &mut MacroParams,
	parent: WidgetID,
	setting: SettingType,
	min: i32,
	max: i32,
	step: i32,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("id"), Rc::from(id.as_ref()));

	match setting.get_translation() {
		Ok(translation) => params.insert(Rc::from("translation"), translation.into()),
		Err(raw_text) => params.insert(Rc::from("text"), raw_text.into()),
	};

	if let Some(tooltip) = setting.get_tooltip() {
		params.insert(Rc::from("tooltip"), Rc::from(tooltip));
	}

	let id_cell = horiz_cell(mp.layout, parent)?;

	let value = setting.mut_i32(mp.config).to_string();
	params.insert(Rc::from("value"), Rc::from(value));
	params.insert(Rc::from("min"), Rc::from(min.to_string()));
	params.insert(Rc::from("max"), Rc::from(max.to_string()));
	params.insert(Rc::from("step"), Rc::from(step.to_string()));

	mp.parser_state
		.instantiate_template(mp.doc_params, "SliderSetting", mp.layout, id_cell, params)?;

	if setting.requires_restart() {
		mount_requires_restart(mp.layout, id_cell)?;
	}

	let slider = mp.parser_state.fetch_component_as::<ComponentSlider>(&id)?;
	slider.on_value_changed(Box::new({
		let tasks = mp.tasks.clone();
		move |_common, e| {
			tasks.push(Task::UpdateInt(setting, e.value as i32));
			Ok(())
		}
	}));
	Ok(())
}

pub fn options_dropdown<EnumType>(
	mp: &mut MacroParams,
	parent: WidgetID,
	setting: &'static SettingType,
) -> anyhow::Result<()>
where
	EnumType: strum::VariantArray + strum::EnumProperty + std::convert::AsRef<str> + Copy + 'static,
{
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("id"), Rc::from(id.as_ref()));

	match setting.get_translation() {
		Ok(translation) => params.insert(Rc::from("translation"), translation.into()),
		Err(raw_text) => params.insert(Rc::from("text"), raw_text.into()),
	};

	if let Some(tooltip) = setting.get_tooltip() {
		params.insert(Rc::from("tooltip"), Rc::from(tooltip));
	}

	let id_cell = horiz_cell(mp.layout, parent)?;

	mp.parser_state
		.instantiate_template(mp.doc_params, "DropdownButton", mp.layout, id_cell, params)?;

	if setting.requires_restart() {
		mount_requires_restart(mp.layout, id_cell)?;
	}

	let setting_str = setting.as_ref();
	let title = setting.get_enum_title(mp.config);

	{
		let mut label = mp
			.parser_state
			.fetch_widget_as::<WidgetLabel>(&mp.layout.state, &format!("{id}_value"))?;
		label.set_text_simple(&mut mp.layout.state.globals.get(), title);
	}

	let btn = mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
	btn.on_click(Rc::new({
		let tasks = mp.tasks.clone();
		move |_common, e: ButtonClickEvent| {
			tasks.push(Task::OpenContextMenu(
				e.mouse_pos_absolute.unwrap_or_default(),
				EnumType::VARIANTS
					.iter()
					.filter_map(|item| {
						if item.get_bool("Hidden").unwrap_or(false) {
							return None;
						}

						let value = item.as_ref();
						let title = SettingType::get_enum_title_inner(*item);
						let tooltip = SettingType::get_enum_tooltip_inner(*item);

						let text = &title.text;
						let translated = if title.translated { "1" } else { "0" };

						Some(context_menu::Cell {
							action_name: Some(format!("{setting_str};{id};{value};{text};{translated}").into()),
							title,
							tooltip,
							attribs: vec![],
						})
					})
					.collect(),
			));
			Ok(())
		}
	}));

	Ok(())
}

pub fn options_danger_button(
	mp: &mut MacroParams,
	parent: WidgetID,
	translation: &str,
	icon: &str,
	task: Task,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("id"), Rc::from(id.as_ref()));
	params.insert(Rc::from("translation"), Rc::from(translation));
	params.insert(Rc::from("icon"), Rc::from(icon));

	mp.parser_state
		.instantiate_template(mp.doc_params, "DangerButton", mp.layout, parent, params)?;

	let btn = mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
	btn.on_click(Rc::new({
		let tasks = mp.tasks.clone();
		move |_common, _e| {
			tasks.push(task.clone());
			Ok(())
		}
	}));

	Ok(())
}

pub fn options_autostart_app(
	mp: &mut MacroParams,
	parent: WidgetID,
	text: &str,
	ids: &mut Vec<Rc<str>>,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
	params.insert(Rc::from("id"), Rc::from(id.as_ref()));
	params.insert(Rc::from("text"), Rc::from(text));

	mp.parser_state
		.instantiate_template(mp.doc_params, "AutostartApp", mp.layout, parent, params)?;

	let btn = mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
	let id: Rc<str> = Rc::from(id);

	ids.push(id.clone());

	btn.on_click(Rc::new({
		let tasks = mp.tasks.clone();
		move |_common, _e| {
			tasks.push(Task::RemoveAutostartApp(id.clone()));
			Ok(())
		}
	}));
	Ok(())
}

pub struct MacroParams<'a> {
	pub layout: &'a mut Layout,
	pub parser_state: &'a mut ParserState,
	pub doc_params: &'a ParseDocumentParams<'a>,
	pub config: &'a mut GeneralConfig,
	pub tasks: Tasks<settings::Task>,
	pub idx: usize,
}
