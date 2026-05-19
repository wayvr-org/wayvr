use glam::Vec3A;
use wlx_common::config::GeneralConfig;

pub struct SpaceGravityUpdateParams<'a> {
    pub dt: f32,
    pub dragging: bool,
    pub config: &'a GeneralConfig,
}

pub struct SpaceGravity {
    velocity: Vec3A,
    space_pos: Vec3A,
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
        self.velocity = hand_pos_diff * config.space_drag_fling_strength / dt;
        self.space_pos = space_pos;
    }

    pub fn update(&mut self, par: SpaceGravityUpdateParams) -> Option<Vec3A> {
        if !par.dragging {
            self.velocity.y += par.config.space_drag_gravity * par.dt;
            // terminal velocity
            self.velocity.y = self.velocity.y.min(200.0);

            self.velocity *= (par.config.space_drag_damping).powf(par.dt * 10.0);
            self.space_pos += self.velocity * par.dt;

            self.space_pos.y = self.space_pos.y.min(0.0);

            if self.velocity.length_squared() > 0.00003 {
                // log::info!("velocity {}", self.velocity);
                return Some(self.space_pos);
            }
        }

        None
    }
}
