use std::ops::Index;

use strum::EnumCount;

use crate::{
	color::{WguiColor, WguiColorName},
	drawing,
};

#[derive(Clone)]
pub struct WguiColorPalette {
	colors: [drawing::Color; WguiColorName::COUNT],
}

impl WguiColorPalette {
	pub fn get_builtin(palette_name: &str) -> Self {
		for (name, palette) in PALETTES {
			if *name == palette_name {
				// just clone here, makes it easier for future customization
				return (*palette).clone();
			}
		}

		log::warn!("No color palette with name '{palette_name}', using default colors.");
		DEFAULT.clone()
	}

	pub const fn from_inner(colors: [drawing::Color; WguiColorName::COUNT]) -> Self {
		Self { colors }
	}

	fn apply_modifier(color: WguiColor, modifier: &str) -> Option<WguiColor> {
		match modifier {
			"transparent" => Some(color.with_alpha(0.0)),
			"opaque" => Some(color.with_alpha(1.0)),
			other => {
				let (prefix, val_str) = other.rsplit_once('-')?;
				let val = val_str.parse::<f32>().ok()?;
				match prefix {
					"opacity" => Some(color.with_alpha(val)),
					"rgb-mult" => Some(color.mult_rgb(val)),
					"rgb-add" => Some(color.add_rgb(val)),
					_ => None,
				}
			}
		}
	}

	pub fn find(&self, in_name: &str) -> Option<WguiColor> {
		if let Some((base, modifiers_str)) = in_name.split_once('(') {
			let base_name = base.trim();
			let modifiers = modifiers_str.trim_end_matches(')');

			let name = WguiColorName::try_from(base_name).ok()?;
			let mut color = name.to_wgui_color();

			for mod_str in modifiers.split(',') {
				color = WguiColorPalette::apply_modifier(color, mod_str.trim())?;
			}

			Some(color)
		} else {
			WguiColorName::try_from(in_name).ok().map(|n| n.to_wgui_color())
		}
	}
}

impl Index<WguiColorName> for WguiColorPalette {
	type Output = drawing::Color;

	fn index(&self, name: WguiColorName) -> &Self::Output {
		// internally guaranteed to never be out of range
		&self.colors[name as usize]
	}
}

// palettes below

const fn hex(hex: &str) -> drawing::Color {
	match drawing::Color::from_hex(hex) {
		Some(color) => color,
		// just fail the build
		None => panic!("invalid static color"),
	}
}

