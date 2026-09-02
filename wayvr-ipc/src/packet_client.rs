// Contents of this file should be the same as on wayvr.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{ipc::Serial, packet_server};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Handshake {
	pub protocol_version: u32, // always set to PROTOCOL_VERSION
	pub magic: String,         // always set to CONNECTION_MAGIC
	pub client_name: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum PositionMode {
	Float,
	Anchor,
	Static,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HandsfreeMode {
	None,
	Hmd,
	HmdPinch,
	EyeTracking,
	EyeTrackingPinch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HandsfreeAction {
	Click,
	Grab,
	RightModifier,
	MiddleModifier,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HandsfreeParams {
	SetMode(HandsfreeMode),
	Press(HandsfreeAction),
	Release(HandsfreeAction),
	Toggle(HandsfreeAction),
	Scroll(f32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WvrProcessLaunchParams {
	pub name: String,
	pub exec: String,
	pub env: Vec<String>,
	pub args: String,
	pub icon: Option<String>,
	pub resolution: [u32; 2],
	pub pos_mode: PositionMode,
	pub userdata: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WlxHapticsParams {
	pub intensity: f32,
	pub duration: f32,
	pub frequency: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WlxModifyPanelCommand {
	SetText(String),
	SetColor(String),
	SetImage(String),
	SetVisible(bool),
	SetStickyState(bool),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WlxModifyPanelParams {
	pub overlay: String,
	pub element: String,
	pub command: WlxModifyPanelCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WlxHand {
	Left,
	Right,
}

// see wlx_common::windowing::Positioning
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum WlxPositioning {
	Floating,
	Anchored,
	Static,
	FollowHead {
		#[serde(default)]
		lerp: f32,
	},
	FollowHand {
		hand: WlxHand,
		#[serde(default)]
		lerp: f32,
	},
}

// see wlx_common::windowing::OverlayWindowState
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WlxWindowStateField {
	Alpha,
	Grabbable,
	Interactable,
	Positioning,
	Curvature,
	Additive,
	BlockInput,
	AlignToHmd,
	/// see wlx_common::windowing::OverlayWindowConfig::global
	Global,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WlxWindowStateValue {
	Bool(bool),
	Float(f32),
	Positioning(WlxPositioning),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WlxWindowStateGetParams {
	pub overlay: String,
	pub field: WlxWindowStateField,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WlxWindowStateSetParams {
	pub overlay: String,
	pub field: WlxWindowStateField,
	pub value: WlxWindowStateValue,
}

// see wlx_common::overlays::BackendAttrib
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WlxWindowAttrib {
	Stereo,
	StereoFullFrame,
	StereoAdjustMouse,
	MouseTransform,
	WindowSize,
}

// see wlx_common::overlays::StereoMode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WlxStereoMode {
	None,
	LeftRight,
	RightLeft,
	TopBottom,
	BottomTop,
}

// see wlx_common::overlays::MouseTransform
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WlxMouseTransform {
	Default,
	Normal,
	Rotated90,
	Rotated180,
	Rotated270,
	Flipped,
	Flipped90,
	Flipped180,
	Flipped270,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WlxWindowAttribValue {
	Stereo(WlxStereoMode),
	StereoFullFrame(bool),
	StereoAdjustMouse(bool),
	MouseTransform(WlxMouseTransform),
	WindowSize([u32; 2]),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WlxWindowAttribGetParams {
	pub overlay: String,
	pub attrib: WlxWindowAttrib,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WlxWindowAttribSetParams {
	pub overlay: String,
	pub attrib: WlxWindowAttrib,
	pub value: WlxWindowAttribValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WlxOverlayListParams {
	pub visible: bool,
	pub hidden: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum PacketClient {
	Handshake(Handshake),
	WlxOverlayList(Serial, WlxOverlayListParams),
	WvrWindowSetVisible(packet_server::WvrWindowHandle, bool),
	WvrProcessGet(Serial, packet_server::WvrProcessHandle),
	WvrProcessLaunch(Serial, WvrProcessLaunchParams),
	WvrProcessList(Serial),
	WvrProcessTerminate(packet_server::WvrProcessHandle),
	WvrInputCapture(bool),
	WlxInputState(Serial),
	WlxModifyPanel(WlxModifyPanelParams),
	WlxDeviceHaptics(usize, WlxHapticsParams),
	WlxShowHide,
	WlxSwitchSet(Option<usize>),
	WlxHandsfree(HandsfreeParams),
	WlxWindowStateGet(Serial, WlxWindowStateGetParams),
	WlxWindowStateSet(WlxWindowStateSetParams),
	WlxWindowAttribGet(Serial, WlxWindowAttribGetParams),
	WlxWindowAttribSet(Serial, WlxWindowAttribSetParams),
	WlxOverlaySetVisible(String, bool),
}
