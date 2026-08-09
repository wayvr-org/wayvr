use crate::{drawing, palette::WguiColorPalette};
use strum::{EnumCount, EnumString};
// Primary: button color
// OnPrimary: text color placed on the Primary-colored button

#[derive(Debug, Copy, Clone, Eq, PartialEq, EnumCount, EnumString)]
#[repr(usize)]
#[strum(serialize_all = "snake_case")]
pub enum WguiColorName {
	Primary,
	OnPrimary,
	Secondary,
	OnSecondary,
	Tertiary,
	OnTertiary,
	Danger,
	OnDanger,
	Background,
	OnBackground,
	BackgroundVariant,
	OnBackgroundVariant,
	BackgroundContrast,
	OnBackgroundContrast,
	Outline,
	Shadow,
	Highlight,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WguiNamedColor {
	name: WguiColorName,
	rgb_multiplier: f32,
	rgb_addition: f32,
	alpha: f32,
}

#[derive(Default, Clone, Copy, Debug, EnumString)]
#[strum(ascii_case_insensitive, serialize_all = "snake_case")]
pub enum ParentColor {
	#[default]
	Foreground,
	Background,
	Ignore,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WguiColor {
	Raw(drawing::Color),
	Named(WguiNamedColor),
}

impl WguiColor {
	pub fn resolve(&self, palette: &WguiColorPalette) -> drawing::Color {
		match &self {
			WguiColor::Raw(color) => *color,
			WguiColor::Named(color) => color.resolve(palette),
		}
	}

	#[must_use]
	pub const fn mult_rgb(&self, mult: f32) -> WguiColor {
		match self {
			WguiColor::Raw(color) => WguiColor::Raw(color.mult_rgb(mult)),
			WguiColor::Named(color) => WguiColor::Named(WguiNamedColor {
				name: color.name,
				rgb_multiplier: color.rgb_multiplier * mult,
				rgb_addition: color.rgb_addition,
				alpha: color.alpha,
			}),
		}
	}

	#[must_use]
	pub const fn add_rgb(&self, addition: f32) -> WguiColor {
		match self {
			WguiColor::Raw(color) => WguiColor::Raw(color.add_rgb(addition)),
			WguiColor::Named(color) => WguiColor::Named(WguiNamedColor {
				name: color.name,
				rgb_multiplier: color.rgb_multiplier,
				rgb_addition: color.rgb_addition + addition,
				alpha: color.alpha,
			}),
		}
	}

	#[must_use]
	pub const fn add_alpha(&self, alpha: f32) -> WguiColor {
		match self {
			WguiColor::Raw(color) => WguiColor::Raw(drawing::Color {
				r: color.r,
				g: color.g,
				b: color.b,
				a: color.a + alpha,
			}),
			WguiColor::Named(color) => WguiColor::Named(WguiNamedColor {
				name: color.name,
				rgb_multiplier: color.rgb_multiplier,
				rgb_addition: color.rgb_addition,
				alpha: color.alpha + alpha,
			}),
		}
	}

	#[must_use]
	pub const fn with_alpha(&self, alpha: f32) -> WguiColor {
		match self {
			WguiColor::Raw(color) => WguiColor::Raw(color.with_alpha(alpha)),
			WguiColor::Named(color) => WguiColor::Named(WguiNamedColor {
				name: color.name,
				rgb_multiplier: color.rgb_multiplier,
				rgb_addition: color.rgb_addition,
				alpha,
			}),
		}
	}

	// returns named colors if val is 0 or 1, raw otherwise
	#[must_use]
	pub fn lerp(&self, palette: &WguiColorPalette, other: &WguiColor, val: f32) -> WguiColor {
		if val <= 0.0 {
			*self
		} else if val >= 1.0 {
			*other
		} else {
			let c1 = self.resolve(palette);
			let c2 = other.resolve(palette);
			c1.lerp(&c2, val).into()
		}
	}

	/// Gets a matching foreground color from a background color
	pub fn fg_color(&self) -> Option<Self> {
		if let Self::Named(name) = self {
			name.name.fg_color().map(Into::into)
		} else {
			None
		}
	}

	/// Gets a matching background color from a foreground color
	pub fn bg_color(&self) -> Option<Self> {
		if let Self::Named(name) = self {
			name.name.bg_color().map(Into::into)
		} else {
			None
		}
	}
}

impl WguiColorName {
	pub const fn fg_color(&self) -> Option<Self> {
		match self {
			Self::Primary => Some(Self::OnPrimary),
			Self::Secondary => Some(Self::OnSecondary),
			Self::Tertiary => Some(Self::OnTertiary),
			Self::Danger => Some(Self::OnDanger),
			Self::Background => Some(Self::OnBackground),
			Self::BackgroundVariant => Some(Self::OnBackgroundVariant),
			Self::BackgroundContrast => Some(Self::OnBackgroundContrast),
			_ => None,
		}
	}

	pub const fn bg_color(&self) -> Option<Self> {
		match self {
			Self::OnPrimary => Some(Self::Primary),
			Self::OnSecondary => Some(Self::Secondary),
			Self::OnTertiary => Some(Self::Tertiary),
			Self::OnDanger => Some(Self::Danger),
			Self::OnBackground => Some(Self::Background),
			Self::OnBackgroundVariant => Some(Self::BackgroundVariant),
			Self::OnBackgroundContrast => Some(Self::BackgroundContrast),
			_ => None,
		}
	}

	pub const fn to_wgui_color(&self) -> WguiColor {
		WguiColor::Named(WguiNamedColor {
			name: *self,
			alpha: 1.0,
			rgb_multiplier: 1.0,
			rgb_addition: 0.0,
		})
	}
}

impl From<WguiColorName> for WguiColor {
	fn from(name: WguiColorName) -> Self {
		name.to_wgui_color()
	}
}

impl From<drawing::Color> for WguiColor {
	fn from(color: drawing::Color) -> Self {
		WguiColor::Raw(color)
	}
}

impl Default for WguiColor {
	fn default() -> Self {
		Self::Raw(Default::default())
	}
}

impl WguiNamedColor {
	pub const fn with_alpha(name: WguiColorName, alpha: f32) -> WguiNamedColor {
		WguiNamedColor {
			name,
			rgb_multiplier: 1.0,
			rgb_addition: 0.0,
			alpha,
		}
	}

	pub fn resolve(&self, palette: &WguiColorPalette) -> drawing::Color {
		let mut color = palette[self.name];
		if self.alpha != 1.0 {
			color.a *= self.alpha;
		}

		if self.rgb_multiplier != 1.0 {
			color.r *= self.rgb_multiplier;
			color.g *= self.rgb_multiplier;
			color.b *= self.rgb_multiplier;
		}

		if self.rgb_addition != 0.0 {
			color.r += self.rgb_addition;
			color.g += self.rgb_addition;
			color.b += self.rgb_addition;
		}

		color
	}
}
