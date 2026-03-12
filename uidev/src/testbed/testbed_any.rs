use std::path::PathBuf;

use crate::{
	assets,
	testbed::{Testbed, TestbedUpdateParams},
};
use glam::Vec2;
use wgui::{
	assets::AssetPath,
	font_config::WguiFontConfig,
	globals::WguiGlobals,
	layout::{Layout, LayoutParams, LayoutUpdateParams},
	parser::{ParseDocumentParams, ParserState},
};
use wlx_common::locale::WayVRLangProvider;

pub struct TestbedAny {
	pub layout: Layout,

	#[allow(dead_code)]
	state: ParserState,
}

impl TestbedAny {
	pub fn new(assets: Box<assets::Asset>, name: &str) -> anyhow::Result<Self> {
		let path = if name.ends_with(".xml") {
			AssetPath::FileOrBuiltIn(name)
		} else {
			AssetPath::BuiltIn(&format!("gui/{name}.xml"))
		};

		let lang_provider = WayVRLangProvider::default();

		let globals = WguiGlobals::new(
			assets,
			&lang_provider,
			&WguiFontConfig::default(),
			PathBuf::new(), // cwd
		)?;

		let (layout, state) = wgui::parser::new_layout_from_assets(
			&ParseDocumentParams {
				globals,
				path,
				extra: Default::default(),
			},
			LayoutParams::default(),
		)?;
		Ok(Self { layout, state })
	}
}

impl Testbed for TestbedAny {
	fn update(&mut self, mut params: TestbedUpdateParams) -> anyhow::Result<()> {
		let res = self.layout.update(&mut LayoutUpdateParams {
			size: Vec2::new(params.width, params.height),
			timestep_alpha: params.timestep_alpha,
		})?;
		params.process_layout_result(res);
		Ok(())
	}

	fn layout(&mut self) -> &mut Layout {
		&mut self.layout
	}
}
