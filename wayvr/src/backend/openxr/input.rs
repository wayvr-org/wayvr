use std::{
    array::from_fn,
    mem::transmute,
    time::{Duration, Instant},
};

use glam::{Affine3A, Quat, Vec3, bool};
use libmonado::{self as mnd, DeviceLogic};
use openxr::{self as xr, Quaternionf, Vector2f, Vector3f};
use serde::{Deserialize, Serialize};
use wlx_common::{config::HandsfreePointer, config_io};

use crate::{
    backend::input::{Haptics, InputState, Pointer, TrackedDevice, TrackedDeviceRole},
    state::{AppSession, AppState},
};

use super::{XrState, helpers::posef_to_transform};

static CLICK_TIMES: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_millis(500),
    Duration::from_millis(750),
];

pub(super) struct OpenXrInputSource {
    action_set: xr::ActionSet,
    pointers: [OpenXrPointer; 2],
    handsfree_pointer: OpenXrPointer,
}

pub(super) struct OpenXrPointer {
    source: OpenXrHandSource,
    space: xr::Space,
}

pub struct MultiClickHandler<const COUNT: usize> {
    name: String,
    action_f32: xr::Action<f32>,
    action_bool: xr::Action<bool>,
    previous: [Instant; COUNT],
    held_active: bool,
    held_inactive: bool,
}

impl<const COUNT: usize> MultiClickHandler<COUNT> {
    fn new(action_set: &xr::ActionSet, action_name: &str, side: &str) -> anyhow::Result<Self> {
        let name = format!("{side}_{COUNT}-{action_name}");
        let name_f32 = format!("{}_value", &name);

        let action_bool = action_set.create_action::<bool>(&name, &name, &[])?;
        let action_f32 = action_set.create_action::<f32>(&name_f32, &name_f32, &[])?;

        Ok(Self {
            name,
            action_f32,
            action_bool,
            previous: from_fn(|_| Instant::now()),
            held_active: false,
            held_inactive: false,
        })
    }
    fn check<G>(&mut self, session: &xr::Session<G>, threshold: f32) -> anyhow::Result<bool> {
        let res = self.action_bool.state(session, xr::Path::NULL)?;
        let mut state = res.is_active && res.current_state;

        if !state {
            let res = self.action_f32.state(session, xr::Path::NULL)?;
            state = res.is_active && res.current_state >= threshold - 0.001;
        }

        if !state {
            self.held_active = false;
            self.held_inactive = false;
            return Ok(false);
        }

        if self.held_active {
            return Ok(true);
        }

        if self.held_inactive {
            return Ok(false);
        }

        let passed = self
            .previous
            .iter()
            .all(|instant| instant.elapsed() < CLICK_TIMES[COUNT]);

        if passed {
            log::trace!("{}: passed", self.name);
            self.held_active = true;
            self.held_inactive = false;

            // reset to no prior clicks
            let long_ago = Instant::now().checked_sub(Duration::from_secs(10)).unwrap();
            self.previous
                .iter_mut()
                .for_each(|instant| *instant = long_ago);
        } else if COUNT > 0 {
            log::trace!("{}: rotate", self.name);
            self.previous.rotate_right(1);
            self.previous[0] = Instant::now();
            self.held_inactive = true;
        }

        Ok(passed)
    }
}

pub struct CustomClickAction {
    single: MultiClickHandler<0>,
    double: MultiClickHandler<1>,
    triple: MultiClickHandler<2>,
}

impl CustomClickAction {
    pub fn new(action_set: &xr::ActionSet, name: &str, side: &str) -> anyhow::Result<Self> {
        let single = MultiClickHandler::new(action_set, name, side)?;
        let double = MultiClickHandler::new(action_set, name, side)?;
        let triple = MultiClickHandler::new(action_set, name, side)?;

        Ok(Self {
            single,
            double,
            triple,
        })
    }
    pub fn state(
        &mut self,
        before: bool,
        state: &XrState,
        session: &AppSession,
    ) -> anyhow::Result<bool> {
        let threshold = if before {
            session.config.xr_click_sensitivity_release
        } else {
            session.config.xr_click_sensitivity
        };

        Ok(self.single.check(&state.session, threshold)?
            || self.double.check(&state.session, threshold)?
            || self.triple.check(&state.session, threshold)?)
    }
}

