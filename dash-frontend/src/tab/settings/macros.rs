use std::rc::Rc;

use crate::{
	tab::settings::{self, SettingTypeImpl, Task, horiz_cell, mount_requires_restart},
	util::wgui_simple,
};
use wgui::{
	assets::AssetPathRef,
	components::{button::ComponentButton, checkbox::ComponentCheckbox, slider::ComponentSlider},
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState, TemplateParams},
	task::Tasks,
	windowing::context_menu,
};
use wlx_common::{
	DesktopBackend, XrBackend,
	config::{GeneralConfig, SettingType, SettingVar, SettingVarRef},
	dash_interface::InterfaceFeats,
};

// built-in asset path
fn get_icon_from_setting_type(t: SettingType) -> Option<&'static str> {
	match t {
		SettingType::Clock12h => Some("dashboard/hours_12.svg"),
		SettingType::NotificationsEnabled => Some("dashboard/notification.svg"),
		SettingType::NotificationsSoundEnabled => Some("dashboard/notification.svg"),
		SettingType::KeyboardSoundEnabled => Some("dashboard/keyboard.svg"),
		SettingType::OpaqueBackground => Some("dashboard/translucent.svg"),
		SettingType::ShowUsername => Some("dashboard/person.svg"),
		SettingType::UseSkybox => Some("dashboard/globe.svg"),
		SettingType::UsePassthrough => Some("dashboard/camera.svg"),
		SettingType::BlockGameInput => Some("dashboard/block.svg"),
		SettingType::BlockGameInputIgnoreWatch => Some("dashboard/block.svg"),
		SettingType::BlockPosesOnKbdInteraction => Some("dashboard/block.svg"),
		SettingType::SpaceRotateUnlocked => Some("dashboard/rotate.svg"),
		SettingType::SpaceDragUnlocked => Some("dashboard/drag.svg"),
		SettingType::InvertScrollDirectionX => Some("dashboard/horizontal.svg"),
		SettingType::InvertScrollDirectionY => Some("dashboard/vertical.svg"),
		SettingType::EnableWatch => Some("dashboard/watch.svg"),
		SettingType::SetsOnWatch => Some("dashboard/123.svg"),
		SettingType::MouseAcceleration => Some("dashboard/mouse.svg"),
		SettingType::ShowGrabHelp => Some("dashboard/help.svg"),
		SettingType::LeftHandedMouse => Some("dashboard/tools.svg"),
		SettingType::XwaylandByDefault => Some("dashboard/tools.svg"),
		SettingType::UprightScreenFix => Some("dashboard/tools.svg"),
		SettingType::DoubleCursorFix => Some("dashboard/tools.svg"),
		SettingType::ScreenRenderDown => Some("dashboard/tools.svg"),
		SettingType::Language => Some("dashboard/language.svg"),
		SettingType::DefaultPositioning => Some("dashboard/drag.svg"),
		SettingType::KeyboardMiddleClick => Some("dashboard/keyboard.svg"),
		SettingType::InputCaptureMethod => Some("dashboard/input.svg"),
		SettingType::InputEmulationMethod => Some("dashboard/input.svg"),
		SettingType::CaptureMethod => Some("dashboard/monitor.svg"),
		_ => None,
	}
}

pub fn options_split_horiz2(mp: &mut MacroParams, parent: WidgetID) -> anyhow::Result<(WidgetID, WidgetID)> {
	let id1 = mp.gen_id();
	let id2 = mp.gen_id();

	let mut params = TemplateParams::new();
	params.insert("id1", &id1);
	params.insert("id2", &id2);

	mp.parser_state
		.instantiate_template(mp.doc_params, "SettingsSplitHoriz2", mp.layout, parent, params)?;

	Ok((
		mp.parser_state.get_widget_id(&id1)?,
		mp.parser_state.get_widget_id(&id2)?,
	))
}

pub fn options_split_horiz3(mp: &mut MacroParams, parent: WidgetID) -> anyhow::Result<(WidgetID, WidgetID, WidgetID)> {
	let id1 = mp.gen_id();
	let id2 = mp.gen_id();
	let id3 = mp.gen_id();

	let mut params = TemplateParams::new();
	params.insert("id1", &id1);
	params.insert("id2", &id2);
	params.insert("id3", &id3);

	mp.parser_state
		.instantiate_template(mp.doc_params, "SettingsSplitHoriz3", mp.layout, parent, params)?;

	Ok((
		mp.parser_state.get_widget_id(&id1)?,
		mp.parser_state.get_widget_id(&id2)?,
		mp.parser_state.get_widget_id(&id3)?,
	))
}