pub static PALETTES: &[(&'static str, &WguiColorPalette)] = &[
	("Default", DEFAULT),
	("Ayu Dusk", AYU),
	("Catppuccin", CATPPUCCIN),
	("Cyberpunk", CYBERPUNK),
	("Dracula", DRACULA),
	("Eldritch", ELDRITCH),
	("Everforest", EVERFOREST),
	("Gruvbox", GRUVBOX),
	("Kanagawa", KANAGAWA),
	("Monochrome", MONOCHROME),
	("Nord", NORD),
	("Osaka Jade", OSAKA_JADE),
	("Rosé Pine", ROSEPINE),
	("Solarized Night", SOLARIZED_NIGHT),
	("Tokyo Night", TOKYO_NIGHT),
	("Ayu Day", AYU_LIGHT),
	("Catppuccin Latte", CATPPUCCIN_LATTE),
	("Cyberpunk Light", CYBERPUNK_LIGHT),
	("Alucard", DRACULA_LIGHT),
	("Eldritch Dusk", ELDRITCH_LIGHT),
	("Everforest Light", EVERFOREST_LIGHT),
	("Gruvbox Light", GRUVBOX_LIGHT),
	("Kanagawa Lotus", KANAGAWA_LIGHT),
	("Monochrome Light", MONOCHROME_LIGHT),
	("Nord Light", NORD_LIGHT),
	("Osaka Jade Light", OSAKA_JADE_LIGHT),
	("Rosé Pine Dawn", ROSEPINE_LIGHT),
	("Solarized Day", SOLARIZED_DAY),
	("Tokyo Day", TOKYO_DAY),
];

static DEFAULT: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#21adff"), // Primary
		hex("#eaf7ff"), // OnPrimary
		hex("#f4c95d"), // Secondary
		hex("#2a2105"), // OnSecondary
		hex("#10d0b3"), // Tertiary
		hex("#000000"), // OnTertiary
		hex("#f7469a"), // Danger
		hex("#ffebf5"), // OnDanger
		hex("#002e43"), // Background
		hex("#e4f5f6"), // OnBackground
		hex("#0c5170"), // OackgroundVariant
		hex("#b5cacc"), // OnBackgroundVariant
		hex("#00131c"), // BackgroundContrast
		hex("#e4edf6"), // OnBackgroundContrast
		hex("#1c6788"), // Outline
		hex("#000000"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static AYU: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#E6B450"), // Primary
		hex("#0B0E14"), // OnPrimary
		hex("#AAD94C"), // Secondary
		hex("#0B0E14"), // OnSecondary
		hex("#39BAE6"), // Tertiary
		hex("#0B0E14"), // OnTertiary
		hex("#D95757"), // Danger
		hex("#0B0E14"), // OnDanger
		hex("#1E222A"), // Background
		hex("#BFBDB6"), // OnBackground
		hex("#0B0E14"), // BackgroundVariant
		hex("#636A72"), // OnBackgroundVariant
		hex("#07090D"), // BackgroundContrast
		hex("#BFBDB6"), // OnBackgroundContrast
		hex("#565B66"), // Outline
		hex("#000000"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static AYU_LIGHT: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#FF8F40"), // Primary
		hex("#F8F9FA"), // OnPrimary
		hex("#86B300"), // Secondary
		hex("#F8F9FA"), // OnSecondary
		hex("#55B4D4"), // Tertiary
		hex("#F8F9FA"), // OnTertiary
		hex("#E65050"), // Danger
		hex("#F8F9FA"), // OnDanger
		hex("#E4E6E9"), // Background
		hex("#5C6166"), // OnBackground
		hex("#F8F9FA"), // BackgroundVariant
		hex("#ABADB1"), // OnBackgroundVariant
		hex("#D5D7DA"), // BackgroundContrast
		hex("#5C6166"), // OnBackgroundContrast
		hex("#8A9199"), // Outline
		hex("#F8F9FA"), // Shadow
		hex("#000000"), // Highlight
	],
};

static CATPPUCCIN: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#cba6f7"), // Primary
		hex("#11111b"), // OnPrimary
		hex("#fab387"), // Secondary
		hex("#11111b"), // OnSecondary
		hex("#94e2d5"), // Tertiary
		hex("#11111b"), // OnTertiary
		hex("#f38ba8"), // Danger
		hex("#11111b"), // OnDanger
		hex("#1e1e2e"), // Background
		hex("#cdd6f4"), // OnBackground
		hex("#313244"), // BackgroundVariant
		hex("#a3b4eb"), // OnBackgroundVariant
		hex("#181825"), // BackgroundContrast
		hex("#cdd6f4"), // OnBackgroundContrast
		hex("#4c4f69"), // Outline
		hex("#11111b"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static CATPPUCCIN_LATTE: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#8839EF"), // Primary
		hex("#EFF1F5"), // OnPrimary
		hex("#FE640B"), // Secondary
		hex("#EFF1F5"), // OnSecondary
		hex("#179299"), // Tertiary
		hex("#EFF1F5"), // OnTertiary
		hex("#D20F39"), // Danger
		hex("#DCE0E8"), // OnDanger
		hex("#EFF1F5"), // Background
		hex("#4C4F69"), // OnBackground
		hex("#CCD0DA"), // BackgroundVariant
		hex("#6C6F85"), // OnBackgroundVariant
		hex("#DCE0E8"), // BackgroundContrast
		hex("#4C4F69"), // OnBackgroundContrast
		hex("#A5ADCB"), // Outline
		hex("#DCE0E8"), // Shadow
		hex("#000000"), // Highlight
	],
};

