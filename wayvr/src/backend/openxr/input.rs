use std::{
    collections::HashMap,
    mem::transmute,
    time::{Duration, Instant},
};

use glam::{Affine3A, FloatExt, Quat, Vec3, bool};
use libmonado::{self as mnd, DeviceLogic};
use openxr::{self as xr, Quaternionf, Vector2f, Vector3f};
use wlx_common::{
    config::HandsfreePointer,
    openxr_actions::{
        OneOrMany, OpenXrInputAction, OpenXrInputChordMember, OpenXrInputProfile,
        load_xr_input_profiles,
    },
    openxr_bindings_schema::DEFAULT_BUTTON_THRESHOLDS,
};

use crate::{
    backend::input::{
        Haptics, InputState, Pointer, PointerState, TrackedDevice, TrackedDeviceRole,
    },
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
    physical_inputs: Vec<PhysicalInput>,
    pointers: [OpenXrPointer; 2],
    handsfree_pointer: OpenXrPointer,
}

pub(super) struct OpenXrPointer {
    source: OpenXrHandSource,
    space: xr::Space,
}

pub(super) struct PhysicalInput {
    profile_index: usize,
    path: xr::Path,
    action: PhysicalInputAction,
}

enum PhysicalInputAction {
    Bool {
        action: xr::Action<bool>,
        current: Option<bool>,
    },
    Float {
        action: xr::Action<f32>,
        current: Option<f32>,
    },
}

impl PhysicalInput {
    fn update<G>(&mut self, session: &xr::Session<G>) -> anyhow::Result<()> {
        match &mut self.action {
            PhysicalInputAction::Bool { action, current } => {
                let state = action.state(session, xr::Path::NULL)?;
                *current = state.is_active.then_some(state.current_state);
            }
            PhysicalInputAction::Float { action, current } => {
                let state = action.state(session, xr::Path::NULL)?;
                *current = state.is_active.then_some(state.current_state);
            }
        }
        Ok(())
    }

    fn pressed(&self, before: bool, threshold: [f32; 2]) -> bool {
        match &self.action {
            PhysicalInputAction::Bool { current, .. } => current.unwrap_or(false),
            PhysicalInputAction::Float { current, .. } => {
                let threshold = if before { threshold[0] } else { threshold[1] };
                current.is_some_and(|value| value >= threshold - 0.001)
            }
        }
    }
}

struct ButtonCondition {
    input: usize,
    threshold: [f32; 2],
    active: bool,
}

impl ButtonCondition {
    const fn new(input: usize, threshold: [f32; 2]) -> Self {
        Self {
            input,
            threshold,
            active: false,
        }
    }

    fn update(&mut self, physical_inputs: &[PhysicalInput]) -> bool {
        self.active = physical_inputs[self.input].pressed(self.active, self.threshold);
        self.active
    }
}

#[derive(Clone, Copy)]
enum ClickCount {
    Single,
    Double,
    Triple,
}

impl ClickCount {
    const fn previous_clicks(self) -> usize {
        match self {
            Self::Single => 0,
            Self::Double => 1,
            Self::Triple => 2,
        }
    }

    const fn timeout(self) -> Duration {
        CLICK_TIMES[self.previous_clicks()]
    }
}

struct MultiClickHandler {
    count: ClickCount,
    previous: [Option<Instant>; 2],
    held_active: bool,
    held_inactive: bool,
}

impl MultiClickHandler {
    const fn new(count: ClickCount) -> Self {
        Self {
            count,
            previous: [None, None],
            held_active: false,
            held_inactive: false,
        }
    }

    fn check(&mut self, state: bool) -> bool {
        if !state {
            self.held_active = false;
            self.held_inactive = false;
            return false;
        }

        if self.held_active {
            return true;
        }

        if self.held_inactive {
            return false;
        }

        let previous_clicks = self.count.previous_clicks();
        if previous_clicks == 0 {
            self.held_active = true;
            return true;
        }

        let now = Instant::now();
        let passed = self.previous[..previous_clicks].iter().all(|instant| {
            instant.is_some_and(|instant| now.duration_since(instant) < self.count.timeout())
        });

        if passed {
            self.held_active = true;
            self.previous = [None, None];
        } else {
            self.previous.rotate_right(1);
            self.previous[0] = Some(now);
            self.held_inactive = true;
        }

        passed
    }
}

