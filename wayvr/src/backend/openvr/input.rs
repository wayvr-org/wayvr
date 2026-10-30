use std::{array, fs::File, io::Write, time::Duration};

use anyhow::bail;
use glam::{Affine3A, FloatExt, Quat, Vec3};
use ovr_overlay::{
    TrackedDeviceIndex,
    input::{ActionHandle, ActionSetHandle, ActiveActionSet, InputManager, InputValueHandle},
    overlay::OverlayManager,
    sys::{
        ETrackedControllerRole, ETrackedDeviceClass, ETrackedDeviceProperty,
        ETrackingUniverseOrigin,
    },
    system::SystemManager,
};
use wlx_common::{config::HandsfreePointer, config_io};

use crate::{
    backend::input::{Haptics, Pointer, PointerState, TrackedDevice, TrackedDeviceRole},
    state::AppState,
};

use super::helpers::{Affine3AConvert, OVRError};

const SET_DEFAULT: &str = "/actions/default";
const INPUT_SOURCES: [&str; 2] = ["/user/hand/left", "/user/hand/right"];
const INPUT_HEAD: &str = "/user/head";
const PATH_POSES: [&str; 2] = [
    "/actions/default/in/LeftHand",
    "/actions/default/in/RightHand",
];
const PATH_HAPTICS: [&str; 2] = [
    "/actions/default/out/HapticsLeft",
    "/actions/default/out/HapticsRight",
];

const PATH_ALT_CLICK: &str = "/actions/default/in/AltClick";
const PATH_EYE_TRACKING: &str = "/actions/default/in/EyeTracking";
const PATH_CLICK_MODIFIER_MIDDLE: &str = "/actions/default/in/ClickModifierMiddle";
const PATH_CLICK_MODIFIER_RIGHT: &str = "/actions/default/in/ClickModifierRight";
const PATH_CLICK: &str = "/actions/default/in/Click";
const PATH_GRAB: &str = "/actions/default/in/Grab";
const PATH_MOVE_MOUSE: &str = "/actions/default/in/MoveMouse";
const PATH_SCROLL: &str = "/actions/default/in/Scroll";
const PATH_SHOW_HIDE: &str = "/actions/default/in/ShowHide";
const PATH_SPACE_DRAG: &str = "/actions/default/in/SpaceDrag";
const PATH_SPACE_ROTATE: &str = "/actions/default/in/SpaceRotate";
const PATH_SPACE_RESET: &str = "/actions/default/in/SpaceReset";
const PATH_TOGGLE_DASHBOARD: &str = "/actions/default/in/ToggleDashboard";

const INPUT_ANY: InputValueHandle = InputValueHandle(ovr_overlay::sys::k_ulInvalidInputValueHandle);

pub(super) struct OpenVrInputSource {
    hands: [OpenVrHandSource; 2],
    head_hnd: InputValueHandle,
    set_hnd: ActionSetHandle,
    eye_tracking_hnd: ActionHandle,
    click_hnd: ActionHandle,
    grab_hnd: ActionHandle,
    scroll_hnd: ActionHandle,
    alt_click_hnd: ActionHandle,
    show_hide_hnd: ActionHandle,
    toggle_dashboard_hnd: ActionHandle,
    space_drag_hnd: ActionHandle,
    space_rotate_hnd: ActionHandle,
    space_reset_hnd: ActionHandle,
    click_modifier_right_hnd: ActionHandle,
    click_modifier_middle_hnd: ActionHandle,
    move_mouse_hnd: ActionHandle,
}

pub(super) struct OpenVrHandSource {
    has_pose: bool,
    device: Option<TrackedDeviceIndex>,
    input_hnd: InputValueHandle,
    pose_hnd: ActionHandle,
    haptics_hnd: ActionHandle,
}

