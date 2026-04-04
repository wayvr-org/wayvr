use glam::Vec2;
use std::{marker::PhantomData, rc::Rc, str::FromStr};
use strum::{AsRefStr, EnumProperty, EnumString};
use wgui::{
	assets::AssetPath,
	components::tabs::ComponentTabs,
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
use wlx_common::{
	async_executor::AsyncExecutor, config::GeneralConfig, config_io::ConfigRoot, dash_interface::RecenterMode,
};

use crate::{
	frontend::{Frontend, FrontendTask},
	tab::{Tab, TabType, settings::macros::MacroParams},
};

mod macros;
mod tab_autostart_apps;
mod tab_controls;
mod tab_features;
mod tab_look_and_feel;
mod tab_misc;
mod tab_skybox;
mod tab_troubleshooting;

#[derive(Clone)]
enum TabNameEnum {
	LookAndFeel,
	Features,
	Controls,
	Misc,
	AutostartApps,
	Troubleshooting,
	Skybox,
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
			"skybox" => Some(TabNameEnum::Skybox),
			_ => None,
		}
	}
}

#[derive(Clone)]
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

struct SettingsMountParams<'a> {
	mp: &'a mut MacroParams<'a>,
	globals: &'a WguiGlobals,
	parent_id: WidgetID,
}

struct SettingsUpdateParams<'a> {
	layout: &'a mut Layout,
	executor: &'a AsyncExecutor,
}

trait SettingsTab {
	fn update(&mut self, _par: SettingsUpdateParams) -> anyhow::Result<()> {
		Ok(())
	}
}

pub struct TabSettings<T> {
	pub state: ParserState,

	app_button_ids: Vec<Rc<str>>,
	context_menu: ContextMenu,

	current_tab: Option<Box<dyn SettingsTab>>,

	tasks: Tasks<Task>,
	marker: PhantomData<T>,
}

impl<T> Tab<T> for TabSettings<T> {
	fn get_type(&self) -> TabType {
		TabType::Settings
	}

	fn update(&mut self, frontend: &mut Frontend<T>, _time_ms: u32, data: &mut T) -> anyhow::Result<()> {
		if let Some(tab) = &mut self.current_tab {
			tab.update(SettingsUpdateParams {
				layout: &mut frontend.layout,
				executor: &frontend.executor,
			})?;
		}

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
		self.current_tab = None;

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
				tab_look_and_feel::mount(&mut mp, root)?;
			}
			TabNameEnum::Features => {
				tab_features::mount(&mut mp, root)?;
			}
			TabNameEnum::Controls => {
				tab_controls::mount(&mut mp, root)?;
			}
			TabNameEnum::Misc => {
				tab_misc::mount(&mut mp, root)?;
			}
			TabNameEnum::AutostartApps => {
				tab_autostart_apps::mount(&mut mp, root, &mut self.app_button_ids)?;
			}
			TabNameEnum::Troubleshooting => {
				tab_troubleshooting::mount(&mut mp, root)?;
			}
			TabNameEnum::Skybox => {
				self.current_tab = Some(Box::new(tab_skybox::State::mount(SettingsMountParams {
					mp: &mut mp,
					globals: &globals,
					parent_id: root,
				})?));
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
			current_tab: None,
		})
	}
}
