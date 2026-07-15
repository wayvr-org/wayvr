use glam::{Affine3A, EulerRot, Mat3A, Quat, Vec3, Vec3A};
use idmap_derive::IntegerId;
use std::{f32::consts::PI, sync::Arc};
use wlx_common::windowing::{OverlayWindowState, Positioning};

use crate::{
    state::AppState,
    subsystem::input::InputFocus,
    windowing::{
        backend::{FrameMeta, OverlayBackend, RenderResources, ShouldRender},
        snap_upright,
    },
};

pub struct OverlayWindowData<T> {
    pub config: OverlayWindowConfig,
    pub data: T,
    pub birthframe: usize,
    pub primary_pointer: Option<usize>,
    pub hover_pointers: [bool; 2],
}

impl<T> OverlayWindowData<T>
where
    T: Default,
{
    pub fn from_config(config: OverlayWindowConfig) -> Self {
        Self {
            data: T::default(),
            config,
            primary_pointer: None,
            hover_pointers: [false, false],
            birthframe: 0,
        }
    }
}

impl<T> OverlayWindowData<T> {
    pub fn init(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        //TODO: load state?

        self.config.backend.init(app)
    }
    pub fn should_render(&mut self, app: &mut AppState) -> anyhow::Result<ShouldRender> {
        self.config.backend.should_render(app)
    }
    pub fn render(&mut self, app: &mut AppState, rdr: &mut RenderResources) -> anyhow::Result<()> {
        self.config.backend.render(app, rdr)
    }
    pub fn frame_meta(&mut self) -> Option<FrameMeta> {
        self.config.backend.frame_meta()
    }
}

#[derive(Debug, Clone, Copy, IntegerId, PartialEq, Eq)]
pub enum OverlayCategory {
    Internal,
    Keyboard,
    Dashboard,
    /// User-provided custom panels
    Panel,
    /// Same as Panel but does not support Modify and Reload
    BuiltInPanel,
    Screen,
    Mirror,
    WayVR,
}

pub struct OverlayWindowConfig {
    pub name: Arc<str>,
    pub backend: Box<dyn OverlayBackend>,
    /// The default state to show when the overlay is newly spawned.
    pub default_state: OverlayWindowState,
    /// The current state to show. None if the overlay is hidden.
    pub active_state: Option<OverlayWindowState>,
    /// Order to draw overlays in. Overlays with higher numbers will be drawn over ones with lower numbers.
    pub z_order: u32,
    /// If set, hovering this overlay will cause the HID provider to switch focus.
    pub input_focus: Option<InputFocus>,
    /// Category of the overlay, used by toolbox on the watch.
    pub category: OverlayCategory,
    /// Should the overlay be displayed on the next frame?
    pub show_on_spawn: bool,
    /// Does not belong to any set; switching sets does not affect this overlay.
    pub global: bool,
    /// True if transform, curvature, alpha has changed. Only used by OpenVR.
    pub dirty: bool,
    /// True if the window is showing the edit overlay
    pub editing: bool,
    /// True if the window is being resized. Captures pointer hits even if not directly hit.
    pub resizing: bool,
    /// Used by grab to pause following of HMD or other devices
    pub pause_movement: bool,
}

impl OverlayWindowConfig {
    pub fn from_backend(backend: Box<dyn OverlayBackend>) -> Self {
        Self {
            name: "".into(),
            backend,
            default_state: OverlayWindowState {
                transform: Affine3A::from_translation(Vec3::NEG_Z),
                ..OverlayWindowState::default()
            },
            active_state: None,
            z_order: 0,
            input_focus: None,
            category: OverlayCategory::Internal,
            show_on_spawn: false,
            global: false,
            dirty: true,
            editing: false,
            resizing: false,
            pause_movement: false,
        }
    }

    pub const fn is_active(&self) -> bool {
        self.active_state.is_some()
    }