enum ButtonConditionSet {
    Any(Vec<ButtonCondition>),
    All(Vec<ButtonCondition>),
}

struct ButtonBinding {
    conditions: ButtonConditionSet,
    clicks: MultiClickHandler,
}

impl ButtonBinding {
    fn state(&mut self, physical_inputs: &[PhysicalInput]) -> bool {
        let active = match &mut self.conditions {
            ButtonConditionSet::Any(conditions) => {
                let mut active = false;
                for condition in conditions {
                    active |= condition.update(physical_inputs);
                }
                active
            }
            ButtonConditionSet::All(conditions) => {
                let mut active = true;
                for condition in conditions {
                    active &= condition.update(physical_inputs);
                }
                active
            }
        };

        self.clicks.check(active)
    }
}

#[derive(Default)]
struct CustomClickAction {
    bindings: Vec<ButtonBinding>,
}

impl CustomClickAction {
    fn add_binding(&mut self, binding: ButtonBinding) {
        self.bindings.push(binding);
    }

    fn state(&mut self, physical_inputs: &[PhysicalInput]) -> bool {
        let mut active = false;
        for binding in &mut self.bindings {
            active |= binding.state(physical_inputs);
        }
        active
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

        let mut left_source = OpenXrHandSource::new(&mut action_set, "left")?;
        let mut right_source = OpenXrHandSource::new(&mut action_set, "right")?;
        let mut fallback_source = OpenXrHandSource::new(&mut action_set, "handsfree")?;

        let profiles = load_xr_input_profiles();
        let (physical_inputs, physical_input_map) =
            create_physical_inputs(&action_set, &xr.instance, &profiles)?;

        let mut hands: [&mut OpenXrHandSource; 3] =
            [&mut left_source, &mut right_source, &mut fallback_source];
        suggest_bindings(
            &xr.instance,
            &mut hands,
            &profiles,
            &physical_inputs,
            &physical_input_map,
        );

        xr.session.attach_action_sets(&[&action_set])?;

        Ok(Self {
            action_set,
            physical_inputs,
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

    pub fn update(&mut self, xr: &XrState, app: &mut AppState) -> anyhow::Result<()> {
        xr.session.sync_actions(&[(&self.action_set).into()])?;

        for input in &mut self.physical_inputs {
            input.update(&xr.session)?;
        }
        let physical_inputs = &self.physical_inputs;

        app.input_state.eye_gaze = if xr.extra_exts.ext_eye_gaze_interaction {
            self.handsfree_pointer.locate_tracked_pose(xr)?
        } else {
            None
        };

        let loc = xr.view.locate(&xr.stage, xr.predicted_display_time)?;
        let hmd = posef_to_transform(&loc.pose);
        let mut hmd_tracked = true;
        if loc
            .location_flags
            .contains(xr::SpaceLocationFlags::ORIENTATION_VALID)
        {
            app.input_state.hmd.matrix3 = hmd.matrix3;
        } else {
            hmd_tracked = false;
        }

        if loc
            .location_flags
            .contains(xr::SpaceLocationFlags::POSITION_VALID)
        {
            app.input_state.hmd.translation = hmd.translation;
        } else {
            hmd_tracked = false;
        }

        let mut any_tracked = false;
        let old_handsfree = app.session.config.handsfree_pointer;

        if app.input_state.picking_focus.is_none() {
            let should_disable_lerp = app.input_state.should_disable_lerp();
            for i in 0..2 {
                let pointer = &mut app.input_state.pointers[i];
                self.pointers[i].update(
                    pointer,
                    xr,
                    &app.session,
                    !should_disable_lerp,
                    physical_inputs,
                )?;
                any_tracked |= pointer.tracked;
            }
        } else {
            app.session.config.handsfree_pointer = app.session.config.handsfree_alt_tab.into();

            app.input_state.handsfree_state.scroll_x =
                app.input_state.handsfree_state.scroll_x.lerp(0.0, 0.7);
            app.input_state.handsfree_state.scroll_y =
                app.input_state.handsfree_state.scroll_y.lerp(0.0, 0.7);

            let ptr1 = &mut app.input_state.pointers[1];
            ptr1.before = ptr1.now;
            ptr1.now = PointerState::default();
            ptr1.tracked = false;
        }

        if !any_tracked {
            self.handsfree_pointer.update_handsfree(
                &mut app.input_state.pointers[0],
                xr,
                &app.session,
                hmd,
                hmd_tracked,
                &app.input_state.handsfree_state,
                physical_inputs,
            )?;

            app.session.config.handsfree_pointer = old_handsfree;
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

    fn locate_tracked_pose(&self, xr: &XrState) -> anyhow::Result<Option<Affine3A>> {
        let location = self.space.locate(&xr.stage, xr.predicted_display_time)?;
        let required_flags = xr::SpaceLocationFlags::ORIENTATION_VALID
            | xr::SpaceLocationFlags::ORIENTATION_TRACKED
            | xr::SpaceLocationFlags::POSITION_VALID;

        Ok(location
            .location_flags
            .contains(required_flags)
            .then(|| posef_to_transform(&location.pose)))
    }

    pub(super) fn update_handsfree(
        &mut self,
        pointer: &mut Pointer,
        xr: &XrState,
        session: &AppSession,
        hmd: Affine3A,
        hmd_tracked: bool,
        handsfree_state: &PointerState,
        physical_inputs: &[PhysicalInput],
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
                self.pointer_load_pose(
                    pointer,
                    xr,
                    Some(session.config.pointer_lerp_factor * 0.5),
                )?;
            }
        }

        pointer.handsfree = pointer.tracked;
        if matches!(
            session.config.handsfree_pointer,
            HandsfreePointer::HmdOnly | HandsfreePointer::EyeTrackingOnly
        ) {
            // input from wayvrctl
            pointer.now.click = handsfree_state.click;
            pointer.now.grab = handsfree_state.grab;
            pointer.now.grab_float = handsfree_state.grab_float;
            pointer.now.click_modifier_right = handsfree_state.click_modifier_right;
            pointer.now.click_modifier_middle = handsfree_state.click_modifier_middle;
            pointer.now.scroll_y = handsfree_state.scroll_y;

            // skip action loading
            return Ok(());
        }

        self.pointer_load_actions(pointer, xr, physical_inputs)?;
        pointer.now.click_modifier_right = handsfree_state.click_modifier_right;
        pointer.now.click_modifier_middle = handsfree_state.click_modifier_middle;
        pointer.now.scroll_y = handsfree_state.scroll_y;

        Ok(())
    }

    pub(super) fn update(
        &mut self,
        pointer: &mut Pointer,
        xr: &XrState,
        session: &AppSession,
        do_lerp: bool,
        physical_inputs: &[PhysicalInput],
    ) -> anyhow::Result<()> {
        pointer.handsfree = false;
        self.pointer_load_pose(
            pointer,
            xr,
            if do_lerp {
                Some(session.config.pointer_lerp_factor)
            } else {
                None
            },
        )?;
        self.pointer_load_actions(pointer, xr, physical_inputs)?;

        Ok(())
    }

    fn pointer_load_pose(
        &mut self,
        pointer: &mut Pointer,
        xr: &XrState,
        lerp_factor: Option<f32>,
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

            pointer.raw_pose = Affine3A::from_rotation_translation(new_quat, new_pos);
            if let Some(lerp_factor) = lerp_factor {
                let lerp_factor = (1.0 / (xr.fps / 100.0) * lerp_factor).clamp(0.1, 1.0);
                pointer.pose = Affine3A::from_rotation_translation(
                    cur_quat.lerp(new_quat, lerp_factor),
                    cur_pos.lerp(new_pos.into(), lerp_factor).into(),
                );
            } else {
                pointer.pose = pointer.raw_pose; // no lerp
            }
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
        physical_inputs: &[PhysicalInput],
    ) -> anyhow::Result<()> {
        pointer.now.click = self.source.click.state(physical_inputs);

        pointer.now.grab = self.source.grab.state(physical_inputs);

        let scroll = self
            .source
            .scroll
            .state(&xr.session, xr::Path::NULL)?
            .current_state;

        pointer.now.scroll_x = scroll.x;
        pointer.now.scroll_y = scroll.y;

        pointer.now.alt_click = self.source.alt_click.state(physical_inputs);

        pointer.now.show_hide = self.source.show_hide.state(physical_inputs);

        pointer.now.click_modifier_right = self.source.modifier_right.state(physical_inputs);

        pointer.now.toggle_dashboard = self.source.toggle_dashboard.state(physical_inputs);

        pointer.now.click_modifier_middle = self.source.modifier_middle.state(physical_inputs);

        pointer.now.move_mouse = self.source.move_mouse.state(physical_inputs);

        pointer.now.space_drag = self.source.space_drag.state(physical_inputs);

        pointer.now.space_rotate = self.source.space_rotate.state(physical_inputs);

        pointer.now.space_reset = self.source.space_reset.state(physical_inputs);

        Ok(())
    }
}

// supported direct action types: Haptic, Posef, Vector2f
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
            click: CustomClickAction::default(),
            grab: CustomClickAction::default(),
            scroll: action_scroll,
            alt_click: CustomClickAction::default(),
            show_hide: CustomClickAction::default(),
            toggle_dashboard: CustomClickAction::default(),
            space_drag: CustomClickAction::default(),
            space_rotate: CustomClickAction::default(),
            space_reset: CustomClickAction::default(),
            modifier_right: CustomClickAction::default(),
            modifier_middle: CustomClickAction::default(),
            move_mouse: CustomClickAction::default(),
            haptics: action_haptics,
        })
    }
}

