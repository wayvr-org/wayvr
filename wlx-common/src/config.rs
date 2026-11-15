use chrono::Offset;
use derive_more::{From, TryInto};
use idmap::IdMap;
use serde::{Deserialize, Deserializer, Serialize};
use std::{collections::HashMap, rc::Rc, sync::Arc};
use strum::{AsRefStr, EnumProperty, EnumString, VariantArray, VariantNames};
use wayvr_ipc::packet_client::WvrProcessLaunchParams;
use wgui::drawing::{self, HsvColor};

use crate::{
	astr_containers::{AStrMap, AStrSet},
	locale::{self},
	overlays::{BackendAttribValue, ToastDisplayMethod, ToastTopic},
	windowing::OverlayWindowState,
};

pub const CONFIG_VERSION: u32 = 1;

pub type PwTokenMap = AStrMap<String>;
pub type SerializedWindowStates = HashMap<Arc<str>, OverlayWindowState>;

#[derive(Default, Clone, Copy, Serialize, Deserialize, AsRefStr, EnumString, EnumProperty, VariantArray)]
pub enum CaptureMethod {
	#[default]
	#[serde(alias = "auto")]
	#[strum(props(Translation = "APP_SETTINGS.OPTION.AUTO", Tooltip = "APP_SETTINGS.OPTION.AUTO_HELP"))]
	Auto,

	#[serde(alias = "pipewire")]
	#[strum(props(Text = "PipeWire GPU", Tooltip = "APP_SETTINGS.OPTION.PIPEWIRE_HELP"))]
	PipeWire,

	#[strum(props(
		Text = "ScreenCopy GPU",
		Tooltip = "APP_SETTINGS.OPTION.SCREENCOPY_GPU_HELP",
		Desktop = "Wayland"
	))]
	ScreenCopyGpu,

	#[serde(alias = "pw-fallback")]
	#[strum(props(Text = "PipeWire CPU", Tooltip = "APP_SETTINGS.OPTION.PW_FALLBACK_HELP"))]
	PipeWireCpu,

	#[serde(alias = "screencopy")]
	#[strum(props(
		Text = "ScreenCopy CPU",
		Tooltip = "APP_SETTINGS.OPTION.SCREENCOPY_HELP",
		Desktop = "Wayland"
	))]
	ScreenCopyCpu,
}

#[derive(Default, Clone, Copy, Serialize, Deserialize, AsRefStr, EnumString, EnumProperty, VariantArray)]
pub enum InputEmulationMethod {
	#[default]
	#[strum(props(
		Translation = "APP_SETTINGS.OPTION.UINPUT",
		Tooltip = "APP_SETTINGS.OPTION.UINPUT_HELP"
	))]
	Uinput,

	#[strum(props(
		Translation = "APP_SETTINGS.OPTION.WL_VIRTUAL",
		Tooltip = "APP_SETTINGS.OPTION.WL_VIRTUAL_HELP",
		Desktop = "Wayland"
	))]
	WlVirtual,

	#[strum(props(
		Translation = "APP_SETTINGS.OPTION.NONE",
		Tooltip = "APP_SETTINGS.OPTION.NONE_INPUT_HELP"
	))]
	None,
}

#[derive(Default, Clone, Copy, Serialize, Deserialize, AsRefStr, EnumString, EnumProperty, VariantArray)]
pub enum InputCaptureMethod {
	#[strum(props(Tooltip = "APP_SETTINGS.OPTION.EVDEV_HELP"))]
	Evdev,

	#[default]
	#[strum(props(
		Translation = "APP_SETTINGS.OPTION.NONE",
		Tooltip = "APP_SETTINGS.OPTION.NONE_INPUT_CAPTURE_HELP"
	))]
	None,
}

#[derive(Default, Clone, Copy, Serialize, Deserialize, AsRefStr, EnumString, EnumProperty, VariantArray)]
pub enum DefaultPositioning {
	#[default]
	#[strum(props(Translation = "DEFINITIONS.FLOATING", Tooltip = "APP_SETTINGS.OPTION.FLOATING_HELP",))]
	Floating,

