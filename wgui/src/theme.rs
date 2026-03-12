use crate::drawing;

#[derive(Clone)]
pub struct WguiTheme {
	pub dark_mode: bool,
	pub text_color: drawing::Color,
	pub button_color: drawing::Color,
	pub accent_color: drawing::Color,
	pub danger_color: drawing::Color,
	pub faded_color: drawing::Color,
	pub bg_color: drawing::Color,
	pub editbox_color: drawing::Color,
	pub translucent_alpha: f32,
	pub animation_mult: f32,
	pub rounding_mult: f32,
	pub gradient_intensity: f32, // currently used for buttons
}

impl Default for WguiTheme {
	fn default() -> Self {
		Self {
			dark_mode: true,
			text_color: drawing::Color::new(1.0, 1.0, 1.0, 1.0),
			button_color: drawing::Color::new(1.0, 1.0, 1.0, 0.02),
			accent_color: drawing::Color::new(0.13, 0.68, 1.0, 1.0),
			danger_color: drawing::Color::new(0.9, 0.0, 0.0, 1.0),
			faded_color: drawing::Color::new(0.67, 0.74, 0.80, 1.0),
			bg_color: drawing::Color::new(0.0, 0.07, 0.1, 0.75),
			editbox_color: drawing::Color::new(0.15, 0.25, 0.35, 0.95),
			translucent_alpha: 0.5,
			animation_mult: 1.0,
			rounding_mult: 1.0,
			gradient_intensity: 0.3,
		}
	}
}
