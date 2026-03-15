use std::{collections::HashMap, marker::PhantomData, rc::Rc, str::FromStr};

use glam::Vec2;
use strum::{AsRefStr, EnumProperty, EnumString, VariantArray};
use wgui::{
	assets::AssetPath,
	components::{
		button::{ButtonClickEvent, ComponentButton},
		checkbox::ComponentCheckbox,
		slider::ComponentSlider,
		tabs::ComponentTabs,
	},
	drawing,
	event::{CallbackDataCommon, EventAlterables},
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	log::LogErr,
	parser::{Fetchable, ParseDocumentParams, ParserState},
	renderer_vk::text::{FontWeight, TextStyle},
	taffy::{self, prelude::length},
	task::Tasks,
	widget::{
		div::WidgetDiv,
		label::{WidgetLabel, WidgetLabelParams},
	},
	windowing::context_menu::{self, Blueprint, ContextMenu, TickResult},
};
use wlx_common::{config::GeneralConfig, config_io::ConfigRoot, dash_interface::RecenterMode};

use crate::{
	frontend::{Frontend, FrontendTask},
	tab::{Tab, TabType},
};

#[derive(Clone)]
enum TabNameEnum {
	LookAndFeel,
	Features,
	Controls,
	Misc,
	AutostartApps,
	Troubleshooting,
}

impl TabNameEnum {
	fn from_string(s: &str) -> Option<Self> {
		match s {
			"look_and_feel" => Some(TabNameEnum::LookAndFeel),
			"features" => Some(TabNameEnum::Features),
			"controls" => Some(TabNameEnum::Controls),
			"misc" => Some(TabNameEnum::Misc),
			"autostart_apps" => Some(TabNameEnum::AutostartApps),
			"troubleshooting" => Some(TabNameEnum::Troubleshooting),
			_ => None,
		}
	}
}

enum Task {
	UpdateBool(SettingType, bool),
	UpdateFloat(SettingType, f32),
	UpdateInt(SettingType, i32),
	SettingUpdated(SettingType),
	OpenContextMenu(Vec2, Vec<context_menu::Cell>),
	ClearPipewireTokens,
	ClearSavedState,
	DeleteAllConfigs,
	ResetPlayspace,
	RestartSoftware,
	RemoveAutostartApp(Rc<str>),
	SetTab(TabNameEnum),
}

pub struct TabSettings<T> {
	pub state: ParserState,

	app_button_ids: Vec<Rc<str>>,
	context_menu: ContextMenu,

	tasks: Tasks<Task>,
	marker: PhantomData<T>,
}

impl<T> Tab<T> for TabSettings<T> {
	fn get_type(&self) -> TabType {
		TabType::Settings
	}

	fn update(&mut self, frontend: &mut Frontend<T>, _time_ms: u32, data: &mut T) -> anyhow::Result<()> {
		let mut changed = false;
		for task in self.tasks.drain() {
			match task {
				Task::SetTab(tab) => {
					self.set_tab(frontend, data, tab)?;
				}
				Task::UpdateBool(setting, n) => {
					self.tasks.push(Task::SettingUpdated(setting));
					if let Some(task) = setting.get_frontend_task() {
						frontend.tasks.push(task)
					}
					let config = frontend.interface.general_config(data);
					*setting.mut_bool(config) = n;
					changed = true;
				}
				Task::UpdateFloat(setting, n) => {
					self.tasks.push(Task::SettingUpdated(setting));
					if let Some(task) = setting.get_frontend_task() {
						frontend.tasks.push(task)
					}
					let config = frontend.interface.general_config(data);
					*setting.mut_f32(config) = n;
					changed = true;
				}
				Task::UpdateInt(setting, n) => {
					self.tasks.push(Task::SettingUpdated(setting));
					if let Some(task) = setting.get_frontend_task() {
						frontend.tasks.push(task)
					}
					let config = frontend.interface.general_config(data);
					*setting.mut_i32(config) = n;
					changed = true;
				}
				Task::ClearPipewireTokens => {
					let _ = std::fs::remove_file(ConfigRoot::Generic.get_conf_d_path().join("pw_tokens.yaml"))
						.log_err("Could not remove pw_tokens.yaml");
				}
				Task::ClearSavedState => {
					let _ = std::fs::remove_file(ConfigRoot::Generic.get_conf_d_path().join("zz-saved-state.json5"))
						.log_err("Could not remove zz-saved-state.json5");
				}
				Task::DeleteAllConfigs => {
					let path = ConfigRoot::Generic.get_conf_d_path();
					std::fs::remove_dir_all(&path)?;
					std::fs::create_dir(&path)?;
				}
				Task::ResetPlayspace => {
					frontend.interface.recenter_playspace(data, RecenterMode::Reset)?;
					return Ok(());
				}
				Task::RestartSoftware => {
					frontend.interface.restart(data);
					return Ok(());
				}
				Task::OpenContextMenu(position, cells) => {
					self.context_menu.open(context_menu::OpenParams {
						on_custom_attribs: None,
						position,
						blueprint: Blueprint::Cells(cells),
					});
				}
				Task::RemoveAutostartApp(button_id) => {
					if let (Some(idx), Ok(widget)) = (
						self.app_button_ids.iter().position(|x| button_id.eq(x)),
						self.state.get_widget_id(&format!("{button_id}_root")),
					) {
						self.app_button_ids.remove(idx);
						let config = frontend.interface.general_config(data);
						config.autostart_apps.remove(idx);
						frontend.layout.remove_widget(widget);
						changed = true;
					}
				}
				Task::SettingUpdated(setting) => match setting {
					SettingType::UiAnimationSpeed | SettingType::UiGradientIntensity | SettingType::UiRoundMultiplier => {
						// todo: currently, wayvr restart is required to apply these changes (WguiTheme is Rc)
					}
					_ => { /* do nothing */ }
				},
			}
		}

		// Dropdown handling
		if let TickResult::Action(name) = self.context_menu.tick(&mut frontend.layout, &mut self.state)?
			&& let (Some(setting), Some(id), Some(value), Some(text), Some(translated)) = {
				let mut s = name.splitn(5, ';');
				(s.next(), s.next(), s.next(), s.next(), s.next())
			} {
			let mut label = self
				.state
				.fetch_widget_as::<WidgetLabel>(&frontend.layout.state, &format!("{id}_value"))?;

			let mut alterables = EventAlterables::default();
			let mut common = CallbackDataCommon {
				alterables: &mut alterables,
				state: &frontend.layout.state,
			};

			let translation = Translation {
				text: text.into(),
				translated: translated == "1",
			};

			label.set_text(&mut common, translation);

			let setting = SettingType::from_str(setting).expect("Invalid Enum string");
			let config = frontend.interface.general_config(data);
			setting.set_enum(config, value);
			changed = true;
		}

		// Notify overlays of the change
		if changed {
			frontend.interface.config_changed(data);
		}

		Ok(())
	}
}