impl OpenVrInputSource {
    pub fn new(input: &mut InputManager) -> Result<Self, OVRError> {
        let set_hnd = input.get_action_set_handle(SET_DEFAULT)?;
        let head_hnd = input.get_input_source_handle(INPUT_HEAD)?;

        let eye_tracking_hnd = input.get_action_handle(PATH_EYE_TRACKING)?;
        let click_hnd = input.get_action_handle(PATH_CLICK)?;
        let grab_hnd = input.get_action_handle(PATH_GRAB)?;
        let scroll_hnd = input.get_action_handle(PATH_SCROLL)?;
        let alt_click_hnd = input.get_action_handle(PATH_ALT_CLICK)?;
        let show_hide_hnd = input.get_action_handle(PATH_SHOW_HIDE)?;
        let toggle_dashboard_hnd = input.get_action_handle(PATH_TOGGLE_DASHBOARD)?;
        let space_drag_hnd = input.get_action_handle(PATH_SPACE_DRAG)?;
        let space_rotate_hnd = input.get_action_handle(PATH_SPACE_ROTATE)?;
        let space_reset_hnd = input.get_action_handle(PATH_SPACE_RESET)?;
        let click_modifier_right_hnd = input.get_action_handle(PATH_CLICK_MODIFIER_RIGHT)?;
        let click_modifier_middle_hnd = input.get_action_handle(PATH_CLICK_MODIFIER_MIDDLE)?;
        let move_mouse_hnd = input.get_action_handle(PATH_MOVE_MOUSE)?;

        let input_hnd: Vec<InputValueHandle> = INPUT_SOURCES
            .iter()
            .map(|path| Ok((input.get_input_source_handle(path))?))
            .collect::<Result<_, OVRError>>()?;

        let pose_hnd: Vec<ActionHandle> = PATH_POSES
            .iter()
            .map(|path| Ok((input.get_action_handle(path))?))
            .collect::<Result<_, OVRError>>()?;

        let haptics_hnd: Vec<ActionHandle> = PATH_HAPTICS
            .iter()
            .map(|path| Ok((input.get_action_handle(path))?))
            .collect::<Result<_, OVRError>>()?;

        let hands: [OpenVrHandSource; 2] = array::from_fn(|i| OpenVrHandSource {
            has_pose: false,
            device: None,
            input_hnd: input_hnd[i],
            pose_hnd: pose_hnd[i],
            haptics_hnd: haptics_hnd[i],
        });

        Ok(Self {
            hands,
            head_hnd,
            set_hnd,
            eye_tracking_hnd,
            click_hnd,
            grab_hnd,
            scroll_hnd,
            alt_click_hnd,
            show_hide_hnd,
            toggle_dashboard_hnd,
            space_drag_hnd,
            space_rotate_hnd,
            space_reset_hnd,
            click_modifier_right_hnd,
            click_modifier_middle_hnd,
            move_mouse_hnd,
        })
    }

    pub fn open_bindings(&mut self, input: &mut InputManager) {
        let _ = input.open_binding_ui(None, None, self.hands[0].input_hnd, false);
    }

    pub fn haptics(&mut self, input: &mut InputManager, hand: usize, haptics: &Haptics) {
        let action_handle = self.hands[hand].haptics_hnd;
        let _ = input.trigger_haptic_vibration_action(
            action_handle,
            0.0,
            Duration::from_secs_f32(haptics.duration),
            haptics.frequency,
            haptics.intensity,
            INPUT_ANY,
        );
    }

