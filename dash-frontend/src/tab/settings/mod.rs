use std::{marker::PhantomData, rc::Rc, str::FromStr};
use strum::EnumProperty;
use wgui::{
	color::WguiColorName,
	components::tabs::ComponentTabs,
	event::StyleSetRequest,
	i18n::Translation,
	layout::{Layout, WidgetID},
	log::LogErr,
	parser::{Fetchable, ParseDocumentParams, ParserState},
	renderer_vk::text::{FontWeight, TextStyle, WguiTextShadow},
	taffy::{self, prelude::length},
	task::Tasks,
	widget::{
		div::WidgetDiv,
		label::{WidgetLabel, WidgetLabelParams},
	},
	windowing::context_menu::{self, Blueprint, ContextMenu, TickResult},
};
use wlx_common::{
	config::{GeneralConfig, SettingType, SettingVar},
	config_io::ConfigRoot,
	dash_interface::{ConfigChangeKind, DashPlayspaceTask, HmdStats, InterfaceFeats},
};

use crate::{
	frontend::{Frontend, FrontendTask, FrontendTasks},
	tab::{Tab, settings::macros::MacroParams},
	views::ViewUpdateParams,
};

mod macros;
mod tab_appearance;
mod tab_autostart_apps;
mod tab_controls;
mod tab_diagnostics;
mod tab_general;
mod tab_hid;
mod tab_misc;
mod tab_overlays;
mod tab_statistics;
mod tab_troubleshooting;

#[derive(Clone)]
pub(crate) enum TabNameEnum {
	Appearance,
	AutostartApps,
	Controls,
	Diagnostics,
	General,
	Hid,
	Misc,
	Overlays,
	Statistics,
	Troubleshooting,
}

impl TabNameEnum {
	fn from_string(s: &str) -> Self {
		match s {
			"general" => TabNameEnum::General,
			"hid" => TabNameEnum::Hid,
			"appearance" => TabNameEnum::Appearance,
			"autostart_apps" => TabNameEnum::AutostartApps,
			"controls" => TabNameEnum::Controls,
			"misc" => TabNameEnum::Misc,
			"overlays" => TabNameEnum::Overlays,
			"statistics" => TabNameEnum::Statistics,
			"diagnostics" => TabNameEnum::Diagnostics,
			"troubleshooting" => TabNameEnum::Troubleshooting,
			_ => unreachable!(), /* wants panic */
		}
	}
}

#[derive(Clone)]
pub(crate) enum Task {
	ClearPipewireTokens,
	ClearSavedState,
	DeleteAllConfigs,
	OpenContextMenu(context_menu::Position, Vec<context_menu::Cell>),
	RemoveAutostartApp(Rc<str>),
	ResetPlayspace,
	ResetPlayspaceCenter,
	SavePlayspaceCenter,
	SetTab(TabNameEnum),
	SettingUpdated(SettingType),
	UpdateVar(SettingType, SettingVar),
}

struct SettingsMountParams<'a> {
	mp: &'a mut MacroParams<'a>,
	frontend_tasks: &'a FrontendTasks,
	id_parent: WidgetID,
	feats: InterfaceFeats,
}

struct SettingUpdatedParams<'a> {
	layout: &'a mut Layout,
	config: &'a GeneralConfig,
	setting_type: SettingType,
}

struct UpdateExtra {
	hmd_stats: HmdStats,
}

trait SettingsTab {
	fn update(&mut self, _par: &mut ViewUpdateParams, _extra: &UpdateExtra) -> anyhow::Result<()> {
		Ok(())
	}

	fn setting_updated(&mut self, _sup: &mut SettingUpdatedParams) -> anyhow::Result<()> {
		Ok(())
	}

