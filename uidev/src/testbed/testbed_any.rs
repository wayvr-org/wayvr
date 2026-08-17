use std::path::PathBuf;

use crate::{
	assets,
	testbed::{Testbed, TestbedUpdateParams},
};
use glam::Vec2;
use wgui::{
	assets::AssetPathRef,
	font_config::WguiFontConfig,
	globals::WguiGlobals,
	layout::{Layout, LayoutParams, LayoutUpdateParams},
	palette::WguiColorPalette,
	parser::{ParseDocumentExtra, ParseDocumentParams, ParserState},
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
			AssetPathRef::FileOrBuiltIn(name)
		} else {
			AssetPathRef::BuiltIn(&format!("gui/{name}.xml"))
		};

		let lang_provider = WayVRLangProvider::default();
		let palette_name = std::env::var("PALETTE").unwrap_or_else(|_| "Default".to_string());

		let globals = WguiGlobals::new(
			assets,
			&lang_provider,
			&WguiFontConfig::default(),
			PathBuf::new(), // cwd
			WguiColorPalette::get_builtin(&palette_name),
		)?;

		let (layout, state) = wgui::parser::new_layout_from_assets(
			&ParseDocumentParams {
				globals,
				path,
				extra: ParseDocumentExtra {
					root_dir: Some(path.to_rc().strip_filename()),
					..Default::default()
				},
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