static CYBERPUNK: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#C4A82E"), // Primary
		hex("#0E1015"), // OnPrimary
		hex("#D14358"), // Secondary
		hex("#0E1015"), // OnSecondary
		hex("#00A66C"), // Tertiary
		hex("#0E1015"), // OnTertiary
		hex("#B32D2D"), // Danger
		hex("#0E1015"), // OnDanger
		hex("#0C1017"), // Background
		hex("#5C8AC4"), // OnBackground
		hex("#11151D"), // BackgroundVariant
		hex("#9B6BC1"), // OnBackgroundVariant
		hex("#070A0F"), // BackgroundContrast
		hex("#5C8AC4"), // OnBackgroundContrast
		hex("#45A0D6"), // Outline
		hex("#090D13"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static CYBERPUNK_LIGHT: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#00B8B3"), // Primary
		hex("#1A1914"), // OnPrimary
		hex("#D957A0"), // Secondary
		hex("#1A1914"), // OnSecondary
		hex("#45D395"), // Tertiary
		hex("#1A1914"), // OnTertiary
		hex("#E63E5D"), // Danger
		hex("#1A1914"), // OnDanger
		hex("#DAE6E8"), // Background
		hex("#1A1914"), // OnBackground
		hex("#C8DEE6"), // BackgroundVariant
		hex("#1A1914"), // OnBackgroundVariant
		hex("#B8D4E6"), // BackgroundContrast
		hex("#1A1914"), // OnBackgroundContrast
		hex("#7B52AB"), // Outline
		hex("#B8D4E6"), // Shadow
		hex("#000000"), // Highlight
	],
};

static DRACULA: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#BD93F9"), // Primary
		hex("#282A36"), // OnPrimary
		hex("#FF79C6"), // Secondary
		hex("#4E1D32"), // OnSecondary
		hex("#8BE9FD"), // Tertiary
		hex("#003543"), // OnTertiary
		hex("#FF5555"), // Danger
		hex("#282A36"), // OnDanger
		hex("#282A36"), // Background
		hex("#F8F8F2"), // OnBackground
		hex("#44475A"), // BackgroundVariant
		hex("#D6D8E0"), // OnBackgroundVariant
		hex("#1E1F29"), // BackgroundContrast
		hex("#F8F8F2"), // OnBackgroundContrast
		hex("#5A5E77"), // Outline
		hex("#282A36"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static DRACULA_LIGHT: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#8332F4"), // Primary
		hex("#FFFFFF"), // OnPrimary
		hex("#FF1399"), // Secondary
		hex("#FFFFFF"), // OnSecondary
		hex("#0398B9"), // Tertiary
		hex("#FFFFFF"), // OnTertiary
		hex("#FF5555"), // Danger
		hex("#282A36"), // OnDanger
		hex("#F8F8F2"), // Background
		hex("#282A36"), // OnBackground
		hex("#E6E6EA"), // BackgroundVariant
		hex("#44475A"), // OnBackgroundVariant
		hex("#FFFFFF"), // BackgroundContrast
		hex("#282A36"), // OnBackgroundContrast
		hex("#CACAD3"), // Outline
		hex("#D6D8E0"), // Shadow
		hex("#000000"), // Highlight
	],
};