// Sorted alphabetically
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, AsRefStr, EnumString)]
enum SettingType {
	AllowSliding,
	BlockGameInput,
	BlockGameInputIgnoreWatch,
	BlockPosesOnKbdInteraction,
	CaptureMethod,
	ClickFreezeTimeMs,
	Clock12h,
	DoubleCursorFix,
	FocusFollowsMouseMode,
	HandsfreePointer,
	HideGrabHelp,
	HideUsername,
	InvertScrollDirectionX,
	InvertScrollDirectionY,
	KeyboardMiddleClick,
	KeyboardSoundEnabled,
	KeyboardSwipeToTypeEnabled,
	Language,
	LeftHandedMouse,
	LongPressDuration,
	NotificationsEnabled,
	NotificationsSoundEnabled,
	OpaqueBackground,
	PointerLerpFactor,
	ScreenRenderDown,
	ScrollSpeed,
	SetsOnWatch,
	SpaceDragMultiplier,
	SpaceDragUnlocked,
	SpaceRotateUnlocked,
	UiAnimationSpeed,
	UiGradientIntensity,
	UiRoundMultiplier,
	UprightScreenFix,
	UsePassthrough,
	UseSkybox,
	GridOpacity,
	XrClickSensitivity,
	XrClickSensitivityRelease,
	XwaylandByDefault,
}

impl SettingType {
	pub fn mut_bool(self, config: &mut GeneralConfig) -> &mut bool {
		match self {
			Self::InvertScrollDirectionX => &mut config.invert_scroll_direction_x,
			Self::InvertScrollDirectionY => &mut config.invert_scroll_direction_y,
			Self::NotificationsEnabled => &mut config.notifications_enabled,
			Self::NotificationsSoundEnabled => &mut config.notifications_sound_enabled,
			Self::KeyboardSoundEnabled => &mut config.keyboard_sound_enabled,
			Self::UprightScreenFix => &mut config.upright_screen_fix,
			Self::DoubleCursorFix => &mut config.double_cursor_fix,
			Self::SetsOnWatch => &mut config.sets_on_watch,
			Self::HideGrabHelp => &mut config.hide_grab_help,
			Self::AllowSliding => &mut config.allow_sliding,
			Self::FocusFollowsMouseMode => &mut config.focus_follows_mouse_mode,
			Self::LeftHandedMouse => &mut config.left_handed_mouse,
			Self::BlockGameInput => &mut config.block_game_input,
			Self::BlockGameInputIgnoreWatch => &mut config.block_game_input_ignore_watch,
			Self::BlockPosesOnKbdInteraction => &mut config.block_poses_on_kbd_interaction,
			Self::UseSkybox => &mut config.use_skybox,
			Self::UsePassthrough => &mut config.use_passthrough,
			Self::ScreenRenderDown => &mut config.screen_render_down,
			Self::SpaceDragUnlocked => &mut config.space_drag_unlocked,
			Self::SpaceRotateUnlocked => &mut config.space_rotate_unlocked,
			Self::Clock12h => &mut config.clock_12h,
			Self::HideUsername => &mut config.hide_username,
			Self::OpaqueBackground => &mut config.opaque_background,
			Self::XwaylandByDefault => &mut config.xwayland_by_default,
			Self::KeyboardSwipeToTypeEnabled => &mut config.keyboard_swipe_to_type_enabled,
			_ => panic!("Requested bool for non-bool SettingType"),
		}
	}

