use glam::{Mat4, Vec2, Vec3};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WMat4(pub [f32; 16]);

impl WMat4 {
	pub fn from_glam(mat: &Mat4) -> Self {
		Self(*mat.as_ref())
	}
}

impl Default for WMat4 {
	fn default() -> Self {
		Self(*Mat4::IDENTITY.as_ref())
	}
}

// works just like CSS transform-origin 50% 50%
pub fn centered_matrix(box_size: Vec2, input: &Mat4) -> Mat4 {
	Mat4::from_translation(Vec3::new(box_size.x / 2.0, box_size.y / 2.0, 0.0))
		* *input
		* Mat4::from_translation(Vec3::new(-box_size.x / 2.0, -box_size.y / 2.0, 0.0))
}