	#[strum(props(Translation = "DEFINITIONS.ANCHORED", Tooltip = "APP_SETTINGS.OPTION.ANCHORED_HELP"))]
	Anchored,

	#[strum(props(Translation = "DEFINITIONS.STATIC", Tooltip = "APP_SETTINGS.OPTION.STATIC_HELP"))]
	Static,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, AsRefStr, EnumString, EnumProperty, VariantArray)]
pub enum AltModifier {
	#[default]
	#[strum(props(Translation = "APP_SETTINGS.OPTION.NONE"))]
	None,
	Shift,
	Ctrl,
	Alt,
	Super,
	#[serde(alias = "Meta")]
	AltGr,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, AsRefStr, EnumString, EnumProperty, VariantArray)]
pub enum HandsfreePointer {
	#[strum(props(Translation = "APP_SETTINGS.OPTION.NONE"))]
	None,
	#[strum(props(Translation = "APP_SETTINGS.OPTION.HMD_PINCH", Backend = "OpenXR"))]
	#[default]
	Hmd,
	#[strum(props(Translation = "APP_SETTINGS.OPTION.HMD_ONLY"))]
	HmdOnly,
	#[strum(props(Translation = "APP_SETTINGS.OPTION.EYE_PINCH", Backend = "OpenXR"))]
	EyeTracking,
	#[strum(props(Translation = "APP_SETTINGS.OPTION.EYE_ONLY"))]
	EyeTrackingOnly,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, AsRefStr, EnumString, EnumProperty, VariantArray)]
pub enum HandsfreeAltTab {
	#[strum(props(Translation = "APP_SETTINGS.OPTION.HMD_ONLY"))]
	#[default]
	Hmd,
	#[strum(props(Translation = "APP_SETTINGS.OPTION.EYE_ONLY"))]
	EyeTracking,
}

impl From<HandsfreeAltTab> for HandsfreePointer {
	fn from(value: HandsfreeAltTab) -> Self {
		match value {
			HandsfreeAltTab::Hmd => HandsfreePointer::HmdOnly,
			HandsfreeAltTab::EyeTracking => HandsfreePointer::EyeTrackingOnly,
		}
	}
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ChromaKeyParams {
	pub hsv_min: [f32; 3],
	pub hsv_max: [f32; 3],
	pub curve: f32,
	pub despill: f32,
}

fn hsv_green() -> [f32; 3] {
	[2.0 / 6.0, 1.0, 1.0]
}

impl Default for ChromaKeyParams {
	fn default() -> Self {
		Self {
			hsv_min: hsv_green(),
			hsv_max: hsv_green(),
			curve: 0.0, // Monado will ignore chroma keying completely if this value is zero
			despill: 0.0,
		}
	}
}

impl ChromaKeyParams {
	pub fn update_hsv_range_from_rgb(&mut self, rgb_color: drawing::Color, range_h: f32, range_s: f32, range_v: f32) {
		let hsv = HsvColor::from(rgb_color);
		self.hsv_min[0] = hsv.h - range_h / 2.0;
		self.hsv_min[1] = hsv.s - range_s / 2.0;
		self.hsv_min[2] = hsv.v - range_v / 2.0;
		self.hsv_max[0] = hsv.h + range_h / 2.0;
		self.hsv_max[1] = hsv.s + range_s / 2.0;
		self.hsv_max[2] = hsv.v + range_v / 2.0;
	}

