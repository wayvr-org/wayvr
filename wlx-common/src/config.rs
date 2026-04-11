use std::{collections::HashMap, sync::Arc};

use chrono::Offset;
use idmap::IdMap;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumProperty, EnumString, VariantArray};
use wayvr_ipc::packet_client::WvrProcessLaunchParams;

use crate::{
	astr_containers::{AStrMap, AStrSet},
	locale::{self},
	overlays::{BackendAttribValue, ToastDisplayMethod, ToastTopic},
	windowing::OverlayWindowState,
};

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

	#[strum(props(Text = "ScreenCopy GPU", Tooltip = "APP_SETTINGS.OPTION.SCREENCOPY_GPU_HELP"))]
	ScreenCopyGpu,

	#[serde(alias = "pw-fallback")]
	#[strum(props(Text = "PipeWire CPU", Tooltip = "APP_SETTINGS.OPTION.PW_FALLBACK_HELP"))]
	PipeWireCpu,

	#[serde(alias = "screencopy")]
	#[strum(props(Text = "ScreenCopy CPU", Tooltip = "APP_SETTINGS.OPTION.SCREENCOPY_HELP"))]
	ScreenCopyCpu,
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
	Meta,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, AsRefStr, EnumString, EnumProperty, VariantArray)]
pub enum HandsfreePointer {
	#[strum(props(Translation = "APP_SETTINGS.OPTION.NONE"))]
	None,
	#[strum(props(Translation = "APP_SETTINGS.OPTION.HMD_PINCH"))]
	#[default]
	Hmd,
	#[strum(props(Translation = "APP_SETTINGS.OPTION.HMD_ONLY"))]
	HmdOnly,
	#[strum(props(Translation = "APP_SETTINGS.OPTION.EYE_PINCH"))]
	EyeTracking,
	#[strum(props(Translation = "APP_SETTINGS.OPTION.EYE_ONLY"))]
	EyeTrackingOnly,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SerializedWindowSet {
	pub name: Arc<str>,

	#[serde(default)]
	pub overlays: SerializedWindowStates,

	#[serde(default)]
	pub hidden_overlays: SerializedWindowStates,
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

const fn def_false() -> bool {
	false
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

const fn def_point13() -> f32 {
	0.13
}

const fn def_point18() -> f32 {
	0.18
}

const fn def_point12() -> f32 {
	0.12
}

const fn def_point32() -> f32 {
	0.32
}

const fn def_point45() -> f32 {
	0.45
}

const fn def_point145() -> f32 {
	1.45
}

const fn def_osc_port() -> u16 {
	9000
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

fn def_empty() -> Arc<str> {
	"".into()
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

	pub color_text: Option<String>,
	pub color_accent: Option<String>,
	pub color_danger: Option<String>,
	pub color_faded: Option<String>,
	pub color_background: Option<String>,

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
	pub attribs: AStrMap<Vec<BackendAttribValue>>,

	#[serde(default = "def_click_freeze_time_ms")]
	pub click_freeze_time_ms: i32,

	#[serde(default = "def_false")]
	pub invert_scroll_direction_x: bool,

	#[serde(default = "def_false")]
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
	pub desktop_view_scale: f32,

	#[serde(default = "def_half")]
	pub watch_view_angle_min: f32,

	#[serde(default = "def_point7")]
	pub watch_view_angle_max: f32,

	#[serde(default = "def_osc_port")]
	pub osc_out_port: u16,

	#[serde(default = "def_false")]
	pub upright_screen_fix: bool,

	#[serde(default = "def_false")]
	pub double_cursor_fix: bool,

	#[serde(default = "def_false")]
	pub sets_on_watch: bool,

	#[serde(default = "def_false")]
	pub hide_grab_help: bool,

	#[serde(default)]
	pub custom_panels: AStrSet,

	#[serde(default)]
	pub capture_method: CaptureMethod,

	#[serde(default = "def_point7")]
	pub xr_click_sensitivity: f32,

	#[serde(default = "def_half")]
	pub xr_click_sensitivity_release: f32,

	#[serde(default = "def_true")]
	pub allow_sliding: bool,

	#[serde(default = "def_false")]
	pub focus_follows_mouse_mode: bool,

	#[serde(default = "def_false")]
	pub left_handed_mouse: bool,

	#[serde(default = "def_false")]
	pub block_game_input: bool,

	#[serde(default = "def_true")]
	pub block_game_input_ignore_watch: bool,

	#[serde(default = "def_true")]
	pub block_poses_on_kbd_interaction: bool,

	#[serde(default = "def_one")]
	pub space_drag_multiplier: f32,

	#[serde(default = "def_empty")]
	pub skybox_texture: Arc<str>,

	#[serde(default = "def_true")]
	pub use_skybox: bool,

	#[serde(default = "def_true")]
	pub use_passthrough: bool,

	#[serde(default = "def_max_height")]
	pub screen_max_height: u16,

	#[serde(default = "def_false")]
	pub screen_render_down: bool,

	#[serde(default = "def_point3")]
	pub pointer_lerp_factor: f32,

	#[serde(default = "def_point13")]
	pub focused_screen_assist_x: f32,

	#[serde(default = "def_point18")]
	pub focused_screen_assist_y: f32,

	#[serde(default = "def_point13")]
	pub focused_screen_rotate_assist_x: f32,

	#[serde(default = "def_point12")]
	pub focused_screen_rotate_assist_y: f32,

	#[serde(default = "def_point32")]
	#[serde(alias = "focused_screen_curvature")]
	pub focused_screen_curve_x: f32,

	#[serde(default = "def_point45")]
	pub focused_screen_distance: f32,

	#[serde(default = "def_point145")]
	pub focused_screen_scale: f32,

	#[serde(default = "def_true")]
	pub space_drag_unlocked: bool,

	#[serde(default = "def_false")]
	pub space_rotate_unlocked: bool,

	#[serde(default)]
	pub alt_click_down: Vec<String>,

	#[serde(default)]
	pub alt_click_up: Vec<String>,

	#[serde(default = "def_timezones")]
	pub timezones: Vec<String>,

	#[serde(default = "def_false")]
	pub clock_12h: bool,

	#[serde(default)]
	pub sets: Vec<SerializedWindowSet>,

	#[serde(default)]
	pub global_set: SerializedWindowStates,

	#[serde(default)]
	pub autostart_apps: Vec<WvrProcessLaunchParams>,

	#[serde(default)]
	pub last_set: u32,

	#[serde(default)]
	pub hide_username: bool,

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

	#[serde(default = "def_one")]
	pub grid_opacity: f32,

	#[serde(default = "def_false")]
	pub keyboard_swipe_to_type_enabled: bool,
}
