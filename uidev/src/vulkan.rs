use std::sync::Arc;

use wgui::gfx::{WGfx, WGfxCreateInfo};
use winit::{event_loop::EventLoop, window::Window};

#[allow(clippy::type_complexity)]
pub fn init_window(title: &str) -> anyhow::Result<(Arc<WGfx>, EventLoop<()>, Arc<Window>)> {
	let event_loop = EventLoop::new().unwrap(); // want panic

	#[allow(deprecated)]
	let window = Arc::new(
		event_loop
			.create_window(
				Window::default_attributes()
					.with_transparent(true)
					.with_title(title),
			)
			.unwrap(), // want panic
	);

	let gfx = WGfx::new_for_window(
		window.as_ref(),
		&WGfxCreateInfo {
			application_name: "uidev".to_owned(),
			engine_name: "wgui".to_owned(),
			..Default::default()
		},
	)?;

	log::info!("Using vkPhysicalDevice: {}", gfx.device_info().name);
	if gfx.capabilities().filter_cubic {
		log::info!("cubic filtering available");
	}

	Ok((gfx, event_loop, window))
}
