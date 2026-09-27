use glam::{Vec2, vec2};
use std::rc::Rc;
use testbed::{Testbed, testbed_any::TestbedAny};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use vulkan::init_window;
use wgui::{
	event::{MouseButtonEvent, MouseButtonIndex, MouseMotionEvent, MouseWheelEvent},
	gfx::{CommandBufferUsage, PresentStatus, SwapchainError, WGfxClearMode},
	renderer_vk::{self},
};
use winit::{
	event::{ElementState, Event, MouseScrollDelta, WindowEvent},
	event_loop::ControlFlow,
	keyboard::{KeyCode, PhysicalKey},
};
use wlx_common::{audio, timestep::Timestep};

use crate::{
	rate_limiter::RateLimiter,
	testbed::{
		TestbedUpdateParams, testbed_dashboard::TestbedDashboard, testbed_generic::TestbedGeneric,
	},
};

mod assets;
mod profiler;
mod rate_limiter;
mod testbed;
mod vulkan;

fn init_logging() {
	tracing_subscriber::registry()
		.with(
			tracing_subscriber::fmt::layer()
				.pretty()
				.with_writer(std::io::stderr),
		)
		.with(
			/* read RUST_LOG env var */
			EnvFilter::builder()
				.with_default_directive(LevelFilter::DEBUG.into())
				.from_env_lossy()
				.add_directive("cosmic_text=info".parse().unwrap()),
		)
		.init();
}