static ELDRITCH: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#37F499"), // Primary
		hex("#171928"), // OnPrimary
		hex("#04D1F9"), // Secondary
		hex("#171928"), // OnSecondary
		hex("#A48CF2"), // Tertiary
		hex("#171928"), // OnTertiary
		hex("#F16C75"), // Danger
		hex("#171928"), // OnDanger
		hex("#212337"), // Background
		hex("#EBFAFA"), // OnBackground
		hex("#292E42"), // BackgroundVariant
		hex("#ABB4DA"), // OnBackgroundVariant
		hex("#171928"), // BackgroundContrast
		hex("#EBFAFA"), // OnBackgroundContrast
		hex("#3B4261"), // Outline
		hex("#414868"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static ELDRITCH_LIGHT: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#37F499"), // Primary
		hex("#171928"), // OnPrimary
		hex("#04D1F9"), // Secondary
		hex("#171928"), // OnSecondary
		hex("#A48CF2"), // Tertiary
		hex("#171928"), // OnTertiary
		hex("#F16C75"), // Danger
		hex("#171928"), // OnDanger
		hex("#FFFFFF"), // Background
		hex("#171928"), // OnBackground
		hex("#F2F4F8"), // BackgroundVariant
		hex("#3B4261"), // OnBackgroundVariant
		hex("#E0E3E8"), // BackgroundContrast
		hex("#171928"), // OnBackgroundContrast
		hex("#B0B6C3"), // Outline
		hex("#E0E3E8"), // Shadow
		hex("#000000"), // Highlight
	],
};

static EVERFOREST: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#D3C6AA"), // Primary
		hex("#232A2E"), // OnPrimary
		hex("#D3C6AA"), // Secondary
		hex("#232A2E"), // OnSecondary
		hex("#9DA9A0"), // Tertiary
		hex("#232A2E"), // OnTertiary
		hex("#E67E80"), // Danger
		hex("#232A2E"), // OnDanger
		hex("#232A2E"), // Background
		hex("#859289"), // OnBackground
		hex("#2D353B"), // BackgroundVariant
		hex("#D3C6AA"), // OnBackgroundVariant
		hex("#1E2326"), // BackgroundContrast
		hex("#D3C6AA"), // OnBackgroundContrast
		hex("#D3C6AA"), // Outline
		hex("#475258"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static EVERFOREST_LIGHT: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#434F55"), // Primary
		hex("#D3C6AA"), // OnPrimary
		hex("#232A2E"), // Secondary
		hex("#D3C6AA"), // OnSecondary
		hex("#333C43"), // Tertiary
		hex("#9DA9A0"), // OnTertiary
		hex("#E66868"), // Danger
		hex("#9DA9A0"), // OnDanger
		hex("#9DA9A0"), // Background
		hex("#232A2E"), // OnBackground
		hex("#BEC5B2"), // BackgroundVariant
		hex("#333C43"), // OnBackgroundVariant
		hex("#859289"), // BackgroundContrast
		hex("#232A2E"), // OnBackgroundContrast
		hex("#232A2E"), // Outline
		hex("#ECF5ED"), // Shadow
		hex("#000000"), // Highlight
	],
};

static GRUVBOX: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#B8BB26"), // Primary
		hex("#282828"), // OnPrimary
		hex("#FABD2F"), // Secondary
		hex("#282828"), // OnSecondary
		hex("#83A598"), // Tertiary
		hex("#282828"), // OnTertiary
		hex("#FB4934"), // Danger
		hex("#282828"), // OnDanger
		hex("#282828"), // Background
		hex("#FBF1C7"), // OnBackground
		hex("#3C3836"), // BackgroundVariant
		hex("#EBDBB2"), // OnBackgroundVariant
		hex("#1D2021"), // BackgroundContrast
		hex("#FBF1C7"), // OnBackgroundContrast
		hex("#57514E"), // Outline
		hex("#282828"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static GRUVBOX_LIGHT: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#98971A"), // Primary
		hex("#FBF1C7"), // OnPrimary
		hex("#D79921"), // Secondary
		hex("#FBF1C7"), // OnSecondary
		hex("#458588"), // Tertiary
		hex("#FBF1C7"), // OnTertiary
		hex("#CC241D"), // Danger
		hex("#FBF1C7"), // OnDanger
		hex("#FBF1C7"), // Background
		hex("#3C3836"), // OnBackground
		hex("#EBDBB2"), // BackgroundVariant
		hex("#7C6F64"), // OnBackgroundVariant
		hex("#F2E5BC"), // BackgroundContrast
		hex("#3C3836"), // OnBackgroundContrast
		hex("#BDAE93"), // Outline
		hex("#D5C4A1"), // Shadow
		hex("#000000"), // Highlight
	],
};