    pub fn update(
        &mut self,
        universe: ETrackingUniverseOrigin,
        input: &mut InputManager,
        overlay: &mut OverlayManager,
        system: &mut SystemManager,
        app: &mut AppState,
    ) {
        let should_block_input_left = app.input_state.pointers[0].interaction.should_block_input
            && app.session.config.block_game_input;

        let should_block_input_right = app.input_state.pointers[1].interaction.should_block_input
            && app.session.config.block_game_input;

        let aas_left = ActiveActionSet(ovr_overlay::sys::VRActiveActionSet_t {
            ulActionSet: self.set_hnd.0,
            ulRestrictedToDevice: self.hands[0].input_hnd.0,
            ulSecondaryActionSet: 0,
            unPadding: 0,
            // the range between 0x01000000 and 0x01FFFFFF overrides game action sets as long as
            // global input from overlays is enabled in SteamVR developer settings
            // (taken from https://github.com/ValveSoftware/openvr/issues/1236)
            nPriority: if should_block_input_left {
                0x0100_0000
            } else {
                0x0
            },
        });

        let aas_right = ActiveActionSet(ovr_overlay::sys::VRActiveActionSet_t {
            ulActionSet: self.set_hnd.0,
            ulRestrictedToDevice: self.hands[1].input_hnd.0,
            ulSecondaryActionSet: 0,
            unPadding: 0,
            nPriority: if should_block_input_right {
                0x0100_0000
            } else {
                0x0
            },
        });

        let aas_head = ActiveActionSet(ovr_overlay::sys::VRActiveActionSet_t {
            ulActionSet: self.set_hnd.0,
            ulRestrictedToDevice: self.head_hnd.0,
            ulSecondaryActionSet: 0,
            unPadding: 0,
            nPriority: 0x0,
        });

        let _ = input.update_actions(&mut [aas_left, aas_right, aas_head]);

        let eye_gaze = input
            .get_eye_tracking_data_relative_to_now(self.eye_tracking_hnd, universe.clone(), 0.005)
            .ok()
            .and_then(|data| eye_tracking_pose(&data.0));
        app.input_state.eye_gaze = eye_gaze;

        let devices = system.get_device_to_absolute_tracking_pose(universe.clone(), 0.005);
        let hmd = devices[0].mDeviceToAbsoluteTracking.to_affine();
        let hmd_tracked = devices[0].bPoseIsValid;
        if hmd_tracked {
            app.input_state.hmd = hmd;
        }

        let picking_focus = app.input_state.picking_focus;
        let mut any_tracked = false;

        let delta_time = app.delta_time;
        let pointer_lerp_factor = app.session.config.pointer_lerp_factor;

        if picking_focus.is_none() {
            let should_disable_lerp = app.input_state.should_disable_lerp();

            for i in 0..2 {
                let hand = &mut self.hands[i];
                let app_hand = &mut app.input_state.pointers[i];
                app_hand.handsfree = false;

                if let Some(device) = hand.device.filter(|_| !overlay.is_dashboard_visible()) {
                    app_hand.raw_pose = devices[device.0 as usize]
                        .mDeviceToAbsoluteTracking
                        .to_affine();
                    app_hand.tracked = devices[device.0 as usize].bPoseIsValid;
                } else {
                    app_hand.tracked = false;
                }
                any_tracked |= app_hand.tracked;

                hand.has_pose = false;

                let _ = input
                    .get_pose_action_data_relative_to_now(
                        hand.pose_hnd,
                        universe.clone(),
                        0.005,
                        INPUT_ANY,
                    )
                    .map(|pose| {
                        let new_pose = pose.0.pose.mDeviceToAbsoluteTracking.to_affine();
                        let lerp_factor = (!should_disable_lerp)
                            .then(|| (delta_time * 100.0 * pointer_lerp_factor).clamp(0.1, 1.0));
                        Self::pointer_load_pose(app_hand, new_pose, lerp_factor);
                        hand.has_pose = true;
                    });

                app_hand.now.click = input
                    .get_digital_action_data(self.click_hnd, hand.input_hnd)
                    .is_ok_and(|x| x.0.bState);

                app_hand.now.grab = input
                    .get_digital_action_data(self.grab_hnd, hand.input_hnd)
                    .is_ok_and(|x| x.0.bState);

                app_hand.now.alt_click = input
                    .get_digital_action_data(self.alt_click_hnd, hand.input_hnd)
                    .is_ok_and(|x| x.0.bState);

                app_hand.now.show_hide = input
                    .get_digital_action_data(self.show_hide_hnd, hand.input_hnd)
                    .is_ok_and(|x| x.0.bState);

                app_hand.now.toggle_dashboard = input
                    .get_digital_action_data(self.toggle_dashboard_hnd, hand.input_hnd)
                    .is_ok_and(|x| x.0.bState);

                app_hand.now.space_drag = input
                    .get_digital_action_data(self.space_drag_hnd, hand.input_hnd)
                    .is_ok_and(|x| x.0.bState);

                app_hand.now.space_rotate = input
                    .get_digital_action_data(self.space_rotate_hnd, hand.input_hnd)
                    .is_ok_and(|x| x.0.bState);

                app_hand.now.space_reset = input
                    .get_digital_action_data(self.space_reset_hnd, hand.input_hnd)
                    .is_ok_and(|x| x.0.bState);

                app_hand.now.click_modifier_right = input
                    .get_digital_action_data(self.click_modifier_right_hnd, hand.input_hnd)
                    .is_ok_and(|x| x.0.bState);

                app_hand.now.click_modifier_middle = input
                    .get_digital_action_data(self.click_modifier_middle_hnd, hand.input_hnd)
                    .is_ok_and(|x| x.0.bState);

                app_hand.now.move_mouse = input
                    .get_digital_action_data(self.move_mouse_hnd, hand.input_hnd)
                    .is_ok_and(|x| x.0.bState);

                let scroll = input
                    .get_analog_action_data(self.scroll_hnd, hand.input_hnd)
                    .map_or((0.0, 0.0), |x| (x.0.x, x.0.y));
                app_hand.now.scroll_x = scroll.0;
                app_hand.now.scroll_y = scroll.1;
            }
        } else {
            app.input_state.handsfree_state.scroll_x =
                app.input_state.handsfree_state.scroll_x.lerp(0.0, 0.7);
            app.input_state.handsfree_state.scroll_y =
                app.input_state.handsfree_state.scroll_y.lerp(0.0, 0.7);

            for pointer in &mut app.input_state.pointers {
                pointer.before = pointer.now;
                pointer.now = PointerState::default();
                pointer.tracked = false;
                pointer.handsfree = false;
            }
        }

        if !picking_focus.is_none() || !any_tracked {
            let mode: HandsfreePointer = if picking_focus.is_none() {
                app.session.config.handsfree_pointer
            } else {
                app.session.config.handsfree_alt_tab.into()
            };
            let handsfree_state = app.input_state.handsfree_state;

            Self::update_handsfree(
                &mut app.input_state.pointers[0],
                mode,
                hmd,
                hmd_tracked,
                eye_gaze,
                handsfree_state,
                delta_time,
                pointer_lerp_factor,
            );
        }
    }