	// Inverse of `update_hsv_range_from_rgb` function
	pub fn get_rgb_and_hsv_ranges(&self) -> (drawing::Color, f32, f32, f32) {
		(
			HsvColor::new(
				(self.hsv_min[0] + self.hsv_max[0]) / 2.0,
				(self.hsv_min[1] + self.hsv_max[1]) / 2.0,
				(self.hsv_min[2] + self.hsv_max[2]) / 2.0,
				1.0,
			)
			.to_rgb(),
			self.hsv_max[0] - self.hsv_min[0],
			self.hsv_max[1] - self.hsv_min[1],
			self.hsv_max[2] - self.hsv_min[2],
		)
	}
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SerializedWindowSet {
	pub name: Arc<str>,

	#[serde(default)]
	pub overlays: SerializedWindowStates,

	#[serde(default)]
	pub hidden_overlays: SerializedWindowStates,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, Eq, PartialEq, EnumString, VariantNames, AsRefStr)]
pub enum AppPosMode {
	Floating,
	Anchored,
	Static,
}
#[derive(Debug, Serialize, Deserialize, Clone, Copy, Eq, PartialEq, EnumString, VariantNames, AsRefStr)]
pub enum AppOrientationMode {
	Wide,
	SemiWide,
	Square,
	SemiTall,
	Tall,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, Eq, PartialEq, EnumString, VariantNames, AsRefStr)]
pub enum AppCompositorMode {
	Cage,
	Native,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, Eq, PartialEq, EnumString, VariantNames, AsRefStr)]
pub enum AppResMode {
	Res1440,
	Res1080,
	Res720,
	Res480,
}

#[derive(Eq, PartialEq, Debug, Clone, Serialize, Deserialize)]
pub struct PinnedApp {
	pub app_id: Rc<str>, // desktop entry app id, for example "libreoffice-draw"
	pub compositor_mode: AppCompositorMode,
	pub pos_mode: AppPosMode,
	pub orientation_mode: AppOrientationMode,
	pub res_mode: AppResMode,
}

pub const fn def_pw_tokens() -> PwTokenMap {
	AStrMap::new()
}

const fn def_mouse_move_interval_ms() -> i32 {
	10 // 100fps
}

const fn def_click_freeze_time_ms() -> i32 {
	300
}

const fn def_true() -> bool {
	true
}

const fn def_one() -> f32 {
	1.0
}

const fn def_half() -> f32 {
	0.5
}

const fn def_point7() -> f32 {
	0.7
}

const fn def_point3() -> f32 {
	0.3
}

const fn def_point15() -> f32 {
	0.15
}

const fn def_osc_port() -> u16 {
	9000
}

fn deserialize_or_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
	D: Deserializer<'de>,
	T: Deserialize<'de> + Default,
{
	let value = serde_json::Value::deserialize(deserializer)?;
	Ok(T::deserialize(value).unwrap_or_default())
}

fn def_timezones() -> Vec<String> {
	const EMEA: i32 = -60 * 60; // UTC-1
	const APAC: i32 = 5 * 60 * 60; // UTC+5

	let offset = chrono::Local::now().offset().fix();
	match offset.local_minus_utc() {
		i32::MIN..EMEA => vec!["Europe/Paris".into(), "Asia/Tokyo".into()],
		EMEA..APAC => vec!["America/New_York".into(), "Asia/Tokyo".into()],
		APAC..=i32::MAX => vec!["Europe/Paris".into(), "America/New_York".into()],
	}
}

fn def_default() -> Arc<str> {
	"Default".into()
}

fn def_theme_path() -> Arc<str> {
	"theme".into()
}

const fn def_max_height() -> u16 {
	1440
}

#[derive(Deserialize, Serialize)]
pub struct GeneralConfig {
	#[serde(default = "def_theme_path")]
	pub theme_path: Arc<str>,

	pub language: Option<locale::Language>, // auto-detected at runtime if unset

	#[serde(default = "def_one")]
	#[serde(alias = "ui_animation_speed", alias = "animation_speed" /* old name */)]
	pub ui_animation_speed: f32,

	#[serde(default = "def_one")]
	#[serde(alias = "ui_round_multiplier", alias = "round_multiplier" /* old name */)]
	pub ui_round_multiplier: f32,