static KANAGAWA: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#76946A"), // Primary
		hex("#1F1F28"), // OnPrimary
		hex("#C0A36E"), // Secondary
		hex("#1F1F28"), // OnSecondary
		hex("#7E9CD8"), // Tertiary
		hex("#1F1F28"), // OnTertiary
		hex("#C34043"), // Danger
		hex("#1F1F28"), // OnDanger
		hex("#1F1F28"), // Background
		hex("#C8C093"), // OnBackground
		hex("#2A2A37"), // BackgroundVariant
		hex("#717C7C"), // OnBackgroundVariant
		hex("#16161D"), // BackgroundContrast
		hex("#C8C093"), // OnBackgroundContrast
		hex("#363646"), // Outline
		hex("#1F1F28"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static KANAGAWA_LIGHT: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#6F894E"), // Primary
		hex("#F2ECBC"), // OnPrimary
		hex("#77713F"), // Secondary
		hex("#F2ECBC"), // OnSecondary
		hex("#4D699B"), // Tertiary
		hex("#F2ECBC"), // OnTertiary
		hex("#C84053"), // Danger
		hex("#F2ECBC"), // OnDanger
		hex("#F2ECBC"), // Background
		hex("#545464"), // OnBackground
		hex("#E5DDB0"), // BackgroundVariant
		hex("#8A8980"), // OnBackgroundVariant
		hex("#D5CEA3"), // BackgroundContrast
		hex("#545464"), // OnBackgroundContrast
		hex("#CFC49C"), // Outline
		hex("#F2ECBC"), // Shadow
		hex("#000000"), // Highlight
	],
};

static MONOCHROME: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#AAAAAA"), // Primary
		hex("#111111"), // OnPrimary
		hex("#A7A7A7"), // Secondary
		hex("#111111"), // OnSecondary
		hex("#CCCCCC"), // Tertiary
		hex("#111111"), // OnTertiary
		hex("#DDDDDD"), // Danger
		hex("#111111"), // OnDanger
		hex("#111111"), // Background
		hex("#828282"), // OnBackground
		hex("#191919"), // BackgroundVariant
		hex("#5D5D5D"), // OnBackgroundVariant
		hex("#080808"), // BackgroundContrast
		hex("#828282"), // OnBackgroundContrast
		hex("#3C3C3C"), // Outline
		hex("#000000"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static MONOCHROME_LIGHT: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#555555"), // Primary
		hex("#EEEEEE"), // OnPrimary
		hex("#505058"), // Secondary
		hex("#EEEEEE"), // OnSecondary
		hex("#333333"), // Tertiary
		hex("#EEEEEE"), // OnTertiary
		hex("#222222"), // Danger
		hex("#EFEFEF"), // OnDanger
		hex("#D4D4D4"), // Background
		hex("#696969"), // OnBackground
		hex("#E8E8E8"), // BackgroundVariant
		hex("#9E9E9E"), // OnBackgroundVariant
		hex("#BEBEBE"), // BackgroundContrast
		hex("#555555"), // OnBackgroundContrast
		hex("#C3C3C3"), // Outline
		hex("#FAFAFA"), // Shadow
		hex("#000000"), // Highlight
	],
};

