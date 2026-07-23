use glam::{Affine3A, Vec3A};
use slotmap::new_key_type;
use std::sync::Arc;

use crate::windowing::window::scalar_scale;

pub mod backend;
pub mod manager;
pub mod set;
pub mod window;

new_key_type! {
    pub struct OverlayID;
}

#[derive(Clone, Debug)]
pub enum OverlaySelector {
    Id(OverlayID),
    Name(Arc<str>),
    Nothing,
}

pub const PIXELS_TO_METERS: f32 = 1. / 2000.;

pub const Z_ORDER_TOAST: u32 = 71;
pub const Z_ORDER_HELP: u32 = 70;
pub const Z_ORDER_LINES: u32 = 69;
pub const Z_ORDER_WATCH: u32 = 68;
pub const Z_ORDER_ANCHOR: u32 = 67;
pub const Z_ORDER_DEFAULT: u32 = 0;
pub const Z_ORDER_DASHBOARD: u32 = Z_ORDER_DEFAULT;

pub fn snap_upright(transform: Affine3A, up_dir: Vec3A) -> Affine3A {
    if transform.x_axis.dot(up_dir).abs() < 0.2 {
        let scale = scalar_scale(&transform);
        let col_z = transform.z_axis.normalize();
        let col_y = up_dir;
        let col_x = col_y.cross(col_z);
        let col_y = col_z.cross(col_x).normalize();
        let col_x = col_x.normalize();

        Affine3A::from_cols(
            col_x * scale,
            col_y * scale,
            col_z * scale,
            transform.translation,
        )
    } else {
        transform
    }
}