fn to_path(path_str: &str, instance: &xr::Instance) -> Option<xr::Path> {
    instance
        .string_to_path(path_str)
        .inspect_err(|_| {
            log::warn!("Invalid binding path: {path_str}");
        })
        .ok()
}

fn is_bool(path_str: &str) -> bool {
    path_str
        .split('/')
        .next_back()
        .is_some_and(|last| matches!(last, "click" | "touch") || last.starts_with("dpad_"))
}

fn for_each_path(spec: Option<&OneOrMany<String>>, mut f: impl FnMut(&str)) {
    let Some(spec) = spec else {
        return;
    };

    match spec {
        OneOrMany::One(path) => f(path),
        OneOrMany::Many(paths) => paths.iter().for_each(|path| f(path)),
    }
}

fn button_actions(profile: &OpenXrInputProfile) -> [Option<&OpenXrInputAction>; 11] {
    [
        profile.click.as_ref(),
        profile.alt_click.as_ref(),
        profile.grab.as_ref(),
        profile.show_hide.as_ref(),
        profile.toggle_dashboard.as_ref(),
        profile.space_drag.as_ref(),
        profile.space_rotate.as_ref(),
        profile.space_reset.as_ref(),
        profile.click_modifier_right.as_ref(),
        profile.click_modifier_middle.as_ref(),
        profile.move_mouse.as_ref(),
    ]
}