static NORD: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#8FBCBB"), // Primary
		hex("#2E3440"), // OnPrimary
		hex("#88C0D0"), // Secondary
		hex("#2E3440"), // OnSecondary
		hex("#5E81AC"), // Tertiary
		hex("#2E3440"), // OnTertiary
		hex("#BF616A"), // Danger
		hex("#2E3440"), // OnDanger
		hex("#2E3440"), // Background
		hex("#ECEFF4"), // OnBackground
		hex("#3B4252"), // BackgroundVariant
		hex("#D8DEE9"), // OnBackgroundVariant
		hex("#242933"), // BackgroundContrast
		hex("#ECEFF4"), // OnBackgroundContrast
		hex("#505A70"), // Outline
		hex("#2E3440"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static NORD_LIGHT: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#5E81AC"), // Primary
		hex("#ECEFF4"), // OnPrimary
		hex("#64ADC2"), // Secondary
		hex("#ECEFF4"), // OnSecondary
		hex("#6FA9A8"), // Tertiary
		hex("#ECEFF4"), // OnTertiary
		hex("#BF616A"), // Danger
		hex("#ECEFF4"), // OnDanger
		hex("#ECEFF4"), // Background
		hex("#2E3440"), // OnBackground
		hex("#E5E9F0"), // BackgroundVariant
		hex("#4C566A"), // OnBackgroundVariant
		hex("#D8DEE9"), // BackgroundContrast
		hex("#2E3440"), // OnBackgroundContrast
		hex("#C5CEDD"), // Outline
		hex("#D8DEE9"), // Shadow
		hex("#000000"), // Highlight
	],
};

static OSAKA_JADE: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#1E9177"), // Primary
		hex("#B8C8C4"), // OnPrimary
		hex("#167A63"), // Secondary
		hex("#B8C8C4"), // OnSecondary
		hex("#26A589"), // Tertiary
		hex("#B8C8C4"), // OnTertiary
		hex("#933636"), // Danger
		hex("#B8C8C4"), // OnDanger
		hex("#081512"), // Background
		hex("#A6B5B1"), // OnBackground
		hex("#0F251F"), // BackgroundVariant
		hex("#99A8A4"), // OnBackgroundVariant
		hex("#040A09"), // BackgroundContrast
		hex("#A6B5B1"), // OnBackgroundContrast
		hex("#1B6352"), // Outline
		hex("#040A09"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static OSAKA_JADE_LIGHT: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#3B7561"), // Primary
		hex("#D8E5DB"), // OnPrimary
		hex("#526E4A"), // Secondary
		hex("#D8E5DB"), // OnSecondary
		hex("#4A8069"), // Tertiary
		hex("#D8E5DB"), // OnTertiary
		hex("#854145"), // Danger
		hex("#D8E5DB"), // OnDanger
		hex("#AEC2B4"), // Background
		hex("#2C3D35"), // OnBackground
		hex("#95AD9C"), // BackgroundVariant
		hex("#263731"), // OnBackgroundVariant
		hex("#83998B"), // BackgroundContrast
		hex("#263731"), // OnBackgroundContrast
		hex("#5C7A6A"), // Outline
		hex("#8A9E90"), // Shadow
		hex("#000000"), // Highlight
	],
};

static ROSEPINE: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#EBBCBA"), // Primary
		hex("#191724"), // OnPrimary
		hex("#9CCFD8"), // Secondary
		hex("#191724"), // OnSecondary
		hex("#31748F"), // Tertiary
		hex("#E0DEF4"), // OnTertiary
		hex("#EB6F92"), // Danger
		hex("#191724"), // OnDanger
		hex("#1F1D2E"), // Background
		hex("#E0DEF4"), // OnBackground
		hex("#26233A"), // BackgroundVariant
		hex("#908CAA"), // OnBackgroundVariant
		hex("#191724"), // BackgroundContrast
		hex("#E0DEF4"), // OnBackgroundContrast
		hex("#403D52"), // Outline
		hex("#191724"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static ROSEPINE_LIGHT: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#D7827E"), // Primary
		hex("#FAF4ED"), // OnPrimary
		hex("#56949F"), // Secondary
		hex("#FAF4ED"), // OnSecondary
		hex("#286983"), // Tertiary
		hex("#FAF4ED"), // OnTertiary
		hex("#B4637A"), // Danger
		hex("#FAF4ED"), // OnDanger
		hex("#FFFAF3"), // Background
		hex("#575279"), // OnBackground
		hex("#F2E9E1"), // BackgroundVariant
		hex("#797593"), // OnBackgroundVariant
		hex("#DFDAD9"), // BackgroundContrast
		hex("#575279"), // OnBackgroundContrast
		hex("#DFDAD9"), // Outline
		hex("#FAF4ED"), // Shadow
		hex("#000000"), // Highlight
	],
};