	fn context_menu_custom(
		&mut self,
		_action: Rc<str>,
		_config: &mut GeneralConfig,
		_change_kind: &mut Option<ConfigChangeKind>,
		_layout: &mut Layout,
		_state: &mut ParserState,
	) -> anyhow::Result<()> {
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
	frontend_tasks: FrontendTasks,
	feats: InterfaceFeats,
}

impl<T> Tab<T> for TabSettings<T> {
	fn update(&mut self, frontend: &mut Frontend<T>, _time_ms: u32, data: &mut T) -> anyhow::Result<()> {
		if let Some(tab) = &mut self.current_tab {
			let mut config_change_kind = None;

			let hmd_stats = frontend.interface.hmd_stats(data);

			tab.update(
				&mut ViewUpdateParams {
					layout: &mut frontend.layout,
					executor: &frontend.executor,
					general_config: frontend.interface.general_config(data),
					config_change_kind: &mut config_change_kind,
				},
				&UpdateExtra { hmd_stats },
			)?;

			if let Some(kind) = config_change_kind {
				frontend.interface.config_changed(data, kind);
			}
		}

		let mut changed = None;
		for task in self.tasks.drain() {
			match task {
				Task::SetTab(tab) => {
					self.set_tab(frontend, data, tab)?;
				}
				Task::UpdateVar(setting, var) => {
					self.tasks.push(Task::SettingUpdated(setting));
					if let Some(task) = setting.get_frontend_task() {
						frontend.tasks.push(task)
					}
					let config = frontend.interface.general_config(data);
					setting.mut_var(config).set(var)?;
					changed = Some(setting.change_kind());
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
				Task::SavePlayspaceCenter => {
					frontend.interface.playspace_task(data, DashPlayspaceTask::SaveCenter)?;
					return Ok(());
				}
				Task::ResetPlayspaceCenter => {
					frontend
						.interface
						.playspace_task(data, DashPlayspaceTask::ResetCenter)?;
					return Ok(());
				}
				Task::ResetPlayspace => {
					frontend.interface.playspace_task(data, DashPlayspaceTask::Reset)?;
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
						changed = Some(ConfigChangeKind::OverlayConfig);
					}
				}
				Task::SettingUpdated(setting) => {
					if let Some(tab) = &mut self.current_tab {
						tab.setting_updated(&mut SettingUpdatedParams {
							layout: &mut frontend.layout,
							config: frontend.interface.general_config(data),
							setting_type: setting,
						})?;
					}
					match setting {
						SettingType::UiAnimationSpeed | SettingType::UiGradientIntensity | SettingType::UiRoundMultiplier => {
							// todo: currently, wayvr restart is required to apply these changes (WguiTheme is Rc)
						}
						_ => { /* do nothing */ }
					}
				}
			}
		}

		// Dropdown handling
		if let TickResult::Action(name) = self.context_menu.tick(&mut frontend.layout, &mut self.state)? {
			if name.starts_with(';')
				&& let Some(tab) = self.current_tab.as_mut()
			{
				let config = frontend.interface.general_config(data);
				let mut change_kind: Option<ConfigChangeKind> = None;
				tab.context_menu_custom(name, config, &mut change_kind, &mut frontend.layout, &mut self.state)?;
				if let Some(change_kind) = change_kind {
					frontend.interface.config_changed(data, change_kind);
				}
			} else if let (Some(setting), Some(id), Some(value), Some(text), Some(translated)) = {
				let mut s = name.splitn(5, ';');
				(s.next(), s.next(), s.next(), s.next(), s.next())
			} {
				let mut common = frontend.layout.common();
				let mut label = self
					.state
					.fetch_widget_as::<WidgetLabel>(common.state, &format!("{id}_value"))?;

				let translation = Translation {
					text: text.into(),
					translated: translated == "1",
				};

				label.set_text(&mut common, translation);

				let setting = SettingType::from_str(setting).expect("Invalid Enum string");
				let config = frontend.interface.general_config(data);
				setting.set_enum(config, value);
				changed = Some(ConfigChangeKind::OverlayConfig);
			}
		}

		// Notify overlays of the change
		if let Some(changed) = changed {
			frontend.interface.config_changed(data, changed);
		}

		Ok(())
	}
}

trait SettingTypeImpl {
	fn get_translation(&self) -> Result<&'static str, &'static str>;
	fn get_enum_title(&self, config: &mut GeneralConfig) -> Translation;
	fn change_kind(self) -> ConfigChangeKind;
	fn set_enum(self, config: &mut GeneralConfig, value: &str);
	fn get_tooltip(self) -> Option<&'static str>;
	fn requires_restart(self) -> bool;
	fn get_frontend_task(self) -> Option<FrontendTask>;