fn action_spec(action: &OpenXrInputAction, side: usize) -> Option<&OneOrMany<String>> {
    match side {
        0 => action.left.as_ref(),
        1 => action.right.as_ref(),
        2 => action.handsfree.as_ref(),
        _ => unreachable!(),
    }
}

fn action_chord(action: &OpenXrInputAction, side: usize) -> Option<&[OpenXrInputChordMember]> {
    match side {
        0 => action.left_chord.as_deref(),
        1 => action.right_chord.as_deref(),
        2 => None,
        _ => unreachable!(),
    }
}

fn action_threshold(action: &OpenXrInputAction, side: usize) -> [f32; 2] {
    match side {
        0 => action.threshold_left,
        1 => action.threshold_right,
        2 => None,
        _ => unreachable!(),
    }
    .unwrap_or(DEFAULT_BUTTON_THRESHOLDS)
}

fn click_count(action: &OpenXrInputAction) -> ClickCount {
    if action.triple_click.unwrap_or(false) {
        ClickCount::Triple
    } else if action.double_click.unwrap_or(false) {
        ClickCount::Double
    } else {
        ClickCount::Single
    }
}

type PhysicalInputMap = HashMap<(usize, String), usize>;

fn create_physical_inputs(
    action_set: &xr::ActionSet,
    instance: &xr::Instance,
    profiles: &[OpenXrInputProfile],
) -> anyhow::Result<(Vec<PhysicalInput>, PhysicalInputMap)> {
    let mut plans = Vec::<(usize, xr::Path, bool)>::new();
    let mut map = PhysicalInputMap::new();

    for (profile_index, profile) in profiles.iter().enumerate() {
        if instance.string_to_path(&profile.profile).is_err() {
            log::warn!("Invalid interaction profile path: {}", profile.profile);
            continue;
        }

        let mut add_path = |path_str: &str| {
            let key = (profile_index, path_str.to_owned());
            if map.contains_key(&key) {
                return;
            }

            let Some(path) = to_path(path_str, instance) else {
                return;
            };

            let index = plans.len();
            plans.push((profile_index, path, is_bool(path_str)));
            map.insert(key, index);
        };

        for action in button_actions(profile).into_iter().flatten() {
            for side in 0..3 {
                for_each_path(action_spec(action, side), &mut add_path);
            }

            if let Some(chord) = action.left_chord.as_deref() {
                for member in chord {
                    add_path(member.path());
                }
            }
            if let Some(chord) = action.right_chord.as_deref() {
                for member in chord {
                    add_path(member.path());
                }
            }
        }
    }

    let mut physical_inputs = Vec::with_capacity(plans.len());
    for (index, (profile_index, path, bool_input)) in plans.into_iter().enumerate() {
        let name = format!("physical_input_{index}");
        let display_name = format!("Physical input {index}");
        let action = if bool_input {
            PhysicalInputAction::Bool {
                action: action_set.create_action::<bool>(&name, &display_name, &[])?,
                current: None,
            }
        } else {
            PhysicalInputAction::Float {
                action: action_set.create_action::<f32>(&name, &display_name, &[])?,
                current: None,
            }
        };

        physical_inputs.push(PhysicalInput {
            profile_index,
            path,
            action,
        });
    }

    log::debug!(
        "Created {} OpenXR physical button actions",
        physical_inputs.len()
    );

    Ok((physical_inputs, map))
}