    fn pointer_load_pose(pointer: &mut Pointer, new_pose: Affine3A, lerp_factor: Option<f32>) {
        if let Some(lerp_factor) = lerp_factor {
            let cur_quat = Quat::from_affine3(&pointer.pose);
            let cur_pos = Vec3::from(pointer.pose.translation);
            let new_quat = Quat::from_affine3(&new_pose);
            let new_pos = Vec3::from(new_pose.translation);
            pointer.pose = Affine3A::from_rotation_translation(
                cur_quat.lerp(new_quat, lerp_factor),
                cur_pos.lerp(new_pos, lerp_factor),
            );
        } else {
            pointer.pose = new_pose; // no lerp
        }
    }

    fn update_handsfree(
        pointer: &mut Pointer,
        mode: HandsfreePointer,
        hmd: Affine3A,
        hmd_tracked: bool,
        eye_gaze: Option<Affine3A>,
        handsfree_state: PointerState,
        delta_time: f32,
        pointer_lerp_factor: f32,
    ) {
        let (raw_pose, tracked, lerp_scale) = match mode {
            HandsfreePointer::None => return,
            HandsfreePointer::Hmd | HandsfreePointer::HmdOnly => (hmd, hmd_tracked, 1.0),
            HandsfreePointer::EyeTracking | HandsfreePointer::EyeTrackingOnly => {
                let Some(gaze) = eye_gaze else {
                    pointer.tracked = false;
                    pointer.handsfree = false;
                    pointer.now = handsfree_pointer_state(handsfree_state);
                    return;
                };
                // Match the OpenXR backend's stronger smoothing for eye gaze.
                (gaze, true, 0.5)
            }
        };

        let lerp_factor = (delta_time * 100.0 * pointer_lerp_factor * lerp_scale).clamp(0.1, 1.0);

        pointer.raw_pose = raw_pose;
        Self::pointer_load_pose(pointer, raw_pose, Some(lerp_factor));
        pointer.tracked = tracked;
        pointer.handsfree = tracked;
        pointer.now = handsfree_pointer_state(handsfree_state);
    }

    pub fn update_devices(&mut self, system: &mut SystemManager, app: &mut AppState) -> bool {
        let old_len = app.input_state.devices.len();
        app.input_state.devices.clear();
        for idx in 0..TrackedDeviceIndex::MAX {
            let device = TrackedDeviceIndex::new(idx as _).unwrap(); // safe
            if !system.is_tracked_device_connected(device) {
                continue;
            }

            let class = system.get_tracked_device_class(device);

            let role = match class {
                ETrackedDeviceClass::TrackedDeviceClass_HMD => TrackedDeviceRole::Hmd,
                ETrackedDeviceClass::TrackedDeviceClass_Controller => {
                    let role = system.get_controller_role_for_tracked_device_index(device);
                    match role {
                        ETrackedControllerRole::TrackedControllerRole_LeftHand => {
                            self.hands[0].device = Some(device);
                            TrackedDeviceRole::LeftHand
                        }
                        ETrackedControllerRole::TrackedControllerRole_RightHand => {
                            self.hands[1].device = Some(device);
                            TrackedDeviceRole::RightHand
                        }
                        _ => continue,
                    }
                }
                ETrackedDeviceClass::TrackedDeviceClass_GenericTracker => {
                    TrackedDeviceRole::Tracker
                }
                _ => continue,
            };

            if let Some(device) = get_tracked_device(system, device, role) {
                app.input_state.devices.push(device);
            }
        }

        app.input_state.devices.sort_by(|a, b| {
            u8::from(a.soc.is_none())
                .cmp(&u8::from(b.soc.is_none()))
                .then((a.role as u8).cmp(&(b.role as u8)))
                .then(a.soc.unwrap_or(999.).total_cmp(&b.soc.unwrap_or(999.)))
        });

        old_len != app.input_state.devices.len()
    }
}

