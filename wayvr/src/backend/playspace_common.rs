use glam::{Affine3A, Mat3A, Vec3A};
use wlx_common::config::GeneralConfig;

use crate::windowing::manager::OverlayWindowManager;

pub struct SpaceGravityUpdateParams<'a> {
    pub dt: f32,
    pub dragging: bool,
    pub config: &'a GeneralConfig,
    pub floor_height: f32,

    // current playspace offset
    pub current_space_pos: Vec3A,

    pub current_playspace_rot: Mat3A,
}

pub struct SpaceGravity {
    velocity: Vec3A, // local space
}

pub fn shift_world<OverlayData>(
    overlays: &mut OverlayWindowManager<OverlayData>,
    anchor: &mut Affine3A,
    before: &Affine3A,
    after: &Affine3A,
) {
    let correction = after.inverse() * before;
    *anchor = correction * *anchor;

    overlays.values_mut().for_each(|overlay| {
        let Some(state) = overlay.config.active_state.as_mut() else {
            return;
        };
        state.transform = correction * state.transform;
        overlay.config.dirty = true;
    });
}

pub struct SpaceGravityUpdateResult {
    pub playspace_pos_offset: Vec3A, // position difference compared to the previous update() call
}

impl SpaceGravity {
    pub fn new() -> Self {
        Self {
            velocity: Vec3A::default(),
        }
    }

    pub fn mark_end_drag(
        &mut self,
        config: &GeneralConfig,
        hand_pos_diff: Vec3A,
        space_rot: Mat3A,
        dt: f32,
    ) {
        if config.space_gravity_enabled {
            self.velocity =
                space_rot.inverse() * hand_pos_diff * config.space_gravity_fling_strength / dt;
        } else {
            self.reset();
        }
    }

    pub fn reset(&mut self) {
        self.velocity = Default::default();
    }

    pub fn update(&mut self, par: SpaceGravityUpdateParams) -> Option<SpaceGravityUpdateResult> {
        if par.dragging || !par.config.space_gravity_enabled {
            return None;
        }

        let mut velocity = self.velocity;
        velocity += Vec3A::new(0.0, par.config.space_gravity_gravity * par.dt, 0.0) /* downwards gravity */;
        velocity *= par.config.space_gravity_damping.powf(par.dt * 10.0) /* damping */;

        // local space
        let prev_pos = par.current_playspace_rot.inverse() * par.current_space_pos;
        let mut space_pos = prev_pos;
        space_pos += velocity * par.dt;
        space_pos.y = space_pos.y.min(par.floor_height);

        // at floor height or below
        if space_pos.y >= par.floor_height {
            // apply ground friction
            velocity *= 1.0 - par.config.space_gravity_ground_friction * par.dt * 10.0;
        }

        self.velocity = velocity;

        let pos_diff = space_pos - prev_pos;
        if velocity.length_squared() > 0.00003 {
            // Space position changed
            return Some(SpaceGravityUpdateResult {
                playspace_pos_offset: par.current_playspace_rot * pos_diff,
            });
        }

        None
    }
}
