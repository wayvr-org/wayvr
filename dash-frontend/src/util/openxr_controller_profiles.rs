use crate::util::openxr_bindings_schema::{
	Component, ControllerProfile, ControllerUserPath, Side, Subpath, SubpathKind,
};

pub const OPENXR_INPUT_PROFILES: &[&ControllerProfile] = &[
	&VALVE_INDEX_CONTROLLER_PROFILE,
	&OCULUS_TOUCH_CONTROLLER_PROFILE,
	&HTC_VIVE_CONTROLLER_PROFILE,
	&HP_MIXED_REALITY_CONTROLLER_PROFILE,
	&MICROSOFT_MOTION_CONTROLLER_PROFILE,
	&SAMSUNG_ODYSSEY_CONTROLLER_PROFILE,
	&KHR_GENERIC_CONTROLLER_PROFILE,
];

pub const VALVE_INDEX_CONTROLLER_PROFILE: ControllerProfile = ControllerProfile {
	display_name: "Valve Index Controller",
	profile_id: "/interaction_profiles/valve/index_controller",
	user_paths: &[
		ControllerUserPath {
			hand: Side::Left,
			paths: VALVE_INDEX_USER_PATHS,
		},
		ControllerUserPath {
			hand: Side::Right,
			paths: VALVE_INDEX_USER_PATHS,
		},
	],
};

const VALVE_INDEX_USER_PATHS: &[Subpath] = &[
	Subpath {
		kind: SubpathKind::System,
		components: &[Component::Click, Component::Touch],
	},
	Subpath {
		kind: SubpathKind::A,
		components: &[Component::Click, Component::Touch],
	},
	Subpath {
		kind: SubpathKind::B,
		components: &[Component::Click, Component::Touch],
	},
	Subpath {
		kind: SubpathKind::Squeeze,
		components: &[Component::Value, Component::Force],
	},
	Subpath {
		kind: SubpathKind::Trigger,
		components: &[Component::Click, Component::Value, Component::Touch],
	},
	Subpath {
		kind: SubpathKind::Thumbstick,
		components: &[Component::Click, Component::Touch, Component::X, Component::Y],
	},
	Subpath {
		kind: SubpathKind::Trackpad,
		components: &[Component::Force, Component::Touch, Component::X, Component::Y],
	},
	Subpath {
		kind: SubpathKind::Grip,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Aim,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Haptic,
		components: &[],
	},
];

pub const OCULUS_TOUCH_CONTROLLER_PROFILE: ControllerProfile = ControllerProfile {
	display_name: "Touch Controller",
	profile_id: "/interaction_profiles/oculus/touch_controller",
	user_paths: &[
		ControllerUserPath {
			hand: Side::Left,
			paths: OCULUS_TOUCH_LEFT_USER_PATHS,
		},
		ControllerUserPath {
			hand: Side::Right,
			paths: OCULUS_TOUCH_RIGHT_USER_PATHS,
		},
	],
};

const OCULUS_TOUCH_LEFT_USER_PATHS: &[Subpath] = &[
	Subpath {
		kind: SubpathKind::X,
		components: &[Component::Click, Component::Touch],
	},
	Subpath {
		kind: SubpathKind::Y,
		components: &[Component::Click, Component::Touch],
	},
	Subpath {
		kind: SubpathKind::Menu,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Squeeze,
		components: &[Component::Value],
	},
	Subpath {
		kind: SubpathKind::Trigger,
		components: &[Component::Value, Component::Touch],
	},
	Subpath {
		kind: SubpathKind::Thumbstick,
		components: &[Component::Click, Component::Touch, Component::X, Component::Y],
	},
	Subpath {
		kind: SubpathKind::Thumbrest,
		components: &[Component::Touch],
	},
	Subpath {
		kind: SubpathKind::Grip,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Aim,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Haptic,
		components: &[],
	},
];

const OCULUS_TOUCH_RIGHT_USER_PATHS: &[Subpath] = &[
	Subpath {
		kind: SubpathKind::A,
		components: &[Component::Click, Component::Touch],
	},
	Subpath {
		kind: SubpathKind::B,
		components: &[Component::Click, Component::Touch],
	},
	Subpath {
		kind: SubpathKind::System,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Squeeze,
		components: &[Component::Value],
	},
	Subpath {
		kind: SubpathKind::Trigger,
		components: &[Component::Value, Component::Touch],
	},
	Subpath {
		kind: SubpathKind::Thumbstick,
		components: &[Component::Click, Component::Touch, Component::X, Component::Y],
	},
	Subpath {
		kind: SubpathKind::Thumbrest,
		components: &[Component::Touch],
	},
	Subpath {
		kind: SubpathKind::Grip,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Aim,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Haptic,
		components: &[],
	},
];

pub const HP_MIXED_REALITY_CONTROLLER_PROFILE: ControllerProfile = ControllerProfile {
	display_name: "HP Reverb G2 Controller",
	profile_id: "/interaction_profiles/hp/mixed_reality_controller",
	user_paths: &[
		ControllerUserPath {
			hand: Side::Left,
			paths: HP_MIXED_REALITY_LEFT_USER_PATHS,
		},
		ControllerUserPath {
			hand: Side::Right,
			paths: HP_MIXED_REALITY_RIGHT_USER_PATHS,
		},
	],
};