fn handsfree_pointer_state(state: PointerState) -> PointerState {
    PointerState {
        click: state.click,
        grab: state.grab,
        grab_float: state.grab_float,
        click_modifier_right: state.click_modifier_right,
        click_modifier_middle: state.click_modifier_middle,
        scroll_x: state.scroll_x,
        scroll_y: state.scroll_y,
        ..PointerState::default()
    }
}

fn eye_tracking_pose(data: &ovr_overlay::sys::VREyeTrackingData_t) -> Option<Affine3A> {
    if !(data.bActive && data.bValid && data.bTracked) {
        return None;
    }

    let origin = Vec3::new(
        data.vGazeOrigin.v[0],
        data.vGazeOrigin.v[1],
        data.vGazeOrigin.v[2],
    );
    let target = Vec3::new(
        data.vGazeTarget.v[0],
        data.vGazeTarget.v[1],
        data.vGazeTarget.v[2],
    );
    if !origin.is_finite() || !target.is_finite() {
        return None;
    }

    let direction = target - origin;
    if !direction.is_finite() || direction.length_squared() <= f32::EPSILON {
        return None;
    }

    Some(Affine3A::from_rotation_translation(
        Quat::from_rotation_arc(Vec3::NEG_Z, direction.normalize()),
        origin,
    ))
}

fn get_tracked_device(
    system: &mut SystemManager,
    index: TrackedDeviceIndex,
    role: TrackedDeviceRole,
) -> Option<TrackedDevice> {
    let provides_battery = system
        .get_tracked_device_property(
            index,
            ETrackedDeviceProperty::Prop_DeviceProvidesBatteryStatus_Bool,
        )
        .unwrap_or(false);
    if !provides_battery {
        // don't show devices that don't have battery info
        return None;
    }

    let soc = system
        .get_tracked_device_property(
            index,
            ETrackedDeviceProperty::Prop_DeviceBatteryPercentage_Float,
        )
        .ok();

    let charging = if soc.is_some() {
        system
            .get_tracked_device_property(index, ETrackedDeviceProperty::Prop_DeviceIsCharging_Bool)
            .unwrap_or(false)
    } else {
        false
    };

    // TODO: cache this
    let is_alvr = system
        .get_tracked_device_property(
            index,
            ETrackedDeviceProperty::Prop_TrackingSystemName_String,
        )
        .is_ok_and(|x: String| x.contains("ALVR"));

    if is_alvr {
        // don't show ALVR's fake trackers on battery panel
        return None;
    }

    Some(TrackedDevice {
        soc,
        charging,
        role,
    })
}

pub fn set_action_manifest(input: &mut InputManager) -> anyhow::Result<()> {
    let action_path = config_io::get_config_root().join("actions.json");

    if let Err(e) = File::create(&action_path)
        .and_then(|mut f| f.write_all(include_bytes!("../../res/actions.json")))
    {
        log::warn!("Could not write action manifest: {e}");
    }

    let binding_path = config_io::get_config_root().join("actions_binding_knuckles.json");
    if !binding_path.is_file() {
        File::create(&binding_path)?
            .write_all(include_bytes!("../../res/actions_binding_knuckles.json"))?;
    }

    let binding_path = config_io::get_config_root().join("actions_binding_vive.json");
    if !binding_path.is_file() {
        File::create(&binding_path)?
            .write_all(include_bytes!("../../res/actions_binding_vive.json"))?;
    }

    let binding_path = config_io::get_config_root().join("actions_binding_oculus.json");
    if !binding_path.is_file() {
        File::create(&binding_path)?
            .write_all(include_bytes!("../../res/actions_binding_oculus.json"))?;
    }

    let binding_path = config_io::get_config_root().join("actions_binding_frame.json");
    if !binding_path.is_file() {
        File::create(&binding_path)?
            .write_all(include_bytes!("../../res/actions_binding_frame.json"))?;
    }

    let binding_path = config_io::get_config_root().join("actions_binding_generic_hmd.json");
    if !binding_path.is_file() {
        File::create(&binding_path)?
            .write_all(include_bytes!("../../res/actions_binding_generic_hmd.json"))?;
    }

    if let Err(e) = input.set_action_manifest(action_path.as_path()) {
        bail!("Failed to set action manifest: {e}");
    }
    Ok(())
}