	pub fn mut_f32(self, config: &mut GeneralConfig) -> &mut f32 {
		match self {
			Self::UiAnimationSpeed => &mut config.ui_animation_speed,
			Self::UiGradientIntensity => &mut config.ui_gradient_intensity,
			Self::UiRoundMultiplier => &mut config.ui_round_multiplier,
			Self::ScrollSpeed => &mut config.scroll_speed,
			Self::LongPressDuration => &mut config.long_press_duration,
			Self::XrClickSensitivity => &mut config.xr_click_sensitivity,
			Self::XrClickSensitivityRelease => &mut config.xr_click_sensitivity_release,
			Self::SpaceDragMultiplier => &mut config.space_drag_multiplier,
			Self::PointerLerpFactor => &mut config.pointer_lerp_factor,
			Self::GridOpacity => &mut config.grid_opacity,
			_ => panic!("Requested f32 for non-f32 SettingType"),
		}
	}

	pub fn mut_i32(self, config: &mut GeneralConfig) -> &mut i32 {
		match self {
			Self::ClickFreezeTimeMs => &mut config.click_freeze_time_ms,
			_ => panic!("Requested i32 for non-i32 SettingType"),
		}
	}

	pub fn set_enum(self, config: &mut GeneralConfig, value: &str) {
		match self {
			Self::CaptureMethod => {
				config.capture_method = wlx_common::config::CaptureMethod::from_str(value).expect("Invalid enum value!")
			}
			Self::KeyboardMiddleClick => {
				config.keyboard_middle_click_mode =
					wlx_common::config::AltModifier::from_str(value).expect("Invalid enum value!")
			}
			Self::HandsfreePointer => {
				config.handsfree_pointer = wlx_common::config::HandsfreePointer::from_str(value).expect("Invalid enum value!")
			}
			Self::Language => {
				config.language = Some(wlx_common::locale::Language::from_str(value).expect("Invalid enum value!"))
			}
			_ => panic!("Requested enum for non-enum SettingType"),
		}
	}

	fn get_enum_title(self, config: &mut GeneralConfig) -> Translation {
		match self {
			Self::CaptureMethod => Self::get_enum_title_inner(config.capture_method),
			Self::KeyboardMiddleClick => Self::get_enum_title_inner(config.keyboard_middle_click_mode),
			Self::HandsfreePointer => Self::get_enum_title_inner(config.handsfree_pointer),
			Self::Language => match &config.language {
				Some(lang) => Self::get_enum_title_inner(*lang),
				None => Translation::from_translation_key("APP_SETTINGS.OPTION.AUTO"),
			},
			_ => panic!("Requested enum for non-enum SettingType"),
		}
	}

	fn get_enum_title_inner<E>(value: E) -> Translation
	where
		E: EnumProperty + AsRef<str>,
	{
		value
			.get_str("Translation")
			.map(Translation::from_translation_key)
			.or_else(|| value.get_str("Text").map(Translation::from_raw_text))
			.unwrap_or_else(|| Translation::from_raw_text(value.as_ref()))
	}

	fn get_enum_tooltip_inner<E>(value: E) -> Option<Translation>
	where
		E: EnumProperty + AsRef<str>,
	{
		value.get_str("Tooltip").map(Translation::from_translation_key)
	}

