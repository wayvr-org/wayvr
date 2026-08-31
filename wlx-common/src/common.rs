use std::fmt::Debug;

use serde::{Deserialize, Serialize};
use wayvr_ipc::packet_client::WlxHand;

#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default)]
#[repr(u8)]
pub enum LeftRight {
	#[default]
	Left,
	Right,
}

impl From<LeftRight> for WlxHand {
	fn from(value: LeftRight) -> Self {
		match value {
			LeftRight::Left => Self::Left,
			LeftRight::Right => Self::Right,
		}
	}
}

impl From<WlxHand> for LeftRight {
	fn from(value: WlxHand) -> Self {
		match value {
			WlxHand::Left => Self::Left,
			WlxHand::Right => Self::Right,
		}
	}
}
