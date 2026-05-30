use glam::Vec3A;
use wlx_common::config::GeneralConfig;

use crate::windowing::manager::OverlayWindowManager;

pub struct SpaceGravityUpdateParams<'a> {
    pub dt: f32,
    pub dragging: bool,
    pub config: &'a GeneralConfig,
    pub floor_height: f32,
}

pub struct SpaceGravity {
    velocity: Vec3A,
    space_pos: Vec3A,
}

pub fn shift_overlays<OverlayData>(
    overlays: &mut OverlayWindowManager<OverlayData>,
    overlay_offset: Vec3A,
) {
    overlays.values_mut().for_each(|overlay| {
        let Some(state) = overlay.config.active_state.as_mut() else {
            return;
        };
        if state.positioning.moves_with_space() {
            state.transform.translation += overlay_offset;
        }
        overlay.config.dirty = true;
    });
}

pub struct SpaceGravityUpdateResult {
    pub playspace_pos: Vec3A,
    pub playspace_pos_offset: Vec3A, // position difference compared to previous update() call
}

impl SpaceGravity {
    pub fn new() -> Self {
        Self {
            velocity: Vec3A::default(),
            space_pos: Vec3A::default(),
        }
    }

    pub fn mark_end_drag(
        &mut self,
        config: &GeneralConfig,
        hand_pos_diff: Vec3A,
        space_pos: Vec3A,
        dt: f32,
    ) {
        if config.space_gravity_enabled {
            self.velocity = hand_pos_diff * config.space_gravity_fling_strength / dt;
            self.space_pos = space_pos;
        } else {
            self.reset();
        }
    }

    pub fn reset(&mut self) {
        self.velocity = Vec3A::default();
        self.space_pos = Vec3A::default();
    }

    pub fn update(&mut self, par: SpaceGravityUpdateParams) -> Option<SpaceGravityUpdateResult> {
        if par.dragging || !par.config.space_gravity_enabled {
            return None;
        }

        let prev_pos = self.space_pos;

        self.velocity.y += par.config.space_gravity_gravity * par.dt;

        // terminal velocity
        self.velocity.y = self.velocity.y.min(200.0);

        self.velocity *= (par.config.space_gravity_damping).powf(par.dt * 10.0);

        self.space_pos += self.velocity * par.dt;

        self.space_pos.y = self.space_pos.y.min(par.floor_height);

        if self.space_pos.y >= par.floor_height
        /* at floor height or below */
        {
            // apply ground friction
            self.velocity *= 1.0 - par.config.space_gravity_ground_friction * par.dt * 10.0;
        }

        if self.velocity.length_squared() > 0.00003 {
            // Space position changed
            return Some(SpaceGravityUpdateResult {
                playspace_pos: self.space_pos,
                playspace_pos_offset: self.space_pos - prev_pos,
            });
        }

        None
    }
}
