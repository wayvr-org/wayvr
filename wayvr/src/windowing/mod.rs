use glam::{Affine3A, Vec2, Vec3A, Vec3Swizzles};
use slotmap::new_key_type;
use std::{f32::consts::PI, sync::Arc};

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

pub fn raycast_overlay(
    source: &Affine3A,
    overlay_pose: &Affine3A,
    curvature: Option<f32>,
) -> Option<(f32, Vec2)> {
    let (dist, local_pos) = curvature.map_or_else(
        || Some(raycast_plane(source, Vec3A::NEG_Z, overlay_pose, Vec3A::NEG_Z)),
        |curvature| raycast_cylinder(source, Vec3A::NEG_Z, overlay_pose, curvature),
    )?;

    if !dist.is_finite() || dist < 0.0 {
        return None;
    }

    Some((dist, local_pos))
}

fn raycast_plane(
    source: &Affine3A,
    source_fwd: Vec3A,
    plane: &Affine3A,
    plane_norm: Vec3A,
) -> (f32, Vec2) {
    let plane_normal = plane.transform_vector3a(plane_norm);
    let ray_dir = source.transform_vector3a(source_fwd);

    let d = plane.translation.dot(-plane_normal);
    let mut dist = -(d + source.translation.dot(plane_normal)) / ray_dir.dot(plane_normal);

    let hit_local = plane
        .inverse()
        .transform_point3a(source.translation + ray_dir * dist)
        .xy();

    // hitting the backside of the plane, make the hit invalid
    if ray_dir.dot(plane_normal) < 0.0 && dist.is_sign_positive() {
        dist = -dist;
    }

    (dist, hit_local)
}

fn raycast_cylinder(
    source: &Affine3A,
    source_fwd: Vec3A,
    plane: &Affine3A,
    curvature: f32,
) -> Option<(f32, Vec2)> {
    // this is solved locally; (0,0) is the center of the cylinder, and the cylinder is aligned with the Y axis
    let size = plane.x_axis.length();
    let to_local = Affine3A {
        matrix3: plane.matrix3.mul_scalar(1.0 / size),
        translation: plane.translation,
    }
    .inverse();

    let radius = size / (2.0 * PI * curvature);

    let ray_dir = to_local.transform_vector3a(source.transform_vector3a(source_fwd));
    let ray_origin = to_local.transform_point3a(source.translation) + Vec3A::NEG_Z * radius;

    let v_dir = ray_dir.xz();
    let v_pos = ray_origin.xz();

    let l_dir = v_dir.dot(v_dir);
    let l_pos = v_dir.dot(v_pos);
    let c = radius.mul_add(-radius, v_pos.dot(v_pos));

    let d = l_pos.mul_add(l_pos, -(l_dir * c));
    if d < f32::EPSILON {
        return None;
    }

    let sqrt_d = d.sqrt();

    let t1 = (-l_pos - sqrt_d) / l_dir;
    let t2 = (-l_pos + sqrt_d) / l_dir;

    let mut t = t1.max(t2);

    if t < f32::EPSILON {
        return None;
    }

    let mut hit_local = ray_origin + ray_dir * t;
    if hit_local.z > 0.0 {
        // hitting the opposite half of the cylinder
        return None;
    }

    let normal = Vec3A::new(hit_local.x, 0.0, hit_local.z).normalize();
    // If hitting from the outside, flip t
    if ray_dir.dot(normal) < 0.0 && t.is_sign_positive() {
        t = -t;
    }

    let max_angle = 2.0 * (size / (2.0 * radius));
    let x_angle = (hit_local.x / radius).asin();

    hit_local.x = x_angle / max_angle;
    hit_local.y /= size;

    Some((t, hit_local.xy()))
}

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

pub fn overlay_scale_from_extent(size: [u32; 2]) -> (f32, f32) {
    const RELATIVE_SIZE: f32 = 1440.0; // sqrt(1920 * 1080)

    let w = size[0].max(1) as f32;
    let h = size[1].max(1) as f32;

    ((w * h).sqrt() / RELATIVE_SIZE, w / 1920.0)
}