pub fn options_category(
	mp: &mut MacroParams,
	parent: WidgetID,
	translation: &str,
	icon: &str,
) -> anyhow::Result<WidgetID> {
	let id = mp.gen_id();

	let mut params = TemplateParams::new();
	params.insert("translation", translation);
	params.insert("icon", icon);
	params.insert("id", &id);

	mp.parser_state
		.instantiate_template(mp.doc_params, "SettingsGroupBox", mp.layout, parent, params)?;

	mp.parser_state.get_widget_id(&id)
}

pub fn options_stat_row(mp: &mut MacroParams, parent: WidgetID, translation: &str) -> anyhow::Result<WidgetID> {
	let id = mp.gen_id();

	let mut params = TemplateParams::new();
	params.insert("id", &id);
	params.insert("translation", translation);

	mp.parser_state
		.instantiate_template(mp.doc_params, "StatRow", mp.layout, parent, params)?;

	mp.parser_state.get_widget_id(&id)
}

pub fn options_diagnostics_row(
	mp: &mut MacroParams,
	parent: WidgetID,
	title: &str,
	tooltip_str: &str,
	checked: bool,
) -> anyhow::Result<()> {
	let mut params = TemplateParams::new();
	params.insert("title", title);
	params.insert("tooltip_str", tooltip_str);
	params.insert("color", if checked { "success" } else { "danger" });

	params.insert(
		"sprite",
		if checked {
			"@/dashboard/check.svg"
		} else {
			"@/dashboard/cross.svg"
		},
	);

	mp.parser_state
		.instantiate_template(mp.doc_params, "DiagnosticsRow", mp.layout, parent, params)?;

	Ok(())
}

pub fn options_diagnostics_drm_card(
	mp: &mut MacroParams,
	parent: WidgetID,
	name: &str,
	info: &str,
) -> anyhow::Result<()> {
	let mut params = TemplateParams::new();
	params.insert("name", name);
	params.insert("info", info);

	mp.parser_state
		.instantiate_template(mp.doc_params, "DiagnosticsDrmCard", mp.layout, parent, params)?;

	Ok(())
}

pub fn options_checkbox<'a>(mp: &'a mut MacroParams, parent: WidgetID, setting: SettingType) -> anyhow::Result<()> {
	let id = mp.gen_id();

	let mut params = TemplateParams::new();
	params.insert("id", &id);

	if let Some(src) = get_icon_from_setting_type(setting) {
		params.insert_str("sprite_src_builtin", format!("@/{}", src));
	}

	match setting.get_translation() {
		Ok(translation) => params.insert("translation", translation),
		Err(raw_text) => params.insert("text", raw_text),
	};

	if let Some(tooltip) = setting.get_tooltip() {
		params.insert("tooltip", tooltip);
	}

	let checked_t: &'a mut bool = setting.mut_var(mp.config).try_into()?;
	let checked = if *checked_t { "1" } else { "0" };
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
			tasks.push(Task::UpdateVar(setting, SettingVar::Bool(e.checked)));
			Ok(())
		}
	}));

	Ok(())
}

pub fn options_range_f32(
	mp: &mut MacroParams,
	parent: WidgetID,
	setting1: SettingType,
	setting2: SettingType,
	min: f32,
	max: f32,
	step: f32,
) -> anyhow::Result<()> {
	let id = mp.gen_id();

	let mut params = TemplateParams::new();
	params.insert("id", &id);

	match setting1.get_translation() {
		Ok(translation) => params.insert("translation", translation),
		Err(raw_text) => params.insert("text", raw_text),
	};

	if let Some(tooltip) = setting1.get_tooltip() {
		params.insert("tooltip", tooltip);
	}

	let value1_t: &mut f32 = setting1.mut_var(mp.config).try_into()?;
	let value1 = value1_t.to_string();
	let value2_t: &mut f32 = setting2.mut_var(mp.config).try_into()?;
	let value2 = value2_t.to_string();

	params.insert_str("value", value1);
	params.insert_str("value2", value2);
	params.insert_rc("min", min.to_string().into());
	params.insert_rc("max", max.to_string().into());
	params.insert_rc("step", step.to_string().into());

	let id_cell = horiz_cell(mp.layout, parent)?;

	mp.parser_state
		.instantiate_template(mp.doc_params, "RangeSetting", mp.layout, id_cell, params)?;

	if setting1.requires_restart() {
		mount_requires_restart(mp.layout, id_cell)?;
	}

	let slider = mp.parser_state.fetch_component_as::<ComponentSlider>(&id)?;
	slider.on_value_changed(Box::new({
		let tasks = mp.tasks.clone();
		move |_common, e| {
			if matches!(e.index, wgui::components::slider::ValueIndex::Primary) {
				tasks.push(Task::UpdateVar(setting1, SettingVar::F32(e.value)));
			} else {
				tasks.push(Task::UpdateVar(setting2, SettingVar::F32(e.value)));
			}
		}
	}));

	Ok(())
}

