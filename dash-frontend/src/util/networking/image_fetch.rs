use std::rc::Rc;

use wgui::{globals::WguiGlobals, renderer_vk::text::custom_glyph::CustomGlyphData};
use wlx_common::async_executor::AsyncExecutor;

use crate::util::networking::http_client;

pub async fn fetch_to_glyph_data(
	globals: &WguiGlobals,
	executor: &AsyncExecutor,
	url: &str,
) -> anyhow::Result<(CustomGlyphData, Rc<Vec<u8>>)> {
	let res = http_client::get_simple(executor, url).await?;
	let glyph_data = CustomGlyphData::from_bytes_raster(globals, url, &res.data)?;
	Ok((glyph_data, Rc::new(res.data)))
}