    /// only call this directly for `OverlayCategory::Internal`
    /// for anything else, use `OverlayTask::ToggleOverlay` instead
    pub fn activate(&mut self, app: &mut AppState) {
        log::debug!("activate {}", self.name.as_ref());
        self.dirty = true;
        self.active_state = Some(self.default_state.clone());
        self.reset(app, true);
    }

    /// only call this directly for `OverlayCategory::Internal`
    /// for anything else, use `OverlayTask::ToggleOverlay` instead
    pub fn deactivate(&mut self) {
        log::debug!("deactivate {}", self.name.as_ref());
        self.active_state = None;
    }

    pub fn tick(&mut self, app: &mut AppState) {
        self.auto_movement(app);
        self.angle_fade(app);
    }

    fn auto_movement(&mut self, app: &mut AppState) {
        if self.pause_movement {
            return;
        }

        let Some(state) = self.active_state.as_mut() else {
            return;
        };

        let cur_transform = state
            .saved_transform
            .unwrap_or(self.default_state.transform);
        let scale = scalar_scale(&cur_transform);

        let (parent_transform, lerp, align_to_hmd) = match state.positioning {
            Positioning::FollowHead { lerp } => (app.input_state.hmd, lerp, false),
            Positioning::FollowHand {
                hand,
                lerp,
                align_to_hmd,
            } => (
                app.input_state.pointers[hand as usize].pose,
                lerp,
                align_to_hmd,
            ),
            Positioning::Anchored => (app.anchor, 1.0, false),
            _ => return,
        };

        let target_transform = parent_transform * cur_transform;

        state.transform = match lerp {
            1.0 => target_transform,
            lerp => {
                let rot_from = Quat::from_mat3a(&state.transform.matrix3.div_scalar(scale));
                let rot_to = Quat::from_mat3a(&target_transform.matrix3.div_scalar(scale));

                let rotation = rot_from.slerp(rot_to, lerp);
                let translation = state
                    .transform
                    .translation
                    .slerp(target_transform.translation, lerp);

                Affine3A::from_scale_rotation_translation(
                    Vec3::ONE * scale,
                    rotation,
                    translation.into(),
                )
            }
        };

        if align_to_hmd {
            realign(
                &mut state.transform,
                &app.input_state.hmd,
                scale,
                app.session.config.snap_angle_deg,
            );
        }

        self.dirty = true;
    }

    fn angle_fade(&mut self, app: &mut AppState) {
        let Some(state) = self.active_state.as_mut() else {
            return;
        };

        if !state.angle_fade {
            return;
        }

        let to_hmd = (state.transform.translation - app.input_state.hmd.translation).normalize();
        let watch_normal = state.transform.transform_vector3a(Vec3A::NEG_Z).normalize();
        let dot = to_hmd.dot(watch_normal);

        state.alpha = (dot - app.session.config.watch_view_angle_min)
            / (app.session.config.watch_view_angle_max - app.session.config.watch_view_angle_min);
        state.alpha += 0.1;
        state.alpha = state.alpha.clamp(0., 1.);
    }

    /// Returns true if changes were saved.
    pub fn reset(&mut self, app: &mut AppState, hard_reset: bool) {
        let Some(state) = self.active_state.as_mut() else {
            return;
        };

        let cur_transform = state
            .saved_transform
            .unwrap_or(self.default_state.transform);

        let (parent_transform, align_to_hmd) = match state.positioning {
            Positioning::Floating | Positioning::FollowHead { .. } => (app.input_state.hmd, false),
            Positioning::FollowHand {
                hand, align_to_hmd, ..
            } => (app.input_state.pointers[hand as usize].pose, align_to_hmd),
            Positioning::Anchored => (app.anchor, false),
            Positioning::Static => {
                if hard_reset {
                    (app.input_state.hmd, false)
                } else {
                    (Affine3A::IDENTITY, false)
                }
            }
        };

        if hard_reset {
            state.saved_transform = None;
        }

        state.transform = parent_transform * cur_transform;

        if align_to_hmd || (state.grabbable && hard_reset) {
            let scale = scalar_scale(&cur_transform);
            realign(
                &mut state.transform,
                &app.input_state.hmd,
                scale,
                app.session.config.snap_angle_deg,
            );
        }
        self.dirty = true;
    }
}