	/// Ok is translation, Err is raw text
	/// `match` sorted alphabetically
	fn get_translation(self) -> Result<&'static str, &'static str> {
		match self {
			Self::AllowSliding => Ok("APP_SETTINGS.ALLOW_SLIDING"),
			Self::BlockGameInput => Ok("APP_SETTINGS.BLOCK_GAME_INPUT"),
			Self::BlockGameInputIgnoreWatch => Ok("APP_SETTINGS.BLOCK_GAME_INPUT_IGNORE_WATCH"),
			Self::BlockPosesOnKbdInteraction => Ok("APP_SETTINGS.BLOCK_POSES_ON_KBD_INTERACTION"),
			Self::CaptureMethod => Ok("APP_SETTINGS.CAPTURE_METHOD"),
			Self::ClickFreezeTimeMs => Ok("APP_SETTINGS.CLICK_FREEZE_TIME_MS"),
			Self::Clock12h => Ok("APP_SETTINGS.CLOCK_12H"),
			Self::DoubleCursorFix => Ok("APP_SETTINGS.DOUBLE_CURSOR_FIX"),
			Self::FocusFollowsMouseMode => Ok("APP_SETTINGS.FOCUS_FOLLOWS_MOUSE_MODE"),
			Self::GridOpacity => Ok("APP_SETTINGS.GRID_OPACITY"),
			Self::HandsfreePointer => Ok("APP_SETTINGS.HANDSFREE_POINTER"),
			Self::HideGrabHelp => Ok("APP_SETTINGS.HIDE_GRAB_HELP"),
			Self::HideUsername => Ok("APP_SETTINGS.HIDE_USERNAME"),
			Self::InvertScrollDirectionX => Ok("APP_SETTINGS.INVERT_SCROLL_DIRECTION_X"),
			Self::InvertScrollDirectionY => Ok("APP_SETTINGS.INVERT_SCROLL_DIRECTION_Y"),
			Self::KeyboardMiddleClick => Ok("APP_SETTINGS.KEYBOARD_MIDDLE_CLICK"),
			Self::KeyboardSoundEnabled => Ok("APP_SETTINGS.KEYBOARD_SOUND_ENABLED"),
			Self::KeyboardSwipeToTypeEnabled => Ok("APP_SETTINGS.KEYBOARD_SWIPE_TO_TYPE_ENABLED"),
			Self::Language => Ok("APP_SETTINGS.LANGUAGE"),
			Self::LeftHandedMouse => Ok("APP_SETTINGS.LEFT_HANDED_MOUSE"),
			Self::LongPressDuration => Ok("APP_SETTINGS.LONG_PRESS_DURATION"),
			Self::NotificationsEnabled => Ok("APP_SETTINGS.NOTIFICATIONS_ENABLED"),
			Self::NotificationsSoundEnabled => Ok("APP_SETTINGS.NOTIFICATIONS_SOUND_ENABLED"),
			Self::OpaqueBackground => Ok("APP_SETTINGS.OPAQUE_BACKGROUND"),
			Self::PointerLerpFactor => Ok("APP_SETTINGS.POINTER_LERP_FACTOR"),
			Self::ScreenRenderDown => Ok("APP_SETTINGS.SCREEN_RENDER_DOWN"),
			Self::ScrollSpeed => Ok("APP_SETTINGS.SCROLL_SPEED"),
			Self::SetsOnWatch => Ok("APP_SETTINGS.SETS_ON_WATCH"),
			Self::SpaceDragMultiplier => Ok("APP_SETTINGS.SPACE_DRAG_MULTIPLIER"),
			Self::SpaceDragUnlocked => Ok("APP_SETTINGS.SPACE_DRAG_UNLOCKED"),
			Self::SpaceRotateUnlocked => Ok("APP_SETTINGS.SPACE_ROTATE_UNLOCKED"),
			Self::UiAnimationSpeed => Ok("APP_SETTINGS.ANIMATION_SPEED"),
			Self::UiGradientIntensity => Ok("APP_SETTINGS.UI_GRADIENT_INTENSITY"),
			Self::UiRoundMultiplier => Ok("APP_SETTINGS.ROUND_MULTIPLIER"),
			Self::UprightScreenFix => Ok("APP_SETTINGS.UPRIGHT_SCREEN_FIX"),
			Self::UsePassthrough => Ok("APP_SETTINGS.USE_PASSTHROUGH"),
			Self::UseSkybox => Ok("APP_SETTINGS.USE_SKYBOX"),
			Self::XrClickSensitivity => Ok("APP_SETTINGS.XR_CLICK_SENSITIVITY"),
			Self::XrClickSensitivityRelease => Ok("APP_SETTINGS.XR_CLICK_SENSITIVITY_RELEASE"),
			Self::XwaylandByDefault => Ok("APP_SETTINGS.XWAYLAND_BY_DEFAULT"),
		}
	}