fn physical_input_index(map: &PhysicalInputMap, profile_index: usize, path: &str) -> Option<usize> {
    map.get(&(profile_index, path.to_owned())).copied()
}

fn build_button_bindings(
    action: &OpenXrInputAction,
    side: usize,
    profile_index: usize,
    physical_input_map: &PhysicalInputMap,
) -> Vec<ButtonBinding> {
    let mut bindings = Vec::with_capacity(2);
    let threshold = action_threshold(action, side);
    let count = click_count(action);

    let mut alternatives = Vec::new();
    for_each_path(action_spec(action, side), |path| {
        if let Some(input) = physical_input_index(physical_input_map, profile_index, path) {
            alternatives.push(ButtonCondition::new(input, threshold));
        }
    });
    if !alternatives.is_empty() {
        bindings.push(ButtonBinding {
            conditions: ButtonConditionSet::Any(alternatives),
            clicks: MultiClickHandler::new(count),
        });
    }

    if let Some(members) = action_chord(action, side).filter(|members| !members.is_empty()) {
        let mut conditions = Vec::with_capacity(members.len());
        let mut valid = true;
        for member in members {
            let Some(input) =
                physical_input_index(physical_input_map, profile_index, member.path())
            else {
                // Never turn an invalid chord into a smaller, easier-to-trigger chord.
                valid = false;
                break;
            };
            conditions.push(ButtonCondition::new(
                input,
                member.threshold().unwrap_or(DEFAULT_BUTTON_THRESHOLDS),
            ));
        }

        if valid {
            bindings.push(ButtonBinding {
                conditions: ButtonConditionSet::All(conditions),
                clicks: MultiClickHandler::new(count),
            });
        }
    }

    bindings
}

fn add_button_bindings(
    action: Option<&OpenXrInputAction>,
    profile_index: usize,
    hands: &mut [&mut OpenXrHandSource; 3],
    physical_input_map: &PhysicalInputMap,
    field: fn(&mut OpenXrHandSource) -> &mut CustomClickAction,
) {
    let Some(action) = action else {
        return;
    };

    for (side, hand) in hands.iter_mut().enumerate() {
        for binding in build_button_bindings(action, side, profile_index, physical_input_map) {
            field(&mut **hand).add_binding(binding);
        }
    }
}