pub(super) struct OpenXrHandSource {
    pose: xr::Action<xr::Posef>,
    click: CustomClickAction,
    grab: CustomClickAction,
    alt_click: CustomClickAction,
    show_hide: CustomClickAction,
    toggle_dashboard: CustomClickAction,
    space_drag: CustomClickAction,
    space_rotate: CustomClickAction,
    space_reset: CustomClickAction,
    modifier_right: CustomClickAction,
    modifier_middle: CustomClickAction,
    move_mouse: CustomClickAction,
    scroll: xr::Action<Vector2f>,
    haptics: xr::Action<xr::Haptic>,
}

impl OpenXrInputSource {
    pub fn new(xr: &XrState) -> anyhow::Result<Self> {
        let mut action_set =
            xr.session
                .instance()
                .create_action_set("wayvr", "WayVR Actions", 0)?;

        let left_source = OpenXrHandSource::new(&mut action_set, "left")?;
        let right_source = OpenXrHandSource::new(&mut action_set, "right")?;
        let fallback_source = OpenXrHandSource::new(&mut action_set, "handsfree")?;

        suggest_bindings(
            &xr.instance,
            &[&left_source, &right_source, &fallback_source],
        );

        xr.session.attach_action_sets(&[&action_set])?;

        Ok(Self {
            action_set,
            pointers: [
                OpenXrPointer::new(xr, left_source)?,
                OpenXrPointer::new(xr, right_source)?,
            ],
            handsfree_pointer: OpenXrPointer::new(xr, fallback_source)?,
        })
    }

    pub fn haptics(&self, xr: &XrState, hand: usize, haptics: &Haptics) {
        let action = &self.pointers[hand].source.haptics;

        let duration_nanos = f64::from(haptics.duration) * 1_000_000_000.0;

        let _ = action.apply_feedback(
            &xr.session,
            xr::Path::NULL,
            &xr::HapticVibration::new()
                .amplitude(haptics.intensity)
                .frequency(haptics.frequency)
                .duration(xr::Duration::from_nanos(duration_nanos as _)),
        );
    }

    pub fn update(&mut self, xr: &XrState, state: &mut AppState) -> anyhow::Result<()> {
        xr.session.sync_actions(&[(&self.action_set).into()])?;

        let loc = xr.view.locate(&xr.stage, xr.predicted_display_time)?;
        let hmd = posef_to_transform(&loc.pose);
        let mut hmd_tracked = true;
        if loc
            .location_flags
            .contains(xr::SpaceLocationFlags::ORIENTATION_VALID)
        {
            state.input_state.hmd.matrix3 = hmd.matrix3;
        } else {
            hmd_tracked = false;
        }

        if loc
            .location_flags
            .contains(xr::SpaceLocationFlags::POSITION_VALID)
        {
            state.input_state.hmd.translation = hmd.translation;
        } else {
            hmd_tracked = false;
        }

        let mut any_tracked = false;
        for i in 0..2 {
            let pointer = &mut state.input_state.pointers[i];
            self.pointers[i].update(pointer, xr, &state.session)?;
            any_tracked |= pointer.tracked;
        }
        if !any_tracked {
            self.handsfree_pointer.update_handsfree(
                &mut state.input_state.pointers[0],
                xr,
                &state.session,
                hmd,
                hmd_tracked,
            )?;
        }

        Ok(())
    }

    fn update_device_battery_status(
        device: &mut mnd::Device,
        role: TrackedDeviceRole,
        input_state: &mut InputState,
    ) {
        if let Ok(status) = device.battery_status()
            && status.present
        {
            input_state.devices.push(TrackedDevice {
                soc: Some(status.charge),
                charging: status.charging,
                role,
            });
            log::debug!(
                "Device {} role {:#?}: {:.0}% (charging {})",
                device.index,
                role,
                status.charge * 100.0f32,
                status.charging
            );
        }
    }

