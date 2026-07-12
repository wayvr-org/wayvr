use std::rc::Rc;

use wgui::{globals::WguiGlobals, renderer_vk::text::custom_glyph::CustomGlyphData};

use crate::util::networking::http_client;

pub async fn fetch_to_glyph_data(globals: &WguiGlobals, url: &str) -> anyhow::Result<(CustomGlyphData, Rc<Vec<u8>>)> {
	let res = http_client::get_simple(url).await?;
	let glyph_data = CustomGlyphData::from_bytes_raster(globals, url, &res.data)?;
	Ok((glyph_data, Rc::new(res.data)))
}
