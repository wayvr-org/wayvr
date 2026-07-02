use crate::openxr_bindings_schema::{
	XrControllerProfile, XrControllerUserPath, XrInputComponent, XrInputSide, XrInputSubpath, XrInputSubpathKind,
};

pub const OPENXR_INPUT_PROFILES: &[&XrControllerProfile] = &[
	&OCULUS_TOUCH_CONTROLLER_PROFILE,
	&VALVE_INDEX_CONTROLLER_PROFILE,
	&VALVE_FRAME_CONTROLLER_VALVE_PROFILE,
	&HTC_VIVE_CONTROLLER_PROFILE,
	&HP_MIXED_REALITY_CONTROLLER_PROFILE,
	&MICROSOFT_MOTION_CONTROLLER_PROFILE,
	&SAMSUNG_ODYSSEY_CONTROLLER_PROFILE,
	&KHR_GENERIC_CONTROLLER_PROFILE,
];

pub const VALVE_INDEX_CONTROLLER_PROFILE: XrControllerProfile = XrControllerProfile {
	display_name: "Valve Index Controller",
	extension: None,
	profile_id: "/interaction_profiles/valve/index_controller",
	user_paths: &[
		XrControllerUserPath {
			hand: XrInputSide::Left,
			paths: VALVE_INDEX_USER_PATHS,
		},
		XrControllerUserPath {
			hand: XrInputSide::Right,
			paths: VALVE_INDEX_USER_PATHS,
		},
	],
};

const VALVE_INDEX_USER_PATHS: &[XrInputSubpath] = &[
	XrInputSubpath {
		kind: XrInputSubpathKind::System,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::A,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::B,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Squeeze,
		components: &[XrInputComponent::Value, XrInputComponent::Force],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trigger,
		components: &[
			XrInputComponent::Click,
			XrInputComponent::Value,
			XrInputComponent::Touch,
		],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Thumbstick,
		components: &[
			XrInputComponent::Click,
			XrInputComponent::Touch,
			XrInputComponent::X,
			XrInputComponent::Y,
		],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trackpad,
		components: &[
			XrInputComponent::Force,
			XrInputComponent::Touch,
			XrInputComponent::X,
			XrInputComponent::Y,
		],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Grip,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Aim,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Haptic,
		components: &[],
	},
];

pub static VALVE_FRAME_CONTROLLER_VALVE_PROFILE: XrControllerProfile = XrControllerProfile {
	display_name: "Steam Frame Controller",
	extension: Some("XR_VALVE_frame_controller_interaction"),
	profile_id: "/interaction_profiles/valve/frame_controller_valve",
	user_paths: &[
		XrControllerUserPath {
			hand: XrInputSide::Left,
			paths: VALVE_FRAME_CONTROLLER_VALVE_LEFT_USER_PATHS,
		},
		XrControllerUserPath {
			hand: XrInputSide::Right,
			paths: VALVE_FRAME_CONTROLLER_VALVE_RIGHT_USER_PATHS,
		},
	],
};

static VALVE_FRAME_CONTROLLER_VALVE_RIGHT_USER_PATHS: &[XrInputSubpath] = &[
	XrInputSubpath {
		kind: XrInputSubpathKind::A,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::B,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::X,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Y,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Menu,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::System,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Bumper,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Squeeze,
		components: &[
			XrInputComponent::Click,
			XrInputComponent::Touch,
			XrInputComponent::Value,
		],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trigger,
		components: &[
			XrInputComponent::Click,
			XrInputComponent::Touch,
			XrInputComponent::Value,
		],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Thumbstick,
		components: &[
			XrInputComponent::Click,
			XrInputComponent::Touch,
			XrInputComponent::X,
			XrInputComponent::Y,
		],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Grip,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Aim,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Haptic,
		components: &[],
	},
];

static VALVE_FRAME_CONTROLLER_VALVE_LEFT_USER_PATHS: &[XrInputSubpath] = &[
	XrInputSubpath {
		kind: XrInputSubpathKind::DpadUp,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::DpadLeft,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::DpadDown,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::DpadRight,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::View,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::System,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Bumper,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Squeeze,
		components: &[
			XrInputComponent::Click,
			XrInputComponent::Touch,
			XrInputComponent::Value,
		],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trigger,
		components: &[
			XrInputComponent::Click,
			XrInputComponent::Touch,
			XrInputComponent::Value,
		],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Thumbstick,
		components: &[
			XrInputComponent::Click,
			XrInputComponent::Touch,
			XrInputComponent::X,
			XrInputComponent::Y,
		],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Grip,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Aim,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Haptic,
		components: &[],
	},
];