	fn get_enum_title_inner<E>(value: E) -> Translation
	where
		E: EnumProperty + AsRef<str>;

	fn get_enum_tooltip_inner<E>(value: E) -> Option<Translation>
	where
		E: EnumProperty + AsRef<str>;
}

/// Ok is translation, Err is raw text
/// `match` sorted alphabetically
impl SettingTypeImpl for SettingType {
	fn change_kind(self) -> ConfigChangeKind {
		match self {
			Self::MouseAcceleration | Self::MouseSpeed => ConfigChangeKind::WvrServerConfig,
			Self::UseSkybox | Self::UsePassthrough => ConfigChangeKind::EnvironmentBlend,
			_ => ConfigChangeKind::OverlayConfig,
		}
	}

	fn set_enum(self, config: &mut GeneralConfig, value: &str) {
		match self {
			Self::CaptureMethod => {
				config.capture_method = wlx_common::config::CaptureMethod::from_str(value).expect("Invalid enum value!")
			}
			Self::DefaultPositioning => {
				config.default_positioning =
					wlx_common::config::DefaultPositioning::from_str(value).expect("Invalid enum value!")
			}
			Self::InputCaptureMethod => {
				config.wvr_input_capture = wlx_common::config::InputCaptureMethod::from_str(value).expect("Invalid enum value!")
			}
			Self::InputEmulationMethod => {
				config.input_emulation_method =
					wlx_common::config::InputEmulationMethod::from_str(value).expect("Invalid enum value!")
			}
			Self::KeyboardMiddleClick => {
				config.keyboard_middle_click_mode =
					wlx_common::config::AltModifier::from_str(value).expect("Invalid enum value!")
			}
			Self::HandsfreeAltTab => {
				config.handsfree_alt_tab = wlx_common::config::HandsfreeAltTab::from_str(value).expect("Invalid enum value!")
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

	fn get_enum_tooltip_inner<E>(value: E) -> Option<Translation>
	where
		E: EnumProperty + AsRef<str>,
	{
		value.get_str("Tooltip").map(Translation::from_translation_key)
	}

	/// `match` sorted alphabetically
	fn get_tooltip(self) -> Option<&'static str> {
		match self {
			Self::BlockGameInputIgnoreWatch => Some("APP_SETTINGS.BLOCK_GAME_INPUT_IGNORE_WATCH_HELP"),
			Self::BlockPosesOnKbdInteraction => Some("APP_SETTINGS.BLOCK_POSES_ON_KBD_INTERACTION_HELP"),
			Self::CaptureMethod => Some("APP_SETTINGS.CAPTURE_METHOD_HELP"),
			Self::DefaultCurvature => Some("APP_SETTINGS.OVERLAY_DEFAULTS_HELP"),
			Self::DefaultOpacity => Some("APP_SETTINGS.OVERLAY_DEFAULTS_HELP"),
			Self::DefaultOverlayScale => Some("APP_SETTINGS.DEFAULT_OVERLAY_SCALE_HELP"),
			Self::DefaultPositioning => Some("APP_SETTINGS.OVERLAY_DEFAULTS_HELP"),
			Self::DoubleCursorFix => Some("APP_SETTINGS.DOUBLE_CURSOR_FIX_HELP"),
			Self::GridOpacity => Some("APP_SETTINGS.GRID_OPACITY_HELP"),
			Self::HandsfreeAltTab => Some("APP_SETTINGS.HANDSFREE_ALT_TAB_HELP"),
			Self::HandsfreePointer => Some("APP_SETTINGS.HANDSFREE_POINTER_HELP"),
			Self::InputCaptureMethod => Some("APP_SETTINGS.INPUT_CAPTURE_METHOD_HELP"),
			Self::InputEmulationMethod => Some("APP_SETTINGS.INPUT_EMULATION_METHOD_HELP"),
			Self::KeyboardMiddleClick => Some("APP_SETTINGS.KEYBOARD_MIDDLE_CLICK_HELP"),
			Self::LeftHandedMouse => Some("APP_SETTINGS.LEFT_HANDED_MOUSE_HELP"),
			Self::MiddleGrabsSingle => Some("APP_SETTINGS.MIDDLE_GRABS_SINGLE_HELP"),
			Self::MouseAcceleration => Some("APP_SETTINGS.POINTER_ACCELERATION_HELP"),
			Self::ScreenRenderDown => Some("APP_SETTINGS.SCREEN_RENDER_DOWN_HELP"),
			Self::SnapAngleDeg => Some("APP_SETTINGS.SNAP_ANGLE_DEG_HELP"),
			Self::SpaceGravityDamping => Some("APP_SETTINGS.SPACE_GRAVITY_DAMPING_HELP"),
			Self::SpaceGravityFlingStrength => Some("APP_SETTINGS.SPACE_GRAVITY_FLING_STRENGTH_HELP"),
			Self::SpaceGravityFloorHeight => Some("APP_SETTINGS.SPACE_GRAVITY_FLOOR_HEIGHT_HELP"),
			Self::SpaceGravityGravity => Some("APP_SETTINGS.SPACE_GRAVITY_GRAVITY_HELP"),
			Self::SpaceGravityGroundFriction => Some("APP_SETTINGS.SPACE_GRAVITY_GROUND_FRICTION_HELP"),
			Self::UprightScreenFix => Some("APP_SETTINGS.UPRIGHT_SCREEN_FIX_HELP"),
			Self::UsePassthrough => Some("APP_SETTINGS.USE_PASSTHROUGH_HELP"),
			Self::UseSkybox => Some("APP_SETTINGS.USE_SKYBOX_HELP"),
			Self::WatchViewAngleMax => Some("APP_SETTINGS.WATCH_VIEW_ANGLE_HELP"),
			Self::WatchViewAngleMin => Some("APP_SETTINGS.WATCH_VIEW_ANGLE_HELP"),
			Self::BlockGameInput => Some("APP_SETTINGS.BLOCK_GAME_INPUT_HELP"),
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
				| Self::Language
				| Self::CaptureMethod
				| Self::InputEmulationMethod
		)
	}

	fn get_frontend_task(self) -> Option<FrontendTask> {
		match self {
			Self::Clock12h => Some(FrontendTask::RefreshClock),
			Self::OpaqueBackground => Some(FrontendTask::RefreshBackground),
			_ => None,
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

	fn get_enum_title(&self, config: &mut GeneralConfig) -> Translation {
		match self {
			Self::CaptureMethod => Self::get_enum_title_inner(config.capture_method),
			Self::DefaultPositioning => Self::get_enum_title_inner(config.default_positioning),
			Self::InputCaptureMethod => Self::get_enum_title_inner(config.wvr_input_capture),
			Self::InputEmulationMethod => Self::get_enum_title_inner(config.input_emulation_method),
			Self::KeyboardMiddleClick => Self::get_enum_title_inner(config.keyboard_middle_click_mode),
			Self::HandsfreePointer => Self::get_enum_title_inner(config.handsfree_pointer),
			Self::HandsfreeAltTab => Self::get_enum_title_inner(config.handsfree_alt_tab),
			Self::Language => match &config.language {
				Some(lang) => Self::get_enum_title_inner(*lang),
				None => Translation::from_translation_key("APP_SETTINGS.OPTION.AUTO"),
			},
			_ => panic!("Requested enum for non-enum SettingType"),
		}
	}
	fn get_translation(&self) -> Result<&'static str, &'static str> {
		match self {
			Self::AllowSliding => Ok("APP_SETTINGS.ALLOW_SLIDING"),
			Self::BlockGameInput => Ok("APP_SETTINGS.BLOCK_GAME_INPUT"),
			Self::BlockGameInputIgnoreWatch => Ok("APP_SETTINGS.BLOCK_GAME_INPUT_IGNORE_WATCH"),
			Self::BlockPosesOnKbdInteraction => Ok("APP_SETTINGS.BLOCK_POSES_ON_KBD_INTERACTION"),
			Self::CaptureMethod => Ok("APP_SETTINGS.CAPTURE_METHOD"),
			Self::ClickFreezeTimeMs => Ok("APP_SETTINGS.CLICK_FREEZE_TIME_MS"),
			Self::Clock12h => Ok("APP_SETTINGS.CLOCK_12H"),
			Self::DefaultCurvature => Ok("APP_SETTINGS.DEFAULT_CURVATURE"),
			Self::DefaultOpacity => Ok("APP_SETTINGS.DEFAULT_OPACITY"),
			Self::DefaultOverlayScale => Ok("APP_SETTINGS.DEFAULT_OVERLAY_SCALE"),
			Self::DefaultPositioning => Ok("APP_SETTINGS.DEFAULT_POSITIONING"),
			Self::DoubleCursorFix => Ok("APP_SETTINGS.DOUBLE_CURSOR_FIX"),
			Self::EnableWatch => Ok("APP_SETTINGS.ENABLE_WATCH"),
			Self::FocusFollowsMouseMode => Ok("APP_SETTINGS.FOCUS_FOLLOWS_MOUSE_MODE"),
			Self::GridOpacity => Ok("APP_SETTINGS.GRID_OPACITY"),
			Self::HandsfreeAltTab => Ok("APP_SETTINGS.HANDSFREE_ALT_TAB"),
			Self::HandsfreePointer => Ok("APP_SETTINGS.HANDSFREE_POINTER"),
			Self::ShowGrabHelp => Ok("APP_SETTINGS.SHOW_GRAB_HELP"),
			Self::InputCaptureMethod => Ok("APP_SETTINGS.INPUT_CAPTURE_METHOD"),
			Self::InputEmulationMethod => Ok("APP_SETTINGS.INPUT_EMULATION_METHOD"),
			Self::InvertScrollDirectionX => Ok("APP_SETTINGS.INVERT_SCROLL_DIRECTION_X"),
			Self::InvertScrollDirectionY => Ok("APP_SETTINGS.INVERT_SCROLL_DIRECTION_Y"),
			Self::KeyboardMiddleClick => Ok("APP_SETTINGS.KEYBOARD_MIDDLE_CLICK"),
			Self::KeyboardSoundEnabled => Ok("APP_SETTINGS.KEYBOARD_SOUND_ENABLED"),
			Self::Language => Ok("APP_SETTINGS.LANGUAGE"),
			Self::LeftHandedMouse => Ok("APP_SETTINGS.LEFT_HANDED_MOUSE"),
			Self::LongPressDuration => Ok("APP_SETTINGS.LONG_PRESS_DURATION"),
			Self::MiddleGrabsSingle => Ok("APP_SETTINGS.MIDDLE_GRABS_SINGLE"),
			Self::MouseAcceleration => Ok("APP_SETTINGS.POINTER_ACCELERATION"),
			Self::MouseSpeed => Ok("APP_SETTINGS.POINTER_SPEED"),
			Self::NotificationsEnabled => Ok("APP_SETTINGS.NOTIFICATIONS_ENABLED"),
			Self::NotificationsSoundEnabled => Ok("APP_SETTINGS.NOTIFICATIONS_SOUND_ENABLED"),
			Self::OpaqueBackground => Ok("APP_SETTINGS.OPAQUE_BACKGROUND"),
			Self::PointerLerpFactor => Ok("APP_SETTINGS.POINTER_LERP_FACTOR"),
			Self::ScreenRenderDown => Ok("APP_SETTINGS.SCREEN_RENDER_DOWN"),
			Self::ScrollSpeed => Ok("APP_SETTINGS.SCROLL_SPEED"),
			Self::SetsOnWatch => Ok("APP_SETTINGS.SETS_ON_WATCH"),
			Self::ShowUsername => Ok("APP_SETTINGS.SHOW_USERNAME"),
			Self::SnapAngleDeg => Ok("APP_SETTINGS.SNAP_ANGLE_DEG"),
			Self::SpaceDragAffectsWorld => Ok("APP_SETTINGS.SPACE_DRAG_AFFECTS_WORLD"),
			Self::SpaceDragMultiplier => Ok("APP_SETTINGS.SPACE_DRAG_MULTIPLIER"),
			Self::SpaceDragUnlocked => Ok("APP_SETTINGS.SPACE_DRAG_UNLOCKED"),
			Self::SpaceGravityDamping => Ok("APP_SETTINGS.SPACE_GRAVITY_DAMPING"),
			Self::SpaceGravityEnabled => Ok("APP_SETTINGS.ENABLED"),
			Self::SpaceGravityFlingStrength => Ok("APP_SETTINGS.SPACE_GRAVITY_FLING_STRENGTH"),
			Self::SpaceGravityFloorHeight => Ok("APP_SETTINGS.SPACE_GRAVITY_FLOOR_HEIGHT"),
			Self::SpaceGravityGravity => Ok("APP_SETTINGS.SPACE_GRAVITY_GRAVITY"),
			Self::SpaceGravityGroundFriction => Ok("APP_SETTINGS.SPACE_GRAVITY_GROUND_FRICTION"),
			Self::SpaceRotateUnlocked => Ok("APP_SETTINGS.SPACE_ROTATE_UNLOCKED"),
			Self::UiAnimationSpeed => Ok("APP_SETTINGS.ANIMATION_SPEED"),
			Self::UiGradientIntensity => Ok("APP_SETTINGS.UI_GRADIENT_INTENSITY"),
			Self::UiRoundMultiplier => Ok("APP_SETTINGS.ROUND_MULTIPLIER"),
			Self::UprightScreenFix => Ok("APP_SETTINGS.UPRIGHT_SCREEN_FIX"),
			Self::UsePassthrough => Ok("APP_SETTINGS.USE_PASSTHROUGH"),
			Self::UseSkybox => Ok("APP_SETTINGS.USE_SKYBOX"),
			Self::WatchViewAngleMax => Ok("APP_SETTINGS.WATCH_VIEW_ANGLE"),
			Self::WatchViewAngleMin => Ok("APP_SETTINGS.WATCH_VIEW_ANGLE"),
			Self::XwaylandByDefault => Ok("APP_SETTINGS.XWAYLAND_BY_DEFAULT"),
		}
	}
}

// creates a simple div with horizontal, centered flow
pub fn horiz_cell(layout: &mut Layout, parent: WidgetID) -> anyhow::Result<WidgetID> {
	let (pair, _) = layout.add_child(
		parent,
		WidgetDiv::create(),
		taffy::Style {
			flex_direction: taffy::FlexDirection::Row,
			align_items: Some(taffy::AlignItems::CENTER),
			gap: length(8.0_f32),
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
				color: Some(WguiColorName::Danger.into()),
				weight: Some(FontWeight::Bold),
				size: Some(10.0),
				shadow: Some(WguiTextShadow {
					x: 2.0,
					y: 2.0,
					..Default::default()
				}),
				..Default::default()
			},
			..Default::default()
		},
	);

	layout.add_child(parent, label, Default::default())?;
	Ok(())
}

const DOC_PARAMS: ParseDocumentParams = ParseDocumentParams::builtin("gui/tab/settings.xml");

impl<T> TabSettings<T> {
	fn set_tab(&mut self, frontend: &mut Frontend<T>, data: &mut T, name: TabNameEnum) -> anyhow::Result<()> {
		let root = self.state.get_widget_id("settings_root")?;
		frontend.layout.remove_children(root);
		self.current_tab = None;

		let vulkan_devices = if matches!(name, TabNameEnum::General) {
			Some(frontend.interface.enumerate_vulkan_devices(data)?)
		} else {
			None
		};

		let mut mp = MacroParams {
			layout: &mut frontend.layout,
			parser_state: &mut self.state,
			doc_params: &DOC_PARAMS,
			config: frontend.interface.general_config(data),
			tasks: self.tasks.clone(),
			id_counter: 9001,
			feats: self.feats,
		};

		let settings_mount_params = SettingsMountParams {
			mp: &mut mp,
			id_parent: root,
			frontend_tasks: &self.frontend_tasks,
			feats: self.feats,
		};

		match name {
			TabNameEnum::General => {
				self.current_tab = Some(Box::new(tab_general::State::mount(
					settings_mount_params,
					vulkan_devices.unwrap(), /* safe */
				)?));
			}
			TabNameEnum::Appearance => {
				self.current_tab = Some(Box::new(tab_appearance::State::mount(settings_mount_params)?));
			}
			TabNameEnum::Hid => {
				self.current_tab = Some(Box::new(tab_hid::State::mount(settings_mount_params)?));
			}
			TabNameEnum::Overlays => {
				self.current_tab = Some(Box::new(tab_overlays::State::mount(settings_mount_params)?));
			}
			TabNameEnum::Controls => {
				self.current_tab = Some(Box::new(tab_controls::State::mount(settings_mount_params)?));
			}
			TabNameEnum::Misc => {
				self.current_tab = Some(Box::new(tab_misc::State::mount(settings_mount_params)?));
			}
			TabNameEnum::AutostartApps => {
				self.current_tab = Some(Box::new(tab_autostart_apps::State::mount(
					settings_mount_params,
					&mut self.app_button_ids,
				)?));
			}
			TabNameEnum::Troubleshooting => {
				self.current_tab = Some(Box::new(tab_troubleshooting::State::mount(settings_mount_params)?));
			}
			TabNameEnum::Statistics => {
				self.current_tab = Some(Box::new(tab_statistics::State::mount(settings_mount_params)?));
			}
			TabNameEnum::Diagnostics => {
				self.current_tab = Some(Box::new(tab_diagnostics::State::mount(settings_mount_params)?));
			}
		}

		Ok(())
	}

	pub fn new(frontend: &mut Frontend<T>, parent_id: WidgetID, data: &mut T) -> anyhow::Result<Self> {
		let doc_params = ParseDocumentParams::builtin("gui/tab/settings.xml");

		let parser_state = wgui::parser::parse_from_assets(&doc_params, &mut frontend.layout, parent_id)?;
		let tasks = Tasks::default();
		let tabs = parser_state.fetch_component_as::<ComponentTabs>("tabs")?;

		{
			let mut label_version = parser_state.fetch_widget_as::<WidgetLabel>(&frontend.layout.state, "label_version")?;

			label_version.set_text_simple(
				&mut frontend.globals.get(),
				Translation::from_raw_text_string(frontend.interface.get_version_string(data)),
			);
		}
		if !frontend.interface.get_feats(data).xr_backend.is_open_xr() {
			let skybox_btn = tabs.get_tab_button("skybox").unwrap();
			frontend
				.layout
				.common()
				.alterables
				.set_style(skybox_btn.get_rect(), StyleSetRequest::Display(taffy::Display::None));
		}

		tabs.on_select({
			let tasks = tasks.clone();
			Rc::new(move |_common, evt| {
				tasks.push(Task::SetTab(TabNameEnum::from_string(&evt.name)));
				Ok(())
			})
		});

		tasks.push(Task::SetTab(TabNameEnum::General));

		let feats = frontend.interface.get_feats(data);

		Ok(Self {
			app_button_ids: Vec::new(),
			tasks,
			state: parser_state,
			marker: PhantomData,
			context_menu: ContextMenu::default(),
			current_tab: None,
			frontend_tasks: frontend.tasks.clone(),
			feats,
		})
	}
}