	#[serde(default = "def_point3")]
	pub ui_gradient_intensity: f32,

	pub default_keymap: Option<String>,

	#[serde(default)]
	pub keyboard_layouts: Vec<Arc<str>>,

	#[serde(default)]
	pub attribs: AStrMap<Vec<BackendAttribValue>>,

	#[serde(default = "def_click_freeze_time_ms")]
	pub click_freeze_time_ms: i32,

	#[serde(default)]
	pub invert_scroll_direction_x: bool,

	#[serde(default)]
	pub invert_scroll_direction_y: bool,

	#[serde(default = "def_one")]
	pub scroll_speed: f32,

	#[serde(default = "def_one")]
	pub long_press_duration: f32,

	#[serde(default = "def_mouse_move_interval_ms")]
	pub mouse_move_interval_ms: i32,

	#[serde(default = "def_true")]
	pub notifications_enabled: bool,

	#[serde(default = "def_true")]
	pub notifications_sound_enabled: bool,

	#[serde(default)]
	pub notification_topics: IdMap<ToastTopic, ToastDisplayMethod>,

	#[serde(default = "def_true")]
	pub keyboard_sound_enabled: bool,

	#[serde(default = "def_one")]
	pub keyboard_scale: f32,

	#[serde(default = "def_one")]
	pub default_overlay_scale: f32,

	#[serde(default = "def_half")]
	pub watch_view_angle_min: f32,

	#[serde(default = "def_point7")]
	pub watch_view_angle_max: f32,

	#[serde(default = "def_osc_port")]
	pub osc_out_port: u16,

	#[serde(default)]
	pub upright_screen_fix: bool,

	#[serde(default)]
	pub double_cursor_fix: bool,

	#[serde(default = "def_true")]
	pub enable_watch: bool,

	#[serde(default = "def_true")]
	pub sets_on_watch: bool,

	#[serde(default = "def_true")]
	pub show_grab_help: bool,

	#[serde(default)]
	pub custom_panels: AStrSet,

	#[serde(default)]
	pub capture_method: CaptureMethod,

	#[serde(default)]
	pub input_emulation_method: InputEmulationMethod,

	#[serde(default = "def_true")]
	pub allow_sliding: bool,

	#[serde(default)]
	pub focus_follows_mouse_mode: bool,

	#[serde(default)]
	pub left_handed_mouse: bool,

	#[serde(default)]
	pub block_game_input: bool,

	#[serde(default = "def_true")]
	pub block_game_input_ignore_watch: bool,

	#[serde(default = "def_true")]
	pub block_poses_on_kbd_interaction: bool,

	#[serde(default = "def_one")]
	pub space_drag_multiplier: f32,

	#[serde(default)]
	pub skybox_texture: Arc<str>,

	#[serde(default = "def_true")]
	pub use_skybox: bool,

	#[serde(default = "def_true")]
	pub use_passthrough: bool,

	#[serde(default = "def_max_height")]
	pub screen_max_height: u16,

	#[serde(default)]
	pub screen_render_down: bool,

	#[serde(default = "def_point3")]
	pub pointer_lerp_factor: f32,

	#[serde(default = "def_true")]
	pub space_drag_unlocked: bool,

	#[serde(default)]
	pub space_drag_affects_world: bool,

	#[serde(default)]
	pub space_rotate_unlocked: bool,

	#[serde(default)]
	pub space_gravity_enabled: bool,

	#[serde(default = "def_one")]
	pub space_gravity_gravity: f32,

	#[serde(default = "def_one")]
	pub space_gravity_damping: f32,

	#[serde(default = "def_one")]
	pub space_gravity_fling_strength: f32,

	#[serde(default = "def_one")]
	pub space_gravity_ground_friction: f32,

	#[serde(default)]
	pub space_gravity_floor_height: f32,

	#[serde(default)]
	pub alt_click_down: Vec<String>,