pub const OCULUS_TOUCH_CONTROLLER_PROFILE: XrControllerProfile = XrControllerProfile {
	display_name: "Touch Controller",
	extension: None,
	profile_id: "/interaction_profiles/oculus/touch_controller",
	user_paths: &[
		XrControllerUserPath {
			hand: XrInputSide::Left,
			paths: OCULUS_TOUCH_LEFT_USER_PATHS,
		},
		XrControllerUserPath {
			hand: XrInputSide::Right,
			paths: OCULUS_TOUCH_RIGHT_USER_PATHS,
		},
	],
};

const OCULUS_TOUCH_LEFT_USER_PATHS: &[XrInputSubpath] = &[
	XrInputSubpath {
		kind: XrInputSubpathKind::X,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Y,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Menu,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Squeeze,
		components: &[XrInputComponent::Value],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trigger,
		components: &[XrInputComponent::Value, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Thumbstick,
		components: &[
			XrInputComponent::Click,
			XrInputComponent::Touch,
			XrInputComponent::X,
			XrInputComponent::Y,
		],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Thumbrest,
		components: &[XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Grip,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Aim,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Haptic,
		components: &[],
	},
];

const OCULUS_TOUCH_RIGHT_USER_PATHS: &[XrInputSubpath] = &[
	XrInputSubpath {
		kind: XrInputSubpathKind::A,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::B,
		components: &[XrInputComponent::Click, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::System,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Squeeze,
		components: &[XrInputComponent::Value],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trigger,
		components: &[XrInputComponent::Value, XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Thumbstick,
		components: &[
			XrInputComponent::Click,
			XrInputComponent::Touch,
			XrInputComponent::X,
			XrInputComponent::Y,
		],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Thumbrest,
		components: &[XrInputComponent::Touch],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Grip,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Aim,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Haptic,
		components: &[],
	},
];

pub const HP_MIXED_REALITY_CONTROLLER_PROFILE: XrControllerProfile = XrControllerProfile {
	display_name: "HP Reverb G2 Controller",
	extension: None,
	profile_id: "/interaction_profiles/hp/mixed_reality_controller",
	user_paths: &[
		XrControllerUserPath {
			hand: XrInputSide::Left,
			paths: HP_MIXED_REALITY_LEFT_USER_PATHS,
		},
		XrControllerUserPath {
			hand: XrInputSide::Right,
			paths: HP_MIXED_REALITY_RIGHT_USER_PATHS,
		},
	],
};

const HP_MIXED_REALITY_LEFT_USER_PATHS: &[XrInputSubpath] = &[
	XrInputSubpath {
		kind: XrInputSubpathKind::X,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Y,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Menu,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Squeeze,
		components: &[XrInputComponent::Value],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trigger,
		components: &[XrInputComponent::Value],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Thumbstick,
		components: &[XrInputComponent::Click, XrInputComponent::X, XrInputComponent::Y],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Grip,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Aim,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Haptic,
		components: &[],
	},
];

const HP_MIXED_REALITY_RIGHT_USER_PATHS: &[XrInputSubpath] = &[
	XrInputSubpath {
		kind: XrInputSubpathKind::A,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::B,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Menu,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Squeeze,
		components: &[XrInputComponent::Value],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trigger,
		components: &[XrInputComponent::Value],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Thumbstick,
		components: &[XrInputComponent::Click, XrInputComponent::X, XrInputComponent::Y],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Grip,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Aim,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Haptic,
		components: &[],
	},
];

pub const SAMSUNG_ODYSSEY_CONTROLLER_PROFILE: XrControllerProfile = XrControllerProfile {
	display_name: "Samsung Odyssey Controller",
	extension: None,
	profile_id: "/interaction_profiles/samsung/odyssey_controller",
	user_paths: &[
		XrControllerUserPath {
			hand: XrInputSide::Left,
			paths: SAMSUNG_ODYSSEY_USER_PATHS,
		},
		XrControllerUserPath {
			hand: XrInputSide::Right,
			paths: SAMSUNG_ODYSSEY_USER_PATHS,
		},
	],
};

const SAMSUNG_ODYSSEY_USER_PATHS: &[XrInputSubpath] = &[
	XrInputSubpath {
		kind: XrInputSubpathKind::Menu,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Squeeze,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trigger,
		components: &[XrInputComponent::Value],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Thumbstick,
		components: &[XrInputComponent::Click, XrInputComponent::X, XrInputComponent::Y],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trackpad,
		components: &[
			XrInputComponent::Click,
			XrInputComponent::Touch,
			XrInputComponent::X,
			XrInputComponent::Y,
		],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Grip,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Aim,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Haptic,
		components: &[],
	},
];

pub const HTC_VIVE_CONTROLLER_PROFILE: XrControllerProfile = XrControllerProfile {
	display_name: "HTC Vive Controller",
	extension: None,
	profile_id: "/interaction_profiles/htc/vive_controller",
	user_paths: &[
		XrControllerUserPath {
			hand: XrInputSide::Left,
			paths: HTC_VIVE_USER_PATHS,
		},
		XrControllerUserPath {
			hand: XrInputSide::Right,
			paths: HTC_VIVE_USER_PATHS,
		},
	],
};

const HTC_VIVE_USER_PATHS: &[XrInputSubpath] = &[
	XrInputSubpath {
		kind: XrInputSubpathKind::System,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Squeeze,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Menu,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trigger,
		components: &[XrInputComponent::Click, XrInputComponent::Value],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trackpad,
		components: &[
			XrInputComponent::Click,
			XrInputComponent::Touch,
			XrInputComponent::X,
			XrInputComponent::Y,
		],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Grip,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Aim,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Haptic,
		components: &[],
	},
];

pub const MICROSOFT_MOTION_CONTROLLER_PROFILE: XrControllerProfile = XrControllerProfile {
	display_name: "Microsoft WMR Controller",
	extension: None,
	profile_id: "/interaction_profiles/microsoft/motion_controller",
	user_paths: &[
		XrControllerUserPath {
			hand: XrInputSide::Left,
			paths: MICROSOFT_MOTION_CONTROLLER_USER_PATHS,
		},
		XrControllerUserPath {
			hand: XrInputSide::Right,
			paths: MICROSOFT_MOTION_CONTROLLER_USER_PATHS,
		},
	],
};

const MICROSOFT_MOTION_CONTROLLER_USER_PATHS: &[XrInputSubpath] = &[
	XrInputSubpath {
		kind: XrInputSubpathKind::Menu,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Squeeze,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trigger,
		components: &[XrInputComponent::Value],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Thumbstick,
		components: &[XrInputComponent::Click, XrInputComponent::X, XrInputComponent::Y],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trackpad,
		components: &[
			XrInputComponent::Click,
			XrInputComponent::Touch,
			XrInputComponent::X,
			XrInputComponent::Y,
		],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Grip,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Aim,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Haptic,
		components: &[],
	},
];

pub const KHR_GENERIC_CONTROLLER_PROFILE: XrControllerProfile = XrControllerProfile {
	display_name: "Khronos Generic Controller",
	extension: None,
	profile_id: "/interaction_profiles/khr/generic_controller",
	user_paths: &[
		XrControllerUserPath {
			hand: XrInputSide::Left,
			paths: KHR_GENERIC_CONTROLLER_USER_PATHS,
		},
		XrControllerUserPath {
			hand: XrInputSide::Right,
			paths: KHR_GENERIC_CONTROLLER_USER_PATHS,
		},
	],
};

const KHR_GENERIC_CONTROLLER_USER_PATHS: &[XrInputSubpath] = &[
	XrInputSubpath {
		kind: XrInputSubpathKind::Primary,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Secondary,
		components: &[XrInputComponent::Click],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Thumbstick,
		components: &[XrInputComponent::Click, XrInputComponent::X, XrInputComponent::Y],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Squeeze,
		components: &[XrInputComponent::Value],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Trigger,
		components: &[XrInputComponent::Value],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Grip,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Aim,
		components: &[XrInputComponent::Pose],
	},
	XrInputSubpath {
		kind: XrInputSubpathKind::Haptic,
		components: &[],
	},
];