pub fn options_slider<'a, NumType>(
	mp: &'a mut MacroParams,
	parent: WidgetID,
	setting: SettingType,
	min: NumType,
	max: NumType,
	step: NumType,
) -> anyhow::Result<()>
where
	NumType: std::string::ToString + 'static,
	&'a mut NumType: TryFrom<SettingVarRef<'a>, Error = anyhow::Error>,
{
	let id = mp.gen_id();

	let mut params = TemplateParams::new();
	params.insert("id", &id);

	match setting.get_translation() {
		Ok(translation) => params.insert("translation", translation),
		Err(raw_text) => params.insert("text", raw_text),
	};

	if let Some(tooltip) = setting.get_tooltip() {
		params.insert("tooltip", tooltip);
	}

	let value_t: &'a mut NumType = setting.mut_var(mp.config).try_into()?;
	let value = value_t.to_string();

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
			tasks.push(Task::UpdateVar(setting, e.value.into()));
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
	let id = mp.gen_id();

	let mut params = TemplateParams::new();

	let sprite_src = get_icon_from_setting_type(*setting);

	match setting.get_translation() {
		Ok(translation) => params.insert("translation", translation),
		Err(raw_text) => params.insert("text", raw_text),
	};

	if let Some(tooltip) = setting.get_tooltip() {
		params.insert("tooltip", tooltip);
	}

	let id_cell = horiz_cell(mp.layout, parent)?;

	let setting_str = setting.as_ref();
	let title = setting.get_enum_title(mp.config);

	let feats = mp.feats;
	let cells = EnumType::VARIANTS
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

			let sprite_src_builtin = item
				.get_str("CountryCode")
				.map(|code| Rc::from(format!("@/dashboard/flags/{}.svg", code)));

			let value = item.as_ref();
			let title = SettingType::get_enum_title_inner(*item);
			let tooltip = SettingType::get_enum_tooltip_inner(*item);

			let text = &title.text;
			let translated = if title.translated { "1" } else { "0" };
			let action = format!("{setting_str};{id};{value};{text};{translated}");

			Some(context_menu::Cell::Button(context_menu::Button {
				action_name: Some(action.into()),
				title,
				tooltip,
				sprite_src_builtin,
				..Default::default()
			}))
		})
		.collect();

	let tasks = mp.tasks.clone();
	wgui_simple::mount_dropdown(wgui_simple::MountDropdownParams {
		layout: mp.layout,
		parser_state: mp.parser_state,
		doc_params: mp.doc_params,
		parent: id_cell,
		id,
		params,
		current_text: title,
		cells,
		sprite_src: sprite_src.map(AssetPathRef::BuiltIn),
		on_open: Box::new(move |pos, cells| tasks.push(Task::OpenContextMenu(pos, cells))),
	})?;

	if setting.requires_restart() {
		mount_requires_restart(mp.layout, id_cell)?;
	}

	Ok(())
}

pub fn options_danger_button(
	mp: &mut MacroParams,
	parent: WidgetID,
	translation: &str,
	icon: &str,
	task: Task,
) -> anyhow::Result<()> {
	let id = mp.gen_id();

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
	let id = mp.gen_id();

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
	let id = mp.gen_id();

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
	pub feats: InterfaceFeats,
	pub id_counter: usize,
}

impl<'ż> MacroParams<'ż> {
	pub fn gen_id(&mut self) -> String {
		let id = self.id_counter.to_string();
		self.id_counter += 1;
		id
	}
}