fn load_testbed(audio: &mut Box<dyn audio::AudioProvider>) -> anyhow::Result<Box<dyn Testbed>> {
	let mut assets = Box::new(assets::Asset {});
	audio.register_wgui_samples(assets.as_mut())?;

	let name = std::env::var("TESTBED").unwrap_or_default();
	Ok(match name.as_str() {
		"dashboard" => Box::new(TestbedDashboard::new()?),
		"" => Box::new(TestbedGeneric::new(assets)?),
		_ => Box::new(TestbedAny::new(assets, &name)?),
	})
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	init_logging();

	let (gfx, event_loop, window) =
		init_window("[-/=]: gui scale, F10: debug draw, F11: print tree")?;
	let inner_size = window.inner_size();
	let mut swapchain_size = [inner_size.width, inner_size.height];
	let mut swapchain = gfx.create_swapchain(swapchain_size)?;
	swapchain_size = swapchain.extent();

	let mut recreate = false;

	let mut scale = window.scale_factor() as f32;

	let mut audio: Box<dyn audio::AudioProvider> = Box::new(audio::RealAudioProvider::new());
	let mut testbed = load_testbed(&mut audio)?;

	let mut mouse = Vec2::ZERO;

	let mut shared_context = renderer_vk::context::SharedContext::new(gfx.clone())?;
	let mut render_context = renderer_vk::context::Context::new(&mut shared_context, scale)?;

	render_context.update_viewport(&mut shared_context, swapchain_size, scale)?;
	log::trace!("new swapchain_size: {swapchain_size:?}");

	let mut debug_draw_enabled = false;

	let mut profiler = profiler::Profiler::new(1000);
	let mut frame_index: u64 = 0;

	let mut timestep = Timestep::new(60.0);

	let mut limiter = RateLimiter::new();

	#[allow(deprecated)]
	event_loop.run(move |event, elwt| {
		elwt.set_control_flow(ControlFlow::Poll);

		match event {
			Event::WindowEvent {
				event: WindowEvent::MouseWheel { delta, .. },
				..
			} => match delta {
				MouseScrollDelta::LineDelta(x, y) => {
					testbed
						.layout()
						.push_event(
							&wgui::event::Event::MouseWheel(MouseWheelEvent {
								delta: Vec2::new(x, y),
								pos: mouse / scale,
								device: wgui::event::DeviceBitmask(0),
							}),
							&mut (),
							&mut (),
						)
						.unwrap();
				}
				MouseScrollDelta::PixelDelta(pos) => {
					testbed
						.layout()
						.push_event(
							&wgui::event::Event::MouseWheel(MouseWheelEvent {
								delta: Vec2::new(pos.x as f32 / 5.0, pos.y as f32 / 5.0),
								pos: mouse / scale,
								device: wgui::event::DeviceBitmask(0),
							}),
							&mut (),
							&mut (),
						)
						.unwrap();
				}
			},
			Event::WindowEvent {
				event: WindowEvent::MouseInput { state, button, .. },
				..
			} => {
				if matches!(button, winit::event::MouseButton::Left) {
					if matches!(state, winit::event::ElementState::Pressed) {
						testbed
							.layout()
							.push_event(
								&wgui::event::Event::MouseDown(MouseButtonEvent {
									pos: mouse / scale,
									index: MouseButtonIndex::Left,
									device: wgui::event::DeviceBitmask(0),
								}),
								&mut (),
								&mut (),
							)
							.unwrap();
					} else {
						testbed
							.layout()
							.push_event(
								&wgui::event::Event::MouseUp(MouseButtonEvent {
									pos: mouse / scale,
									index: MouseButtonIndex::Left,
									device: wgui::event::DeviceBitmask(0),
								}),
								&mut (),
								&mut (),
							)
							.unwrap();
					}
				}
			}
			Event::WindowEvent {
				event: WindowEvent::CursorMoved { position, .. },
				..
			} => {
				mouse = vec2(position.x as _, position.y as _);
				testbed
					.layout()
					.push_event(
						&wgui::event::Event::MouseMotion(MouseMotionEvent {
							pos: mouse / scale,
							device: wgui::event::DeviceBitmask(0),
						}),
						&mut (),
						&mut (),
					)
					.unwrap();
			}
			Event::WindowEvent {
				event: WindowEvent::KeyboardInput { event, .. },
				..
			} => {
				if event.state == ElementState::Pressed {
					if !event.repeat {
						if event.physical_key == PhysicalKey::Code(KeyCode::F10) {
							debug_draw_enabled = !debug_draw_enabled;
							if debug_draw_enabled {
								log::info!(
									"Debug draw enabled\n\tAqua: widget boundary\n\tMagenta: Scissoring (separate render pass)"
								);
							}
							testbed.layout().mark_redraw();
						}

						if event.physical_key == PhysicalKey::Code(KeyCode::F11) {
							testbed.layout().print_tree();
						}

						if event.physical_key == PhysicalKey::Code(KeyCode::Equal) {
							scale *= 1.25;
							render_context
								.update_viewport(&mut shared_context, swapchain_size, scale)
								.unwrap();
						}

						if event.physical_key == PhysicalKey::Code(KeyCode::Minus) {
							scale *= 0.75;
							render_context
								.update_viewport(&mut shared_context, swapchain_size, scale)
								.unwrap();
						}
					}

					testbed
						.layout()
						.push_event(
							&wgui::event::Event::TextInput(wgui::event::TextInputEvent {
								text: event.text.map(|text| Rc::from(text.as_str())),
							}),
							&mut (),
							&mut (),
						)
						.unwrap();
				}
			}
			Event::WindowEvent {
				event: WindowEvent::CloseRequested,
				..
			} => {
				elwt.exit();
			}
			Event::WindowEvent {
				event: WindowEvent::Resized(_),
				..
			} => {
				recreate = true;
			}
			Event::WindowEvent {
				event: WindowEvent::RedrawRequested,
				..
			} => {
				if recreate {
					let inner_size = window.inner_size();
					if inner_size.width == 0 || inner_size.height == 0 {
						return;
					}

					let requested_size = [inner_size.width, inner_size.height];
					match swapchain.recreate(requested_size) {
						Ok(new_swapchain) => {
							swapchain = new_swapchain;
							swapchain_size = swapchain.extent();
						}
						Err(e) => {
							log::error!("failed to recreate swapchain: {e:#}");
							return;
						}
					}

					render_context
						.update_viewport(&mut shared_context, swapchain_size, scale)
						.unwrap();

					log::trace!("new swapchain_size: {swapchain_size:?}");
					recreate = false;
					window.request_redraw();
				}

				while timestep.on_tick() {
					testbed.layout().tick().unwrap();
				}

				testbed
					.update(TestbedUpdateParams {
						width: (swapchain_size[0] as f32 / scale) as _,
						height: (swapchain_size[1] as f32 / scale) as _,
						timestep_alpha: timestep.alpha,
						audio: &mut audio,
					})
					.unwrap();

				if !render_context.dirty && !testbed.layout().check_toggle_needs_redraw() {
					// no need to redraw
					std::thread::sleep(std::time::Duration::from_millis(5)); // dirty fix to prevent cpu burning precious cycles doing a busy loop
					return;
				}

				log::trace!("drawing frame {frame_index}");
				frame_index += 1;

				limiter.start(120); // max 120 fps
				profiler.start();

				{
					let frame = match swapchain.acquire() {
						Ok(frame) => frame,
						Err(SwapchainError::OutOfDate) => {
							recreate = true;
							return;
						}
						Err(e) => {
							log::error!("failed to acquire next image: {e}");
							return;
						}
					};

					let tgt = frame.image_view().clone();

					let mut cmd_buf = gfx
						.create_gfx_command_buffer(CommandBufferUsage::OneTimeSubmit)
						.unwrap();
					cmd_buf
						.begin_rendering(tgt, WGfxClearMode::Clear([0.0, 0.0, 0.0, 0.1]))
						.unwrap();

					let layout = testbed.layout();
					let globals = layout.state.globals.clone();
					let mut globals = globals.get();

					let mut draw_params = wgui::drawing::DrawParams {
						globals: &mut globals,
						layout,
						debug_draw: debug_draw_enabled,
						timestep_alpha: timestep.alpha,
					};

					let primitives = wgui::drawing::draw(&mut draw_params).unwrap();

					let draw_result = render_context
						.draw(
							&globals.font_system,
							&mut shared_context,
							&mut cmd_buf,
							&primitives,
						)
						.unwrap();

					if debug_draw_enabled {
						log::debug!(
							"pass count: {}, primitive commands count: {}",
							draw_result.pass_count,
							draw_result.primitive_commands_count
						);
					}

					cmd_buf.end_rendering().unwrap();

					let cmd_buf = cmd_buf.build().unwrap();

					match frame.present(&cmd_buf) {
						Ok(PresentStatus::Optimal) => {}
						Ok(PresentStatus::Suboptimal) | Err(SwapchainError::OutOfDate) => {
							recreate = true;
						}
						Err(e) => {
							log::error!("failed to present swapchain image: {e}");
						}
					}
				}

				profiler.end();
				limiter.end();
			}
			Event::AboutToWait => {
				// should be limited to vsync
				window.request_redraw();
			}
			_ => (),
		}
	})?;

	Ok(())
}