    pub fn update_devices(app: &mut AppState) -> bool {
        let Some(monado) = &mut app.monado_state else {
            return false; // monado not available
        };

        let old_len = app.input_state.devices.len();
        app.input_state.devices.clear();

        let roles = [
            (mnd::DeviceRole::Head, TrackedDeviceRole::Hmd),
            (mnd::DeviceRole::Eyes, TrackedDeviceRole::None),
            (mnd::DeviceRole::Left, TrackedDeviceRole::LeftHand),
            (mnd::DeviceRole::Right, TrackedDeviceRole::RightHand),
            (mnd::DeviceRole::Gamepad, TrackedDeviceRole::None),
            (
                mnd::DeviceRole::HandTrackingLeft,
                TrackedDeviceRole::LeftHand,
            ),
            (
                mnd::DeviceRole::HandTrackingRight,
                TrackedDeviceRole::RightHand,
            ),
        ];
        let mut seen = Vec::<u32>::with_capacity(32);

        for (mnd_role, wlx_role) in roles {
            let device = monado.ipc.device_from_role(mnd_role);
            if let Ok(mut device) = device
                && !seen.contains(&device.index)
            {
                seen.push(device.index);
                Self::update_device_battery_status(&mut device, wlx_role, &mut app.input_state);
            }
        }
        if let Ok(devices) = monado.ipc.devices() {
            for mut device in devices {
                if !seen.contains(&device.index) {
                    let role = if device.name_id >= 4 && device.name_id <= 8 {
                        TrackedDeviceRole::Tracker
                    } else {
                        TrackedDeviceRole::None
                    };
                    Self::update_device_battery_status(&mut device, role, &mut app.input_state);
                }
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

impl OpenXrPointer {
    pub(super) fn new(xr: &XrState, source: OpenXrHandSource) -> Result<Self, xr::sys::Result> {
        let space = source
            .pose
            .create_space(&xr.session, xr::Path::NULL, xr::Posef::IDENTITY)?;

        Ok(Self { source, space })
    }

    pub(super) fn update_handsfree(
        &mut self,
        pointer: &mut Pointer,
        xr: &XrState,
        session: &AppSession,
        hmd: Affine3A,
        hmd_tracked: bool,
    ) -> anyhow::Result<()> {
        match session.config.handsfree_pointer {
            HandsfreePointer::None => return Ok(()),
            HandsfreePointer::Hmd | HandsfreePointer::HmdOnly => {
                pointer.tracked = hmd_tracked;
                pointer.raw_pose = hmd;
                pointer.pose = hmd;
                let (cur_quat, cur_pos) =
                    (Quat::from_affine3(&pointer.pose), pointer.pose.translation);

                let (new_quat, new_pos) = (Quat::from_affine3(&hmd), Vec3::from(hmd.translation));
                let lerp_factor =
                    (1.0 / (xr.fps / 100.0) * session.config.pointer_lerp_factor).clamp(0.1, 1.0);
                pointer.raw_pose = Affine3A::from_rotation_translation(new_quat, new_pos);
                pointer.pose = Affine3A::from_rotation_translation(
                    cur_quat.lerp(new_quat, lerp_factor),
                    cur_pos.lerp(new_pos.into(), lerp_factor).into(),
                );
            }
            HandsfreePointer::EyeTracking | HandsfreePointer::EyeTrackingOnly => {
                // more aggressive smoothing for eye
                self.pointer_load_pose(pointer, xr, session.config.pointer_lerp_factor * 0.5)?;
            }
        }

        pointer.handsfree = pointer.tracked;
        if matches!(
            session.config.handsfree_pointer,
            HandsfreePointer::HmdOnly | HandsfreePointer::EyeTrackingOnly
        ) {
            // skip actions
            return Ok(());
        }

        self.pointer_load_actions(pointer, xr, session)?;

        Ok(())
    }

    pub(super) fn update(
        &mut self,
        pointer: &mut Pointer,
        xr: &XrState,
        session: &AppSession,
    ) -> anyhow::Result<()> {
        pointer.handsfree = false;
        self.pointer_load_pose(pointer, xr, session.config.pointer_lerp_factor)?;
        self.pointer_load_actions(pointer, xr, session)?;

        Ok(())
    }

    fn pointer_load_pose(
        &mut self,
        pointer: &mut Pointer,
        xr: &XrState,
        lerp_factor: f32,
    ) -> anyhow::Result<()> {
        let location = self.space.locate(&xr.stage, xr.predicted_display_time)?;
        if location
            .location_flags
            .contains(xr::SpaceLocationFlags::ORIENTATION_VALID)
        {
            let (cur_quat, cur_pos) = (Quat::from_affine3(&pointer.pose), pointer.pose.translation);

            let (new_quat, new_pos) = unsafe {
                (
                    transmute::<Quaternionf, Quat>(location.pose.orientation),
                    transmute::<Vector3f, Vec3>(location.pose.position),
                )
            };
            let lerp_factor = (1.0 / (xr.fps / 100.0) * lerp_factor).clamp(0.1, 1.0);
            pointer.raw_pose = Affine3A::from_rotation_translation(new_quat, new_pos);
            pointer.pose = Affine3A::from_rotation_translation(
                cur_quat.lerp(new_quat, lerp_factor),
                cur_pos.lerp(new_pos.into(), lerp_factor).into(),
            );
            pointer.tracked = true;
        } else {
            pointer.tracked = false;
        }
        Ok(())
    }

    fn pointer_load_actions(
        &mut self,
        pointer: &mut Pointer,
        xr: &XrState,
        session: &AppSession,
    ) -> anyhow::Result<()> {
        pointer.now.click = self.source.click.state(pointer.before.click, xr, session)?;

        pointer.now.grab = self.source.grab.state(pointer.before.grab, xr, session)?;

        let scroll = self
            .source
            .scroll
            .state(&xr.session, xr::Path::NULL)?
            .current_state;

        pointer.now.scroll_x = scroll.x;
        pointer.now.scroll_y = scroll.y;

        pointer.now.alt_click =
            self.source
                .alt_click
                .state(pointer.before.alt_click, xr, session)?;

        pointer.now.show_hide =
            self.source
                .show_hide
                .state(pointer.before.show_hide, xr, session)?;

        pointer.now.click_modifier_right =
            self.source
                .modifier_right
                .state(pointer.before.click_modifier_right, xr, session)?;

        pointer.now.toggle_dashboard =
            self.source
                .toggle_dashboard
                .state(pointer.before.toggle_dashboard, xr, session)?;

        pointer.now.click_modifier_middle =
            self.source
                .modifier_middle
                .state(pointer.before.click_modifier_middle, xr, session)?;

        pointer.now.move_mouse =
            self.source
                .move_mouse
                .state(pointer.before.move_mouse, xr, session)?;

        pointer.now.space_drag =
            self.source
                .space_drag
                .state(pointer.before.space_drag, xr, session)?;

        pointer.now.space_rotate =
            self.source
                .space_rotate
                .state(pointer.before.space_rotate, xr, session)?;

        pointer.now.space_reset =
            self.source
                .space_reset
                .state(pointer.before.space_reset, xr, session)?;

        Ok(())
    }
}

// supported action types: Haptic, Posef, Vector2f, f32, bool
impl OpenXrHandSource {
    pub(super) fn new(action_set: &mut xr::ActionSet, side: &str) -> anyhow::Result<Self> {
        let action_pose = action_set.create_action::<xr::Posef>(
            &format!("{side}_hand"),
            &format!("{side} hand pose"),
            &[],
        )?;

        let action_scroll = action_set.create_action::<Vector2f>(
            &format!("{side}_scroll"),
            &format!("{side} hand scroll"),
            &[],
        )?;
        let action_haptics = action_set.create_action::<xr::Haptic>(
            &format!("{side}_haptics"),
            &format!("{side} hand haptics"),
            &[],
        )?;

        Ok(Self {
            pose: action_pose,
            click: CustomClickAction::new(action_set, "click", side)?,
            grab: CustomClickAction::new(action_set, "grab", side)?,
            scroll: action_scroll,
            alt_click: CustomClickAction::new(action_set, "alt_click", side)?,
            show_hide: CustomClickAction::new(action_set, "show_hide", side)?,
            toggle_dashboard: CustomClickAction::new(action_set, "toggle_dashboard", side)?,
            space_drag: CustomClickAction::new(action_set, "space_drag", side)?,
            space_rotate: CustomClickAction::new(action_set, "space_rotate", side)?,
            space_reset: CustomClickAction::new(action_set, "space_reset", side)?,
            modifier_right: CustomClickAction::new(action_set, "click_modifier_right", side)?,
            modifier_middle: CustomClickAction::new(action_set, "click_modifier_middle", side)?,
            move_mouse: CustomClickAction::new(action_set, "move_mouse", side)?,
            haptics: action_haptics,
        })
    }
}

fn to_paths(maybe_path_str: Option<&str>, instance: &xr::Instance) -> Option<xr::Path> {
    maybe_path_str.and_then(|s| {
        instance
            .string_to_path(s)
            .inspect_err(|_| {
                log::warn!("Invalid binding path: {s}");
            })
            .ok()
    })
}

fn is_bool(path_str: &str) -> bool {
    path_str
        .split('/')
        .next_back()
        .is_some_and(|last| matches!(last, "click" | "touch") || last.starts_with("dpad_"))
}

macro_rules! add_custom {
    ($action:expr, $field:ident, $hands:expr, $bindings:expr, $instance:expr) => {
        if let Some(action) = $action.as_ref() {
            for i in 0..3 {
                let spec = match i {
                    0 => action.left.as_ref(),
                    1 => action.right.as_ref(),
                    2 => action.handsfree.as_ref(),
                    _ => unreachable!(),
                };

                if let Some(spec) = spec {
                    let iter: Box<dyn Iterator<Item = &String>> = match spec {
                        OneOrMany::One(s) => Box::new(std::iter::once(s)),
                        OneOrMany::Many(v) => Box::new(v.iter()),
                    };

                    for s in iter {
                        if let Some(p) = to_paths(Some(s.as_str()), $instance) {
                            if is_bool(s) {
                                if action.triple_click.unwrap_or(false) {
                                    $bindings.push(xr::Binding::new(
                                        &$hands[i].$field.triple.action_bool,
                                        p,
                                    ));
                                } else if action.double_click.unwrap_or(false) {
                                    $bindings.push(xr::Binding::new(
                                        &$hands[i].$field.double.action_bool,
                                        p,
                                    ));
                                } else {
                                    $bindings.push(xr::Binding::new(
                                        &$hands[i].$field.single.action_bool,
                                        p,
                                    ));
                                }
                            } else {
                                if action.triple_click.unwrap_or(false) {
                                    $bindings.push(xr::Binding::new(
                                        &$hands[i].$field.triple.action_f32,
                                        p,
                                    ));
                                } else if action.double_click.unwrap_or(false) {
                                    $bindings.push(xr::Binding::new(
                                        &$hands[i].$field.double.action_f32,
                                        p,
                                    ));
                                } else {
                                    $bindings.push(xr::Binding::new(
                                        &$hands[i].$field.single.action_f32,
                                        p,
                                    ));
                                }
                            }
                        };
                    }
                };
            }
        }
    };
}

// TODO: rename this bad func name
macro_rules! add_custom_lr {
    ($action:expr, $field:ident, $hands:expr, $bindings:expr, $instance:expr) => {
        if let Some(action) = $action {
            for i in 0..3 {
                let spec = match i {
                    0 => action.left.as_ref(),
                    1 => action.right.as_ref(),
                    2 => action.handsfree.as_ref(),
                    _ => unreachable!(),
                };

                if let Some(spec) = spec {
                    let iter: Box<dyn Iterator<Item = &String>> = match spec {
                        OneOrMany::One(s) => Box::new(std::iter::once(s)),
                        OneOrMany::Many(v) => Box::new(v.iter()),
                    };

                    for s in iter {
                        if let Some(p) = to_paths(Some(s.as_str()), $instance) {
                            $bindings.push(xr::Binding::new(&$hands[i].$field, p));
                        }
                    }
                };
            }
        };
    };
}

#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
fn suggest_bindings(instance: &xr::Instance, hands: &[&OpenXrHandSource; 3]) {
    let profiles = load_action_profiles();

    for profile in profiles {
        log::warn!("Loading profile {}", &profile.profile);

        let Ok(profile_path) = instance.string_to_path(&profile.profile) else {
            log::warn!("Profile not supported: {}", profile.profile);
            continue;
        };

        let mut bindings: Vec<xr::Binding> = vec![];

        add_custom_lr!(profile.pose, pose, hands, bindings, instance);
        add_custom_lr!(profile.haptic, haptics, hands, bindings, instance);
        add_custom_lr!(profile.scroll, scroll, hands, bindings, instance);

        add_custom!(profile.click, click, hands, bindings, instance);

        add_custom!(profile.alt_click, alt_click, hands, bindings, instance);

        add_custom!(profile.grab, grab, hands, bindings, instance);

        add_custom!(profile.show_hide, show_hide, hands, bindings, instance);

        add_custom!(
            profile.toggle_dashboard,
            toggle_dashboard,
            hands,
            bindings,
            instance
        );

        add_custom!(profile.space_drag, space_drag, hands, bindings, instance);

        add_custom!(
            profile.space_rotate,
            space_rotate,
            hands,
            bindings,
            instance
        );

        add_custom!(profile.space_reset, space_reset, hands, bindings, instance);

        add_custom!(
            profile.click_modifier_right,
            modifier_right,
            hands,
            bindings,
            instance
        );

        add_custom!(
            profile.click_modifier_middle,
            modifier_middle,
            hands,
            bindings,
            instance
        );

        add_custom!(profile.move_mouse, move_mouse, hands, bindings, instance);

        if instance
            .suggest_interaction_profile_bindings(profile_path, &bindings)
            .is_err()
        {
            log::error!("Bad bindings for {}", &profile.profile[22..]);
            log::error!("Verify config: ~/.config/wayvr/openxr_actions.json5");
        } else {
            log::debug!(
                "Bindings for {} bound successfully.",
                &profile.profile[22..]
            );
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenXrActionConfAction {
    left: Option<OneOrMany<String>>,
    right: Option<OneOrMany<String>>,
    handsfree: Option<OneOrMany<String>>,
    threshold: Option<[f32; 2]>,
    double_click: Option<bool>,
    triple_click: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenXrActionConfProfile {
    profile: String,
    pose: Option<OpenXrActionConfAction>,
    click: Option<OpenXrActionConfAction>,
    grab: Option<OpenXrActionConfAction>,
    alt_click: Option<OpenXrActionConfAction>,
    show_hide: Option<OpenXrActionConfAction>,
    toggle_dashboard: Option<OpenXrActionConfAction>,
    space_drag: Option<OpenXrActionConfAction>,
    space_rotate: Option<OpenXrActionConfAction>,
    space_reset: Option<OpenXrActionConfAction>,
    click_modifier_right: Option<OpenXrActionConfAction>,
    click_modifier_middle: Option<OpenXrActionConfAction>,
    move_mouse: Option<OpenXrActionConfAction>,
    scroll: Option<OpenXrActionConfAction>,
    haptic: Option<OpenXrActionConfAction>,
}

const DEFAULT_PROFILES: &str = include_str!("openxr_actions.json5");

fn load_action_profiles() -> Vec<OpenXrActionConfProfile> {
    let mut profiles: Vec<OpenXrActionConfProfile> =
        serde_json5::from_str(DEFAULT_PROFILES).unwrap(); // want panic

    let Some(conf) = config_io::load("openxr_actions.json5") else {
        return profiles;
    };

    match serde_json5::from_str::<Vec<OpenXrActionConfProfile>>(&conf) {
        Ok(override_profiles) => {
            for new in override_profiles {
                if let Some(i) = profiles.iter().position(|old| old.profile == new.profile) {
                    profiles[i] = new;
                } else {
                    profiles.push(new);
                }
            }
        }
        Err(e) => {
            log::error!("Failed to load openxr_actions.json5: {e}");
        }
    }

    profiles
}