macro_rules! add_direct_bindings {
    ($action:expr, $field:ident, $hands:expr, $bindings:expr, $instance:expr) => {
        if let Some(action) = $action {
            for i in 0..3 {
                let spec = match i {
                    0 => action.left.as_ref(),
                    1 => action.right.as_ref(),
                    2 => action.handsfree.as_ref(),
                    _ => unreachable!(),
                };

                for_each_path(spec, |path_str| {
                    if let Some(path) = to_path(path_str, $instance) {
                        $bindings.push(xr::Binding::new(&$hands[i].$field, path));
                    }
                });
            }
        }
    };
}

#[allow(clippy::too_many_lines)]
fn suggest_bindings(
    instance: &xr::Instance,
    hands: &mut [&mut OpenXrHandSource; 3],
    profiles: &[OpenXrInputProfile],
    physical_inputs: &[PhysicalInput],
    physical_input_map: &PhysicalInputMap,
) {
    for (profile_index, profile) in profiles.iter().enumerate() {
        log::debug!("Loading profile {}", profile.profile);

        let Ok(profile_path) = instance.string_to_path(&profile.profile) else {
            log::warn!("Profile not supported: {}", profile.profile);
            continue;
        };

        // create out virtual buttons, then the xr::Binding's
        add_button_bindings(
            profile.click.as_ref(),
            profile_index,
            hands,
            physical_input_map,
            |hand| &mut hand.click,
        );
        add_button_bindings(
            profile.alt_click.as_ref(),
            profile_index,
            hands,
            physical_input_map,
            |hand| &mut hand.alt_click,
        );
        add_button_bindings(
            profile.grab.as_ref(),
            profile_index,
            hands,
            physical_input_map,
            |hand| &mut hand.grab,
        );
        add_button_bindings(
            profile.show_hide.as_ref(),
            profile_index,
            hands,
            physical_input_map,
            |hand| &mut hand.show_hide,
        );
        add_button_bindings(
            profile.toggle_dashboard.as_ref(),
            profile_index,
            hands,
            physical_input_map,
            |hand| &mut hand.toggle_dashboard,
        );
        add_button_bindings(
            profile.space_drag.as_ref(),
            profile_index,
            hands,
            physical_input_map,
            |hand| &mut hand.space_drag,
        );
        add_button_bindings(
            profile.space_rotate.as_ref(),
            profile_index,
            hands,
            physical_input_map,
            |hand| &mut hand.space_rotate,
        );
        add_button_bindings(
            profile.space_reset.as_ref(),
            profile_index,
            hands,
            physical_input_map,
            |hand| &mut hand.space_reset,
        );
        add_button_bindings(
            profile.click_modifier_right.as_ref(),
            profile_index,
            hands,
            physical_input_map,
            |hand| &mut hand.modifier_right,
        );
        add_button_bindings(
            profile.click_modifier_middle.as_ref(),
            profile_index,
            hands,
            physical_input_map,
            |hand| &mut hand.modifier_middle,
        );
        add_button_bindings(
            profile.move_mouse.as_ref(),
            profile_index,
            hands,
            physical_input_map,
            |hand| &mut hand.move_mouse,
        );

        let mut bindings: Vec<xr::Binding> = vec![];

        add_direct_bindings!(profile.pose.as_ref(), pose, hands, bindings, instance);
        add_direct_bindings!(profile.haptic.as_ref(), haptics, hands, bindings, instance);
        add_direct_bindings!(profile.scroll.as_ref(), scroll, hands, bindings, instance);

        // 1 physical source → 1 action per interaction profile
        for input in physical_inputs
            .iter()
            .filter(|input| input.profile_index == profile_index)
        {
            match &input.action {
                PhysicalInputAction::Bool { action, .. } => {
                    bindings.push(xr::Binding::new(action, input.path));
                }
                PhysicalInputAction::Float { action, .. } => {
                    bindings.push(xr::Binding::new(action, input.path));
                }
            }
        }

        let profile_name = profile
            .profile
            .strip_prefix("/interaction_profiles/")
            .unwrap_or(&profile.profile);
        if instance
            .suggest_interaction_profile_bindings(profile_path, &bindings)
            .is_err()
        {
            log::warn!("Could not apply bindings for {profile_name}");
        } else {
            log::debug!("Bindings for {profile_name} bound successfully.");
        }
    }
}