	/// `match` sorted alphabetically
	fn get_tooltip(self) -> Option<&'static str> {
		match self {
			Self::BlockGameInput => Some("APP_SETTINGS.BLOCK_GAME_INPUT_HELP"),
			Self::BlockGameInputIgnoreWatch => Some("APP_SETTINGS.BLOCK_GAME_INPUT_IGNORE_WATCH_HELP"),
			Self::BlockPosesOnKbdInteraction => Some("APP_SETTINGS.BLOCK_POSES_ON_KBD_INTERACTION_HELP"),
			Self::CaptureMethod => Some("APP_SETTINGS.CAPTURE_METHOD_HELP"),
			Self::DoubleCursorFix => Some("APP_SETTINGS.DOUBLE_CURSOR_FIX_HELP"),
			Self::GridOpacity => Some("APP_SETTINGS.GRID_OPACITY_HELP"),
			Self::HandsfreePointer => Some("APP_SETTINGS.HANDSFREE_POINTER_HELP"),
			Self::KeyboardMiddleClick => Some("APP_SETTINGS.KEYBOARD_MIDDLE_CLICK_HELP"),
			Self::KeyboardSwipeToTypeEnabled => Some("APP_SETTINGS.KEYBOARD_SWIPE_TO_TYPE_ENABLED_HELP"),
			Self::LeftHandedMouse => Some("APP_SETTINGS.LEFT_HANDED_MOUSE_HELP"),
			Self::ScreenRenderDown => Some("APP_SETTINGS.SCREEN_RENDER_DOWN_HELP"),
			Self::UprightScreenFix => Some("APP_SETTINGS.UPRIGHT_SCREEN_FIX_HELP"),
			Self::UsePassthrough => Some("APP_SETTINGS.USE_PASSTHROUGH_HELP"),
			Self::UseSkybox => Some("APP_SETTINGS.USE_SKYBOX_HELP"),
			Self::XrClickSensitivity => Some("APP_SETTINGS.XR_CLICK_SENSITIVITY_HELP"),
			Self::XrClickSensitivityRelease => Some("APP_SETTINGS.XR_CLICK_SENSITIVITY_RELEASE_HELP"),
			_ => None,
		}
	}

	fn requires_restart(self) -> bool {
		matches!(
			self,
			Self::UiAnimationSpeed
				| Self::UiRoundMultiplier
				| Self::UiGradientIntensity
				| Self::UprightScreenFix
				| Self::DoubleCursorFix
				| Self::ScreenRenderDown
				| Self::Language
				| Self::CaptureMethod
		)
	}

	fn get_frontend_task(self) -> Option<FrontendTask> {
		match self {
			Self::Clock12h => Some(FrontendTask::RefreshClock),
			Self::OpaqueBackground => Some(FrontendTask::RefreshBackground),
			_ => None,
		}
	}
}

// creates a simple div with horizontal, centered flow
fn horiz_cell(layout: &mut Layout, parent: WidgetID) -> anyhow::Result<WidgetID> {
	let (pair, _) = layout.add_child(
		parent,
		WidgetDiv::create(),
		taffy::Style {
			flex_direction: taffy::FlexDirection::Row,
			align_items: Some(taffy::AlignItems::Center),
			gap: length(8.0),
			..Default::default()
		},
	)?;

	Ok(pair.id)
}

fn mount_requires_restart(layout: &mut Layout, parent: WidgetID) -> anyhow::Result<()> {
	let content = Translation::from_translation_key("APP_SETTINGS.REQUIRES_RESTART");
	let label = WidgetLabel::create(
		&mut layout.state,
		WidgetLabelParams {
			content,
			style: TextStyle {
				wrap: false,
				color: Some(drawing::Color::new(1.0, 0.5, 0.5, 1.0)),
				weight: Some(FontWeight::Bold),
				size: Some(10.0),
				..Default::default()
			},
		},
	);

	layout.add_child(parent, label, Default::default())?;
	Ok(())
}

macro_rules! category {
	($pe:expr, $root:expr, $translation:expr, $icon:expr) => {{
		let id = $pe.idx.to_string();
		$pe.idx += 1;

		let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
		params.insert(Rc::from("translation"), Rc::from($translation));
		params.insert(Rc::from("icon"), Rc::from($icon));
		params.insert(Rc::from("id"), Rc::from(id.as_ref()));

		$pe
			.parser_state
			.instantiate_template($pe.doc_params, "SettingsGroupBox", $pe.layout, $root, params)?;

		$pe.parser_state.get_widget_id(&id)
	}};
}

macro_rules! checkbox {
	($mp:expr, $root:expr, $setting:expr) => {
		let id = $mp.idx.to_string();
		$mp.idx += 1;

		let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
		params.insert(Rc::from("id"), Rc::from(id.as_ref()));

		match $setting.get_translation() {
			Ok(translation) => params.insert(Rc::from("translation"), translation.into()),
			Err(raw_text) => params.insert(Rc::from("text"), raw_text.into()),
		};

		if let Some(tooltip) = $setting.get_tooltip() {
			params.insert(Rc::from("tooltip"), Rc::from(tooltip));
		}

		let checked = if *$setting.mut_bool($mp.config) { "1" } else { "0" };
		params.insert(Rc::from("checked"), Rc::from(checked));

		let id_cell = horiz_cell($mp.layout, $root)?;

		$mp
			.parser_state
			.instantiate_template($mp.doc_params, "CheckBoxSetting", $mp.layout, id_cell, params)?;

		if $setting.requires_restart() {
			mount_requires_restart($mp.layout, id_cell)?;
		}

		let checkbox = $mp.parser_state.fetch_component_as::<ComponentCheckbox>(&id)?;
		checkbox.on_toggle(Box::new({
			let tasks = $mp.tasks.clone();
			move |_common, e| {
				tasks.push(Task::UpdateBool($setting, e.checked));
				Ok(())
			}
		}));
	};
}