const HP_MIXED_REALITY_LEFT_USER_PATHS: &[Subpath] = &[
	Subpath {
		kind: SubpathKind::X,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Y,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Menu,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Squeeze,
		components: &[Component::Value],
	},
	Subpath {
		kind: SubpathKind::Trigger,
		components: &[Component::Value],
	},
	Subpath {
		kind: SubpathKind::Thumbstick,
		components: &[Component::Click, Component::X, Component::Y],
	},
	Subpath {
		kind: SubpathKind::Grip,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Aim,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Haptic,
		components: &[],
	},
];

const HP_MIXED_REALITY_RIGHT_USER_PATHS: &[Subpath] = &[
	Subpath {
		kind: SubpathKind::A,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::B,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Menu,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Squeeze,
		components: &[Component::Value],
	},
	Subpath {
		kind: SubpathKind::Trigger,
		components: &[Component::Value],
	},
	Subpath {
		kind: SubpathKind::Thumbstick,
		components: &[Component::Click, Component::X, Component::Y],
	},
	Subpath {
		kind: SubpathKind::Grip,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Aim,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Haptic,
		components: &[],
	},
];

pub const SAMSUNG_ODYSSEY_CONTROLLER_PROFILE: ControllerProfile = ControllerProfile {
	display_name: "Samsung Odyssey Controller",
	profile_id: "/interaction_profiles/samsung/odyssey_controller",
	user_paths: &[
		ControllerUserPath {
			hand: Side::Left,
			paths: SAMSUNG_ODYSSEY_USER_PATHS,
		},
		ControllerUserPath {
			hand: Side::Right,
			paths: SAMSUNG_ODYSSEY_USER_PATHS,
		},
	],
};

const SAMSUNG_ODYSSEY_USER_PATHS: &[Subpath] = &[
	Subpath {
		kind: SubpathKind::Menu,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Squeeze,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Trigger,
		components: &[Component::Value],
	},
	Subpath {
		kind: SubpathKind::Thumbstick,
		components: &[Component::Click, Component::X, Component::Y],
	},
	Subpath {
		kind: SubpathKind::Trackpad,
		components: &[Component::Click, Component::Touch, Component::X, Component::Y],
	},
	Subpath {
		kind: SubpathKind::Grip,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Aim,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Haptic,
		components: &[],
	},
];

pub const HTC_VIVE_CONTROLLER_PROFILE: ControllerProfile = ControllerProfile {
	display_name: "HTC Vive Controller",
	profile_id: "/interaction_profiles/htc/vive_controller",
	user_paths: &[
		ControllerUserPath {
			hand: Side::Left,
			paths: HTC_VIVE_USER_PATHS,
		},
		ControllerUserPath {
			hand: Side::Right,
			paths: HTC_VIVE_USER_PATHS,
		},
	],
};

const HTC_VIVE_USER_PATHS: &[Subpath] = &[
	Subpath {
		kind: SubpathKind::System,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Squeeze,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Menu,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Trigger,
		components: &[Component::Click, Component::Value],
	},
	Subpath {
		kind: SubpathKind::Trackpad,
		components: &[Component::Click, Component::Touch, Component::X, Component::Y],
	},
	Subpath {
		kind: SubpathKind::Grip,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Aim,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Haptic,
		components: &[],
	},
];

pub const MICROSOFT_MOTION_CONTROLLER_PROFILE: ControllerProfile = ControllerProfile {
	display_name: "Microsoft WMR Controller",
	profile_id: "/interaction_profiles/microsoft/motion_controller",
	user_paths: &[
		ControllerUserPath {
			hand: Side::Left,
			paths: MICROSOFT_MOTION_CONTROLLER_USER_PATHS,
		},
		ControllerUserPath {
			hand: Side::Right,
			paths: MICROSOFT_MOTION_CONTROLLER_USER_PATHS,
		},
	],
};

const MICROSOFT_MOTION_CONTROLLER_USER_PATHS: &[Subpath] = &[
	Subpath {
		kind: SubpathKind::Menu,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Squeeze,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Trigger,
		components: &[Component::Value],
	},
	Subpath {
		kind: SubpathKind::Thumbstick,
		components: &[Component::Click, Component::X, Component::Y],
	},
	Subpath {
		kind: SubpathKind::Trackpad,
		components: &[Component::Click, Component::Touch, Component::X, Component::Y],
	},
	Subpath {
		kind: SubpathKind::Grip,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Aim,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Haptic,
		components: &[],
	},
];

pub const KHR_GENERIC_CONTROLLER_PROFILE: ControllerProfile = ControllerProfile {
	display_name: "Khronos Generic Controller",
	profile_id: "/interaction_profiles/khr/generic_controller",
	user_paths: &[
		ControllerUserPath {
			hand: Side::Left,
			paths: KHR_GENERIC_CONTROLLER_USER_PATHS,
		},
		ControllerUserPath {
			hand: Side::Right,
			paths: KHR_GENERIC_CONTROLLER_USER_PATHS,
		},
	],
};

const KHR_GENERIC_CONTROLLER_USER_PATHS: &[Subpath] = &[
	Subpath {
		kind: SubpathKind::Primary,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Secondary,
		components: &[Component::Click],
	},
	Subpath {
		kind: SubpathKind::Thumbstick,
		components: &[Component::Click, Component::X, Component::Y],
	},
	Subpath {
		kind: SubpathKind::Squeeze,
		components: &[Component::Value],
	},
	Subpath {
		kind: SubpathKind::Trigger,
		components: &[Component::Value],
	},
	Subpath {
		kind: SubpathKind::Grip,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Aim,
		components: &[Component::Pose],
	},
	Subpath {
		kind: SubpathKind::Haptic,
		components: &[],
	},
];
