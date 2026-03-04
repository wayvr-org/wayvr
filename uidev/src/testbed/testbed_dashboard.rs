use std::path::PathBuf;

use crate::{
	assets,
	testbed::{Testbed, TestbedUpdateParams},
};
use dash_frontend::frontend::{self, FrontendUpdateParams};
use wgui::{font_config::WguiFontConfig, globals::WguiGlobals, layout::Layout};
use wlx_common::{dash_interface_emulated::DashInterfaceEmulated, locale::WayVRLangProvider};

pub struct TestbedDashboard {
	frontend: frontend::Frontend<()>,
}

impl TestbedDashboard {
	pub fn new(assets: Box<assets::Asset>) -> anyhow::Result<Self> {
		let interface = DashInterfaceEmulated::new();
		let lang_provider = WayVRLangProvider::default();
		let globals = WguiGlobals::new(
			assets,
			&lang_provider,
			wgui::globals::Defaults::default(),
			&WguiFontConfig::default(),
			PathBuf::new(), // cwd
		)?;

		let frontend = frontend::Frontend::new(
			frontend::InitParams {
				interface: Box::new(interface),
				has_monado: true,
				globals,
				lang_provider: &lang_provider,
			},
		)?;
		Ok(Self { frontend })
	}
}

impl Testbed for TestbedDashboard {
	fn update(&mut self, params: TestbedUpdateParams) -> anyhow::Result<()> {
		let res = self.frontend.update(FrontendUpdateParams {
			data: &mut (), /* nothing */
			width: params.width,
			height: params.height,
			timestep_alpha: params.timestep_alpha,
		})?;
		self
			.frontend
			.process_update(res, params.audio_system, params.audio_sample_player)?;
		Ok(())
	}

	fn layout(&mut self) -> &mut Layout {
		&mut self.frontend.layout
	}
}