macro_rules! slider_f32 {
	($mp:expr, $root:expr, $setting:expr, $min:expr, $max:expr, $step:expr) => {
		let id = $mp.idx.to_string();
		$mp.idx += 1;

		let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
		params.insert(Rc::from("id"), Rc::from(id.as_ref()));

		match $setting.get_translation() {
			Ok(translation) => params.insert(Rc::from("translation"), translation.into()),
			Err(raw_text) => params.insert(Rc::from("text"), raw_text.into()),
		};

		if let Some(tooltip) = $setting.get_tooltip() {
			params.insert(Rc::from("tooltip"), Rc::from(tooltip));
		}

		let value = $setting.mut_f32($mp.config).to_string();
		params.insert(Rc::from("value"), Rc::from(value));
		params.insert(Rc::from("min"), Rc::from($min.to_string()));
		params.insert(Rc::from("max"), Rc::from($max.to_string()));
		params.insert(Rc::from("step"), Rc::from($step.to_string()));

		let id_cell = horiz_cell($mp.layout, $root)?;

		$mp
			.parser_state
			.instantiate_template($mp.doc_params, "SliderSetting", $mp.layout, id_cell, params)?;

		if $setting.requires_restart() {
			mount_requires_restart($mp.layout, id_cell)?;
		}

		let slider = $mp.parser_state.fetch_component_as::<ComponentSlider>(&id)?;
		slider.on_value_changed(Box::new({
			let tasks = $mp.tasks.clone();
			move |_common, e| {
				tasks.push(Task::UpdateFloat($setting, e.value));
				Ok(())
			}
		}));
	};
}

macro_rules! slider_i32 {
	($mp:expr, $root:expr, $setting:expr, $min:expr, $max:expr, $step:expr) => {
		let id = $mp.idx.to_string();
		$mp.idx += 1;

		let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
		params.insert(Rc::from("id"), Rc::from(id.as_ref()));

		match $setting.get_translation() {
			Ok(translation) => params.insert(Rc::from("translation"), translation.into()),
			Err(raw_text) => params.insert(Rc::from("text"), raw_text.into()),
		};

		if let Some(tooltip) = $setting.get_tooltip() {
			params.insert(Rc::from("tooltip"), Rc::from(tooltip));
		}

		let id_cell = horiz_cell($mp.layout, $root)?;

		let value = $setting.mut_i32($mp.config).to_string();
		params.insert(Rc::from("value"), Rc::from(value));
		params.insert(Rc::from("min"), Rc::from($min.to_string()));
		params.insert(Rc::from("max"), Rc::from($max.to_string()));
		params.insert(Rc::from("step"), Rc::from($step.to_string()));

		$mp
			.parser_state
			.instantiate_template($mp.doc_params, "SliderSetting", $mp.layout, id_cell, params)?;

		if $setting.requires_restart() {
			mount_requires_restart($mp.layout, id_cell)?;
		}

		let slider = $mp.parser_state.fetch_component_as::<ComponentSlider>(&id)?;
		slider.on_value_changed(Box::new({
			let tasks = $mp.tasks.clone();
			move |_common, e| {
				tasks.push(Task::UpdateInt($setting, e.value as i32));
				Ok(())
			}
		}));
	};
}