static SOLARIZED_NIGHT: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#B58900"), // Primary
		hex("#002B36"), // OnPrimary
		hex("#D33682"), // Secondary
		hex("#002B36"), // OnSecondary
		hex("#CB4B16"), // Tertiary
		hex("#002B36"), // OnTertiary
		hex("#DC322F"), // Danger
		hex("#002B36"), // OnDanger
		hex("#002B36"), // Background
		hex("#839496"), // OnBackground
		hex("#073642"), // BackgroundVariant
		hex("#657B83"), // OnBackgroundVariant
		hex("#001F27"), // BackgroundContrast
		hex("#839496"), // OnBackgroundContrast
		hex("#0C5C70"), // Outline
		hex("#002B36"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static SOLARIZED_DAY: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#B58900"), // Primary
		hex("#FDF6E3"), // OnPrimary
		hex("#D33682"), // Secondary
		hex("#FDF6E3"), // OnSecondary
		hex("#CB4B16"), // Tertiary
		hex("#FDF6E3"), // OnTertiary
		hex("#DC322F"), // Danger
		hex("#FDF6E3"), // OnDanger
		hex("#FDF6E3"), // Background
		hex("#657B83"), // OnBackground
		hex("#EEE8D5"), // BackgroundVariant
		hex("#839496"), // OnBackgroundVariant
		hex("#DFD4B1"), // BackgroundContrast
		hex("#657B83"), // OnBackgroundContrast
		hex("#DFD4B1"), // Outline
		hex("#EEE8D5"), // Shadow
		hex("#000000"), // Highlight
	],
};

static TOKYO_NIGHT: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#7AA2F7"), // Primary
		hex("#16161E"), // OnPrimary
		hex("#BB9AF7"), // Secondary
		hex("#16161E"), // OnSecondary
		hex("#9ECE6A"), // Tertiary
		hex("#16161E"), // OnTertiary
		hex("#F7768E"), // Danger
		hex("#16161E"), // OnDanger
		hex("#1A1B26"), // Background
		hex("#C0CAF5"), // OnBackground
		hex("#24283B"), // BackgroundVariant
		hex("#9AA5CE"), // OnBackgroundVariant
		hex("#15161E"), // BackgroundContrast
		hex("#C0CAF5"), // OnBackgroundContrast
		hex("#565F89"), // Outline
		hex("#15161E"), // Shadow
		hex("#ffffff"), // Highlight
	],
};

static TOKYO_DAY: &WguiColorPalette = &WguiColorPalette {
	colors: [
		hex("#2E7DE9"), // Primary
		hex("#E1E2E7"), // OnPrimary
		hex("#9854F1"), // Secondary
		hex("#E1E2E7"), // OnSecondary
		hex("#587539"), // Tertiary
		hex("#E1E2E7"), // OnTertiary
		hex("#F52A65"), // Danger
		hex("#E1E2E7"), // OnDanger
		hex("#E1E2E7"), // Background
		hex("#3760BF"), // OnBackground
		hex("#D0D5E3"), // BackgroundVariant
		hex("#6172B0"), // OnBackgroundVariant
		hex("#BCC2D1"), // BackgroundContrast
		hex("#3760BF"), // OnBackgroundContrast
		hex("#B4B5B9"), // Outline
		hex("#A8AECB"), // Shadow
		hex("#000000"), // Highlight
	],
};
