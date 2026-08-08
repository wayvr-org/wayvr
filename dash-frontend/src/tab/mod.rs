use strum::EnumCount;

use crate::frontend::Frontend;

pub mod apps;
pub mod donate;
pub mod games;
pub mod home;
pub mod monado;
pub mod settings;
pub mod welcome;

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumCount)]
pub enum TabType {
	Home,
	Apps,
	Games,
	Monado,
	Settings,
	Welcome,
	Donate,
}

impl TabType {
	pub fn get_preferred_padding(&self) -> f32 {
		match self {
			TabType::Welcome => 0.0,
			_ => 16.0,
		}
	}
}

pub trait Tab<T> {
	#[allow(dead_code)]
	fn get_type(&self) -> TabType;

	fn update(&mut self, _frontend: &mut Frontend<T>, _time_ms: u32, _user_data: &mut T) -> anyhow::Result<()> {
		Ok(())
	}
}