	#[serde(default)]
	pub alt_click_up: Vec<String>,

	#[serde(default = "def_timezones")]
	pub timezones: Vec<String>,

	#[serde(default)]
	pub clock_12h: bool,

	#[serde(default)]
	pub sets: Vec<SerializedWindowSet>,

	#[serde(default)]
	/// Obsolete, only used for reading
	/// Use `sets` with `name: "global"`
	pub global_set: SerializedWindowStates,

	#[serde(default)]
	pub spawn_overlays: Vec<Arc<str>>,

	#[serde(default, deserialize_with = "deserialize_or_default")]
	pub autostart_apps: Vec<WvrProcessLaunchParams>,

	#[serde(default)]
	pub pinned_apps: Vec<PinnedApp>,

	#[serde(default)]
	pub last_set: u32,

	#[serde(default = "def_true")]
	pub show_username: bool,

	#[serde(default)]
	pub opaque_background: bool,

	#[serde(default)]
	pub xwayland_by_default: bool,

	#[serde(default)]
	pub context_menu_hold_and_release: bool,

	#[serde(default)]
	pub keyboard_middle_click_mode: AltModifier,

	#[serde(default)]
	pub handsfree_pointer: HandsfreePointer,

	#[serde(default)]
	pub handsfree_alt_tab: HandsfreeAltTab,

	#[serde(default = "def_one")]
	pub grid_opacity: f32,

	#[serde(default)]
	pub chroma_key_params: ChromaKeyParams,

	#[serde(default)]
	pub tutorial_graduated: bool,

	#[serde(default)]
	pub whisper_model: Arc<str>,

	#[serde(default)]
	pub whisper_gpu: u32,

	#[serde(default = "def_default")]
	pub color_palette: Arc<str>,

	#[serde(default)]
	pub snap_angle_deg: f32,

	#[serde(default = "def_true")]
	pub wvr_mouse_acceleration: bool,

	#[serde(default)]
	pub wvr_mouse_speed: f32,

	#[serde(default)]
	pub wvr_input_capture: InputCaptureMethod,

	#[serde(default)]
	pub default_positioning: DefaultPositioning,

	#[serde(default = "def_one")]
	pub default_opacity: f32,

	#[serde(default = "def_point15")]
	pub default_curvature: f32,

	#[serde(default)]
	pub middle_grabs_single: bool,

	#[serde(default)]
	pub state_version: u32,

	#[serde(default)]
	pub config_version: u32,
}

// Sorted alphabetically
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Eq, PartialEq, AsRefStr, EnumString)]
pub enum SettingType {
	AllowSliding,
	BlockGameInput,
	BlockGameInputIgnoreWatch,
	BlockPosesOnKbdInteraction,
	CaptureMethod,
	ClickFreezeTimeMs,
	Clock12h,
	DefaultCurvature,
	DefaultOpacity,
	DefaultOverlayScale,
	DefaultPositioning,
	DoubleCursorFix,
	EnableWatch,
	FocusFollowsMouseMode,
	GridOpacity,
	HandsfreeAltTab,
	HandsfreePointer,
	InputCaptureMethod,
	InputEmulationMethod,
	InvertScrollDirectionX,
	InvertScrollDirectionY,
	KeyboardMiddleClick,
	KeyboardSoundEnabled,
	Language,
	LeftHandedMouse,
	LongPressDuration,
	MiddleGrabsSingle,
	MouseAcceleration,
	MouseSpeed,
	NotificationsEnabled,
	NotificationsSoundEnabled,
	OpaqueBackground,
	PointerLerpFactor,
	ScreenRenderDown,
	ScrollSpeed,
	SetsOnWatch,
	ShowGrabHelp,
	ShowUsername,
	SnapAngleDeg,
	SpaceDragAffectsWorld,
	SpaceDragMultiplier,
	SpaceDragUnlocked,
	SpaceGravityDamping,
	SpaceGravityEnabled,
	SpaceGravityFlingStrength,
	SpaceGravityFloorHeight,
	SpaceGravityGravity,
	SpaceGravityGroundFriction,
	SpaceRotateUnlocked,
	UiAnimationSpeed,
	UiGradientIntensity,
	UiRoundMultiplier,
	UprightScreenFix,
	UsePassthrough,
	UseSkybox,
	WatchViewAngleMax,
	WatchViewAngleMin,
	XwaylandByDefault,
}

