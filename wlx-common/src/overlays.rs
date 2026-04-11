use std::sync::Arc;

use idmap_derive::IntegerId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntegerId, Serialize, Deserialize)]
pub enum ToastTopic {
	System,
	Error,
	DesktopNotification,
	XSNotification,
	IpdChange,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ToastDisplayMethod {
	Hide,
	Center,
	Watch,
}

#[derive(Debug, Clone, Copy, IntegerId, PartialEq)]
pub enum BackendAttrib {
	Stereo,
	StereoFullFrame,
	StereoAdjustMouse,
	MouseTransform,
	CropRect,
	Icon,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BackendAttribValue {
	Stereo(StereoMode),
	StereoFullFrame(bool),
	StereoAdjustMouse(bool),
	MouseTransform(MouseTransform),
	CropRect([f32; 4]),
	#[serde(skip_serializing, skip_deserializing)]
	Icon(Arc<str>),
}

impl BackendAttribValue {
	pub fn is_default(&self) -> bool {
		match self {
			Self::Stereo(val) => *val == StereoMode::default(),
			Self::StereoFullFrame(val) => !*val,
			Self::StereoAdjustMouse(val) => !*val,
			Self::MouseTransform(val) => *val == MouseTransform::default(),
			Self::CropRect(val) => *val == [0.0, 0.0, 1.0, 1.0],
			Self::Icon(_) => false,
		}
	}
}

#[derive(Default, Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum StereoMode {
	#[default]
	None,
	LeftRight,
	RightLeft,
	TopBottom,
	BottomTop,
}

#[derive(Default, Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum MouseTransform {
	#[default]
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
