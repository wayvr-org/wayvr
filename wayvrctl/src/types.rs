use wayvr_ipc::packet_client as ipc;

impl From<crate::HandsfreeMode> for ipc::HandsfreeMode {
    fn from(mode: crate::HandsfreeMode) -> Self {
        match mode {
            crate::HandsfreeMode::None => Self::None,
            crate::HandsfreeMode::Hmd => Self::Hmd,
            crate::HandsfreeMode::HmdPinch => Self::HmdPinch,
            crate::HandsfreeMode::EyeTracking => Self::EyeTracking,
            crate::HandsfreeMode::EyeTrackingPinch => Self::EyeTrackingPinch,
        }
    }
}

impl From<crate::HandsfreeAction> for ipc::HandsfreeAction {
    fn from(action: crate::HandsfreeAction) -> Self {
        match action {
            crate::HandsfreeAction::Click => Self::Click,
            crate::HandsfreeAction::Grab => Self::Grab,
            crate::HandsfreeAction::RightModifier => Self::RightModifier,
            crate::HandsfreeAction::MiddleModifier => Self::MiddleModifier,
        }
    }
}

impl From<crate::WindowStateField> for ipc::WlxWindowStateField {
    fn from(field: crate::WindowStateField) -> Self {
        match field {
            crate::WindowStateField::Alpha => Self::Alpha,
            crate::WindowStateField::Grabbable => Self::Grabbable,
            crate::WindowStateField::Interactable => Self::Interactable,
            crate::WindowStateField::Positioning => Self::Positioning,
            crate::WindowStateField::Curvature => Self::Curvature,
            crate::WindowStateField::Additive => Self::Additive,
            crate::WindowStateField::BlockInput => Self::BlockInput,
            crate::WindowStateField::AlignToHmd => Self::AlignToHmd,
            crate::WindowStateField::Global => Self::Global,
        }
    }
}

impl From<crate::WindowAttrib> for ipc::WlxWindowAttrib {
    fn from(attrib: crate::WindowAttrib) -> Self {
        match attrib {
            crate::WindowAttrib::Stereo => Self::Stereo,
            crate::WindowAttrib::StereoFullFrame => Self::StereoFullFrame,
            crate::WindowAttrib::StereoAdjustMouse => Self::StereoAdjustMouse,
            crate::WindowAttrib::MouseTransform => Self::MouseTransform,
            crate::WindowAttrib::WindowSize => Self::WindowSize,
        }
    }
}

impl From<crate::SubcommandHandsfree> for ipc::HandsfreeParams {
    fn from(cmd: crate::SubcommandHandsfree) -> Self {
        match cmd {
            crate::SubcommandHandsfree::SetMode { mode } => Self::SetMode(mode.into()),
            crate::SubcommandHandsfree::Press { action } => Self::Press(action.into()),
            crate::SubcommandHandsfree::Release { action } => Self::Release(action.into()),
            crate::SubcommandHandsfree::Toggle { action } => Self::Toggle(action.into()),
            crate::SubcommandHandsfree::Scroll { amount } => Self::Scroll(amount),
        }
    }
}
