use std::rc::Rc;

use crate::tab::settings::{self, SettingType, Task, horiz_cell, mount_requires_restart};
use wgui::{
	components::{
		button::{ButtonClickEvent, ComponentButton},
		checkbox::ComponentCheckbox,
		slider::ComponentSlider,
	},
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState, TemplateParams},
	task::Tasks,
	widget::label::WidgetLabel,
	windowing::context_menu,
};
use wlx_common::{DesktopBackend, XrBackend, config::GeneralConfig, dash_interface::InterfaceFeats};

pub fn options_category(
	mp: &mut MacroParams,
	parent: WidgetID,
	translation: &str,
	icon: &str,
) -> anyhow::Result<WidgetID> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params = TemplateParams::new();
	params.insert("translation", translation);
	params.insert("icon", icon);
	params.insert("id", &id);

	mp.parser_state
		.instantiate_template(mp.doc_params, "SettingsGroupBox", mp.layout, parent, params)?;

	mp.parser_state.get_widget_id(&id)
}

pub fn options_checkbox(mp: &mut MacroParams, parent: WidgetID, setting: SettingType) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params = TemplateParams::new();
	params.insert("id", &id);

	match setting.get_translation() {
		Ok(translation) => params.insert("translation", translation),
		Err(raw_text) => params.insert("text", raw_text),
	};

	if let Some(tooltip) = setting.get_tooltip() {
		params.insert("tooltip", tooltip);
	}

	let checked = if *setting.mut_bool(mp.config) { "1" } else { "0" };
	params.insert("checked", checked);

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

	let mut params = TemplateParams::new();
	params.insert("id", &id);

	match setting.get_translation() {
		Ok(translation) => params.insert("translation", translation),
		Err(raw_text) => params.insert("text", raw_text),
	};

	if let Some(tooltip) = setting.get_tooltip() {
		params.insert("tooltip", tooltip);
	}

	let value = setting.mut_f32(mp.config).to_string();
	params.insert_rc("value", value.into());
	params.insert_rc("min", min.to_string().into());
	params.insert_rc("max", max.to_string().into());
	params.insert_rc("step", step.to_string().into());

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
		}
	}));

	Ok(())
}

pub fn options_range_f32(
	mp: &mut MacroParams,
	parent: WidgetID,
	setting: SettingType,
	setting2: SettingType,
	min: f32,
	max: f32,
	step: f32,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params = TemplateParams::new();
	params.insert("id", &id);

	match setting.get_translation() {
		Ok(translation) => params.insert("translation", translation),
		Err(raw_text) => params.insert("text", raw_text),
	};

	if let Some(tooltip) = setting.get_tooltip() {
		params.insert("tooltip", tooltip);
	}

	let value = setting.mut_f32(mp.config).to_string();
	let value2 = setting2.mut_f32(mp.config).to_string();
	params.insert_rc("value", value.into());
	params.insert_rc("value2", value2.into());
	params.insert_rc("min", min.to_string().into());
	params.insert_rc("max", max.to_string().into());
	params.insert_rc("step", step.to_string().into());

	let id_cell = horiz_cell(mp.layout, parent)?;

	mp.parser_state
		.instantiate_template(mp.doc_params, "RangeSetting", mp.layout, id_cell, params)?;

	if setting.requires_restart() {
		mount_requires_restart(mp.layout, id_cell)?;
	}

	let slider = mp.parser_state.fetch_component_as::<ComponentSlider>(&id)?;
	slider.on_value_changed(Box::new({
		let tasks = mp.tasks.clone();
		move |_common, e| {
			if matches!(e.index, wgui::components::slider::ValueIndex::Primary) {
				tasks.push(Task::UpdateFloat(setting, e.value));
			} else {
				tasks.push(Task::UpdateFloat(setting2, e.value));
			}
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

	let mut params = TemplateParams::new();
	params.insert("id", &id);

	match setting.get_translation() {
		Ok(translation) => params.insert("translation", translation),
		Err(raw_text) => params.insert("text", raw_text),
	};

	if let Some(tooltip) = setting.get_tooltip() {
		params.insert("tooltip", tooltip);
	}

	let id_cell = horiz_cell(mp.layout, parent)?;

	let value = setting.mut_i32(mp.config).to_string();
	params.insert_rc("value", value.into());
	params.insert_rc("min", min.to_string().into());
	params.insert_rc("max", max.to_string().into());
	params.insert_rc("step", step.to_string().into());

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

	let mut params = TemplateParams::new();
	params.insert("id", &id);

	match setting.get_translation() {
		Ok(translation) => params.insert("translation", translation),
		Err(raw_text) => params.insert("text", raw_text),
	};

	if let Some(tooltip) = setting.get_tooltip() {
		params.insert("tooltip", tooltip);
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
		let feats = mp.feats;
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
						if item
							.get_str("Backend")
							.is_some_and(|x| XrBackend::try_from(x).unwrap() != feats.xr_backend)
						{
							return None;
						}
						if item
							.get_str("Desktop")
							.is_some_and(|x| DesktopBackend::try_from(x).unwrap() != feats.desktop_backend)
						{
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

	let mut params = TemplateParams::new();
	params.insert("id", &id);
	params.insert("translation", translation);
	params.insert("icon", icon);

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

pub fn options_button(
	mp: &mut MacroParams,
	parent: WidgetID,
	translation: &str,
	icon: &str,
	task: Task,
) -> anyhow::Result<()> {
	let id = mp.idx.to_string();
	mp.idx += 1;

	let mut params = TemplateParams::new();
	params.insert("id", &id);
	params.insert("translation", translation);
	params.insert("icon", icon);

	mp.parser_state
		.instantiate_template(mp.doc_params, "ButtonText", mp.layout, parent, params)?;

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

	let mut params = TemplateParams::new();
	params.insert("id", &id);
	params.insert("text", text);

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
	pub feats: InterfaceFeats,
}