impl SettingType {
	pub fn mut_var(self, config: &'_ mut GeneralConfig) -> SettingVarRef<'_> {
		match self {
			// Bool
			Self::AllowSliding => SettingVarRef::Bool(&mut config.allow_sliding),
			Self::BlockGameInput => SettingVarRef::Bool(&mut config.block_game_input),
			Self::BlockGameInputIgnoreWatch => SettingVarRef::Bool(&mut config.block_game_input_ignore_watch),
			Self::BlockPosesOnKbdInteraction => SettingVarRef::Bool(&mut config.block_poses_on_kbd_interaction),
			Self::Clock12h => SettingVarRef::Bool(&mut config.clock_12h),
			Self::DoubleCursorFix => SettingVarRef::Bool(&mut config.double_cursor_fix),
			Self::EnableWatch => SettingVarRef::Bool(&mut config.enable_watch),
			Self::FocusFollowsMouseMode => SettingVarRef::Bool(&mut config.focus_follows_mouse_mode),
			Self::ShowGrabHelp => SettingVarRef::Bool(&mut config.show_grab_help),
			Self::InvertScrollDirectionX => SettingVarRef::Bool(&mut config.invert_scroll_direction_x),
			Self::InvertScrollDirectionY => SettingVarRef::Bool(&mut config.invert_scroll_direction_y),
			Self::KeyboardSoundEnabled => SettingVarRef::Bool(&mut config.keyboard_sound_enabled),
			Self::LeftHandedMouse => SettingVarRef::Bool(&mut config.left_handed_mouse),
			Self::MiddleGrabsSingle => SettingVarRef::Bool(&mut config.middle_grabs_single),
			Self::MouseAcceleration => SettingVarRef::Bool(&mut config.wvr_mouse_acceleration),
			Self::NotificationsEnabled => SettingVarRef::Bool(&mut config.notifications_enabled),
			Self::NotificationsSoundEnabled => SettingVarRef::Bool(&mut config.notifications_sound_enabled),
			Self::OpaqueBackground => SettingVarRef::Bool(&mut config.opaque_background),
			Self::ScreenRenderDown => SettingVarRef::Bool(&mut config.screen_render_down),
			Self::SetsOnWatch => SettingVarRef::Bool(&mut config.sets_on_watch),
			Self::ShowUsername => SettingVarRef::Bool(&mut config.show_username),
			Self::SpaceDragAffectsWorld => SettingVarRef::Bool(&mut config.space_drag_affects_world),
			Self::SpaceDragUnlocked => SettingVarRef::Bool(&mut config.space_drag_unlocked),
			Self::SpaceGravityEnabled => SettingVarRef::Bool(&mut config.space_gravity_enabled),
			Self::SpaceRotateUnlocked => SettingVarRef::Bool(&mut config.space_rotate_unlocked),
			Self::UprightScreenFix => SettingVarRef::Bool(&mut config.upright_screen_fix),
			Self::UsePassthrough => SettingVarRef::Bool(&mut config.use_passthrough),
			Self::UseSkybox => SettingVarRef::Bool(&mut config.use_skybox),
			Self::XwaylandByDefault => SettingVarRef::Bool(&mut config.xwayland_by_default),
			// F32
			Self::DefaultCurvature => SettingVarRef::F32(&mut config.default_curvature),
			Self::DefaultOpacity => SettingVarRef::F32(&mut config.default_opacity),
			Self::DefaultOverlayScale => SettingVarRef::F32(&mut config.default_overlay_scale),
			Self::GridOpacity => SettingVarRef::F32(&mut config.grid_opacity),
			Self::LongPressDuration => SettingVarRef::F32(&mut config.long_press_duration),
			Self::MouseSpeed => SettingVarRef::F32(&mut config.wvr_mouse_speed),
			Self::PointerLerpFactor => SettingVarRef::F32(&mut config.pointer_lerp_factor),
			Self::ScrollSpeed => SettingVarRef::F32(&mut config.scroll_speed),
			Self::SnapAngleDeg => SettingVarRef::F32(&mut config.snap_angle_deg),
			Self::SpaceDragMultiplier => SettingVarRef::F32(&mut config.space_drag_multiplier),
			Self::SpaceGravityDamping => SettingVarRef::F32(&mut config.space_gravity_damping),
			Self::SpaceGravityFlingStrength => SettingVarRef::F32(&mut config.space_gravity_fling_strength),
			Self::SpaceGravityFloorHeight => SettingVarRef::F32(&mut config.space_gravity_floor_height),
			Self::SpaceGravityGravity => SettingVarRef::F32(&mut config.space_gravity_gravity),
			Self::SpaceGravityGroundFriction => SettingVarRef::F32(&mut config.space_gravity_ground_friction),
			Self::UiAnimationSpeed => SettingVarRef::F32(&mut config.ui_animation_speed),
			Self::UiGradientIntensity => SettingVarRef::F32(&mut config.ui_gradient_intensity),
			Self::UiRoundMultiplier => SettingVarRef::F32(&mut config.ui_round_multiplier),
			Self::WatchViewAngleMax => SettingVarRef::F32(&mut config.watch_view_angle_max),
			Self::WatchViewAngleMin => SettingVarRef::F32(&mut config.watch_view_angle_min),
			// I32
			Self::ClickFreezeTimeMs => SettingVarRef::I32(&mut config.click_freeze_time_ms),
			_ => panic!("Requested unsupported SettingType"),
		}
	}
}

#[derive(Debug)]
pub enum SettingVarRef<'a> {
	Bool(&'a mut bool),
	I32(&'a mut i32),
	F32(&'a mut f32),
}

impl<'a> TryFrom<SettingVarRef<'a>> for &'a mut bool {
	type Error = anyhow::Error;

	fn try_from(value: SettingVarRef<'a>) -> Result<Self, Self::Error> {
		match value {
			SettingVarRef::Bool(n) => Ok(n),
			_ => anyhow::bail!("Incorrect setting type, expected bool"),
		}
	}
}

impl<'a> TryFrom<SettingVarRef<'a>> for &'a mut i32 {
	type Error = anyhow::Error;

	fn try_from(value: SettingVarRef<'a>) -> Result<Self, Self::Error> {
		match value {
			SettingVarRef::I32(n) => Ok(n),
			_ => anyhow::bail!("Incorrect setting type, expected i32"),
		}
	}
}

impl<'a> TryFrom<SettingVarRef<'a>> for &'a mut f32 {
	type Error = anyhow::Error;

	fn try_from(value: SettingVarRef<'a>) -> Result<Self, Self::Error> {
		match value {
			SettingVarRef::F32(n) => Ok(n),
			_ => anyhow::bail!("Incorrect setting type, expected f32"),
		}
	}
}

impl<'a> SettingVarRef<'a> {
	pub fn set(&mut self, v: SettingVar) -> anyhow::Result<()> {
		match self {
			SettingVarRef::Bool(c) => **c = v.try_into()?,
			SettingVarRef::I32(c) => **c = v.try_into()?,
			SettingVarRef::F32(c) => **c = v.try_into()?,
		}
		Ok(())
	}
}

#[derive(Debug, From, Clone, TryInto)]
#[try_into(error(anyhow::Error))]
pub enum SettingVar {
	Bool(bool),
	I32(i32),
	F32(f32),
}