macro_rules! dropdown {
	($mp:expr /* `MacroParams` struct */, $root:expr, $setting:expr, $options:expr) => {
		let id = $mp.idx.to_string();
		$mp.idx += 1;

		let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
		params.insert(Rc::from("id"), Rc::from(id.as_ref()));

		match $setting.get_translation() {
			Ok(translation) => params.insert(Rc::from("translation"), translation.into()),
			Err(raw_text) => params.insert(Rc::from("text"), raw_text.into()),
		};

		if let Some(tooltip) = $setting.get_tooltip() {
			params.insert(Rc::from("tooltip"), Rc::from(tooltip));
		}

		let id_cell = horiz_cell($mp.layout, $root)?;

		$mp
			.parser_state
			.instantiate_template($mp.doc_params, "DropdownButton", $mp.layout, id_cell, params)?;

		if $setting.requires_restart() {
			mount_requires_restart($mp.layout, id_cell)?;
		}

		let setting_str = $setting.as_ref();
		let title = $setting.get_enum_title($mp.config);

		{
			let mut label = $mp
				.parser_state
				.fetch_widget_as::<WidgetLabel>(&$mp.layout.state, &format!("{id}_value"))?;
			label.set_text_simple(&mut $mp.layout.state.globals.get(), title);
		}

		let btn = $mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
		btn.on_click(Rc::new({
			let tasks = $mp.tasks.clone();
			move |_common, e: ButtonClickEvent| {
				tasks.push(Task::OpenContextMenu(
					e.mouse_pos_absolute.unwrap_or_default(),
					$options
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
	};
}

macro_rules! danger_button {
	($mp:expr, $root:expr, $translation:expr, $icon:expr, $task:expr) => {
		let id = $mp.idx.to_string();
		$mp.idx += 1;

		let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
		params.insert(Rc::from("id"), Rc::from(id.as_ref()));
		params.insert(Rc::from("translation"), Rc::from($translation));
		params.insert(Rc::from("icon"), Rc::from($icon));

		$mp
			.parser_state
			.instantiate_template($mp.doc_params, "DangerButton", $mp.layout, $root, params)?;

		let btn = $mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
		btn.on_click(Rc::new({
			let tasks = $mp.tasks.clone();
			move |_common, _e| {
				tasks.push($task);
				Ok(())
			}
		}));
	};
}

macro_rules! autostart_app {
	($mp:expr, $root:expr, $text:expr, $ids:expr) => {
		let id = $mp.idx.to_string();
		$mp.idx += 1;

		let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
		params.insert(Rc::from("id"), Rc::from(id.as_ref()));
		params.insert(Rc::from("text"), Rc::from($text.as_str()));

		$mp
			.parser_state
			.instantiate_template($mp.doc_params, "AutostartApp", $mp.layout, $root, params)?;

		let btn = $mp.parser_state.fetch_component_as::<ComponentButton>(&id)?;
		let id: Rc<str> = Rc::from(id);

		$ids.push(id.clone());

		btn.on_click(Rc::new({
			let tasks = $mp.tasks.clone();
			move |_common, _e| {
				tasks.push(Task::RemoveAutostartApp(id.clone()));
				Ok(())
			}
		}));
	};
}

struct MacroParams<'a> {
	layout: &'a mut Layout,
	parser_state: &'a mut ParserState,
	doc_params: &'a ParseDocumentParams<'a>,
	config: &'a mut GeneralConfig,
	tasks: Tasks<Task>,
	idx: usize,
}

fn doc_params(globals: &'_ WguiGlobals) -> ParseDocumentParams<'_> {
	ParseDocumentParams {
		globals: globals.clone(),
		path: AssetPath::BuiltIn("gui/tab/settings.xml"),
		extra: Default::default(),
	}
}

impl<T> TabSettings<T> {
	fn set_tab(&mut self, frontend: &mut Frontend<T>, data: &mut T, name: TabNameEnum) -> anyhow::Result<()> {
		let root = self.state.get_widget_id("settings_root")?;
		frontend.layout.remove_children(root);
		let globals = frontend.layout.state.globals.clone();

		let mut mp = MacroParams {
			layout: &mut frontend.layout,
			parser_state: &mut self.state,
			doc_params: &doc_params(&globals),
			config: frontend.interface.general_config(data),
			tasks: self.tasks.clone(),
			idx: 9001,
		};

		match name {
			TabNameEnum::LookAndFeel => {
				let c = category!(mp, root, "APP_SETTINGS.LOOK_AND_FEEL", "dashboard/palette.svg")?;
				dropdown!(mp, c, SettingType::Language, wlx_common::locale::Language::VARIANTS);
				checkbox!(mp, c, SettingType::OpaqueBackground);
				checkbox!(mp, c, SettingType::HideUsername);
				checkbox!(mp, c, SettingType::HideGrabHelp);
				slider_f32!(mp, c, SettingType::UiAnimationSpeed, 0.5, 5.0, 0.1); // min, max, step
				slider_f32!(mp, c, SettingType::UiGradientIntensity, 0.0, 1.0, 0.05); // min, max, step
				slider_f32!(mp, c, SettingType::UiRoundMultiplier, 0.5, 5.0, 0.1);
				checkbox!(mp, c, SettingType::SetsOnWatch);
				checkbox!(mp, c, SettingType::UseSkybox);
				slider_f32!(mp, c, SettingType::GridOpacity, 0.0, 1.0, 0.05); // min, max, step
				checkbox!(mp, c, SettingType::UsePassthrough);
				checkbox!(mp, c, SettingType::Clock12h);
			}
			TabNameEnum::Features => {
				let c = category!(mp, root, "APP_SETTINGS.FEATURES", "dashboard/options.svg")?;
				checkbox!(mp, c, SettingType::NotificationsEnabled);
				checkbox!(mp, c, SettingType::NotificationsSoundEnabled);
				checkbox!(mp, c, SettingType::KeyboardSoundEnabled);
				checkbox!(mp, c, SettingType::KeyboardSwipeToTypeEnabled);
				checkbox!(mp, c, SettingType::SpaceDragUnlocked);
				checkbox!(mp, c, SettingType::SpaceRotateUnlocked);
				slider_f32!(mp, c, SettingType::SpaceDragMultiplier, -10.0, 10.0, 0.5);
				checkbox!(mp, c, SettingType::BlockGameInput);
				checkbox!(mp, c, SettingType::BlockGameInputIgnoreWatch);
				checkbox!(mp, c, SettingType::BlockPosesOnKbdInteraction);
			}
			TabNameEnum::Controls => {
				let c = category!(mp, root, "APP_SETTINGS.CONTROLS", "dashboard/controller.svg")?;
				dropdown!(
					mp,
					c,
					SettingType::KeyboardMiddleClick,
					wlx_common::config::AltModifier::VARIANTS
				);
				dropdown!(
					mp,
					c,
					SettingType::HandsfreePointer,
					wlx_common::config::HandsfreePointer::VARIANTS
				);
				checkbox!(mp, c, SettingType::FocusFollowsMouseMode);
				checkbox!(mp, c, SettingType::LeftHandedMouse);
				checkbox!(mp, c, SettingType::AllowSliding);
				checkbox!(mp, c, SettingType::InvertScrollDirectionX);
				checkbox!(mp, c, SettingType::InvertScrollDirectionY);
				slider_f32!(mp, c, SettingType::ScrollSpeed, 0.1, 5.0, 0.1);
				slider_f32!(mp, c, SettingType::LongPressDuration, 0.1, 2.0, 0.1);
				slider_f32!(mp, c, SettingType::PointerLerpFactor, 0.1, 1.0, 0.1);
				slider_f32!(mp, c, SettingType::XrClickSensitivity, 0.1, 1.0, 0.1);
				slider_f32!(mp, c, SettingType::XrClickSensitivityRelease, 0.1, 1.0, 0.1);
				slider_i32!(mp, c, SettingType::ClickFreezeTimeMs, 0, 500, 50);
			}
			TabNameEnum::Misc => {
				let c = category!(mp, root, "APP_SETTINGS.MISC", "dashboard/blocks.svg")?;
				dropdown!(
					mp,
					c,
					SettingType::CaptureMethod,
					wlx_common::config::CaptureMethod::VARIANTS
				);
				checkbox!(mp, c, SettingType::XwaylandByDefault);
				checkbox!(mp, c, SettingType::UprightScreenFix);
				checkbox!(mp, c, SettingType::DoubleCursorFix);
				checkbox!(mp, c, SettingType::ScreenRenderDown);
			}
			TabNameEnum::AutostartApps => {
				self.app_button_ids = vec![];

				if !mp.config.autostart_apps.is_empty() {
					let c = category!(mp, root, "APP_SETTINGS.AUTOSTART_APPS", "dashboard/apps.svg")?;

					for app in &mp.config.autostart_apps {
						autostart_app!(mp, c, app.name, self.app_button_ids);
					}
				}
			}
			TabNameEnum::Troubleshooting => {
				let c = category!(mp, root, "APP_SETTINGS.TROUBLESHOOTING", "dashboard/cpu.svg")?;
				danger_button!(
					mp,
					c,
					"APP_SETTINGS.RESET_PLAYSPACE",
					"dashboard/recenter.svg",
					Task::ResetPlayspace
				);
				danger_button!(
					mp,
					c,
					"APP_SETTINGS.CLEAR_PIPEWIRE_TOKENS",
					"dashboard/display.svg",
					Task::ClearPipewireTokens
				);
				danger_button!(
					mp,
					c,
					"APP_SETTINGS.CLEAR_SAVED_STATE",
					"dashboard/binary.svg",
					Task::ClearSavedState
				);
				danger_button!(
					mp,
					c,
					"APP_SETTINGS.DELETE_ALL_CONFIGS",
					"dashboard/circle.svg",
					Task::DeleteAllConfigs
				);
				danger_button!(
					mp,
					c,
					"APP_SETTINGS.RESTART_SOFTWARE",
					"dashboard/refresh.svg",
					Task::RestartSoftware
				);
			}
		}

		Ok(())
	}

	pub fn new(frontend: &mut Frontend<T>, parent_id: WidgetID, _data: &mut T) -> anyhow::Result<Self> {
		let doc_params = ParseDocumentParams {
			globals: frontend.layout.state.globals.clone(),
			path: AssetPath::BuiltIn("gui/tab/settings.xml"),
			extra: Default::default(),
		};

		let parser_state = wgui::parser::parse_from_assets(&doc_params, &mut frontend.layout, parent_id)?;
		let tasks = Tasks::default();
		let tabs = parser_state.fetch_component_as::<ComponentTabs>("tabs")?;
		tabs.on_select({
			let tasks = tasks.clone();
			Rc::new(move |_common, evt| {
				if let Some(tab) = TabNameEnum::from_string(&evt.name) {
					tasks.push(Task::SetTab(tab));
				}
				Ok(())
			})
		});

		tasks.push(Task::SetTab(TabNameEnum::LookAndFeel));

		Ok(Self {
			app_button_ids: Vec::new(),
			tasks,
			state: parser_state,
			marker: PhantomData,
			context_menu: ContextMenu::default(),
		})
	}
}
