use glam::Affine3A;
use serde::{Deserialize, Serialize};

use crate::{common::LeftRight, config::DefaultPositioning};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub enum Positioning {
	/// Stays in place, recenters relative to HMD
	#[default]
	Floating,
	/// Stays in place, recenters relative to anchor. Follows anchor during anchor grab.
	Anchored,
	/// Stays in place, no recentering
	Static,
	/// Following HMD
	FollowHead {
		#[serde(default)]
		lerp: f32,
	},
	/// Following hand
	FollowHand {
		hand: LeftRight,
		#[serde(default)]
		lerp: f32,
	},
}

impl From<DefaultPositioning> for Positioning {
	fn from(value: DefaultPositioning) -> Self {
		match value {
			DefaultPositioning::Anchored => Self::Anchored,
			DefaultPositioning::Floating => Self::Floating,
			DefaultPositioning::Static => Self::Static,
		}
	}
}

impl Positioning {
	pub const fn get_lerp(self) -> Option<f32> {
		match self {
			Self::FollowHead { lerp } => Some(lerp),
			Self::FollowHand { lerp, .. } => Some(lerp),
			Self::Floating | Self::Anchored | Self::Static => None,
		}
	}
	pub const fn with_lerp(mut self, value: f32) -> Self {
		match self {
			Self::FollowHead { ref mut lerp } => *lerp = value,
			Self::FollowHand { ref mut lerp, .. } => *lerp = value,
			Self::Floating | Self::Anchored | Self::Static => {}
		}
		self
	}
}

// Contains the window state for a given set
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayWindowState {
	#[serde(skip_serializing, skip_deserializing)]
	pub transform: Affine3A,
	pub alpha: f32,
	pub grabbable: bool,
	pub interactable: bool,
	pub positioning: Positioning,
	pub curvature: Option<f32>,
	pub additive: bool,
	pub saved_transform: Option<Affine3A>,
	pub block_input: bool,
	pub angle_fade: bool,
	#[serde(default)]
	pub align_to_hmd: bool,
}

impl Default for OverlayWindowState {
	fn default() -> Self {
		Self {
			grabbable: false,
			interactable: false,
			alpha: 1.0,
			positioning: Positioning::Floating,
			curvature: None,
			transform: Affine3A::IDENTITY,
			additive: false,
			saved_transform: None,
			block_input: true,
			angle_fade: false,
			align_to_hmd: false,
		}
	}
}