pub fn realign(transform: &mut Affine3A, hmd: &Affine3A, scale: f32, snap_angle: f32) {
    let to_hmd = hmd.translation - transform.translation;
    let up_dir: Vec3A;

    if hmd.x_axis.dot(Vec3A::Y).abs() > 0.2 {
        // Snap upright
        up_dir = hmd.y_axis;
    } else {
        let dot = to_hmd.normalize().dot(hmd.z_axis);
        let z_dist = to_hmd.length();
        let y_dist = (transform.translation.y - hmd.translation.y).abs();
        let x_angle = (y_dist / z_dist).asin();

        if dot < -f32::EPSILON {
            // facing down
            let up_point = hmd.translation + z_dist / x_angle.cos() * Vec3A::Y;
            up_dir = (up_point - transform.translation).normalize();
        } else if dot > f32::EPSILON {
            // facing up
            let dn_point = hmd.translation + z_dist / x_angle.cos() * Vec3A::NEG_Y;
            up_dir = (transform.translation - dn_point).normalize();
        } else {
            // perfectly upright
            up_dir = Vec3A::Y;
        }
    }

    let col_z = (transform.translation - hmd.translation).normalize();
    let col_y = up_dir;
    let col_x = col_y.cross(col_z);
    let col_y = col_z.cross(col_x).normalize();
    let col_x = col_x.normalize();

    let flip = Mat3A::from_quat(Quat::from_axis_angle(Vec3::Y, PI));
    let mut rotation = Mat3A::from_cols(col_x, col_y, col_z) * flip;

    if snap_angle > f32::EPSILON {
        let snap_step = snap_angle.to_radians();
        let (yaw, pitch, roll) = rotation.to_euler(EulerRot::YXZ);

        let pitch = (pitch / snap_step).round() * snap_step;
        let roll = (roll / snap_step).round() * snap_step;

        rotation = Mat3A::from_euler(EulerRot::YXZ, yaw, pitch, roll);
    }

    transform.matrix3 = rotation.mul_scalar(scale);
}

pub fn scalar_scale(a: &Affine3A) -> f32 {
    let det = a.matrix3.determinant();
    (a.matrix3.x_axis.length() * det.signum()
        + a.matrix3.y_axis.length()
        + a.matrix3.z_axis.length())
        / 3.0
}

pub fn save_transform(state: &mut OverlayWindowState, app: &mut AppState) {
    let parent_transform = match state.positioning {
        Positioning::Floating => snap_upright(app.input_state.hmd, Vec3A::Y),
        Positioning::FollowHead { .. } => app.input_state.hmd,
        Positioning::FollowHand { hand, .. } => app.input_state.pointers[hand as usize].pose,
        Positioning::Anchored => snap_upright(app.anchor, Vec3A::Y),
        Positioning::Static => Affine3A::IDENTITY,
    };

    state.saved_transform = Some(parent_transform.inverse() * state.transform);
}

pub fn spawn_transform_from_parent(
    parent: &Affine3A,
    hmd: &Affine3A,
    child_scale: f32,
) -> Affine3A {
    const LEFT: f32 = 0.08;
    const DOWN: f32 = 0.08;
    const CLOSER_TO_HMD: f32 = 0.06;

    let x_axis = parent.x_axis.normalize();
    let y_axis = parent.y_axis.normalize();
    let z_axis = parent.z_axis.normalize();
    let to_hmd = hmd.translation - parent.translation;
    let toward_hmd = if z_axis.dot(to_hmd) >= 0.0 {
        z_axis
    } else {
        -z_axis
    };

    let translation =
        parent.translation - x_axis * LEFT - y_axis * DOWN + toward_hmd * CLOSER_TO_HMD;

    let rotation = Quat::from_mat3a(&Mat3A::from_cols(x_axis, y_axis, z_axis));

    Affine3A::from_scale_rotation_translation(Vec3::ONE * child_scale, rotation, translation.into())
}
