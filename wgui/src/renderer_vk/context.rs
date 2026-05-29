use std::{cell::RefCell, rc::Rc, sync::Arc};

use cosmic_text::Buffer;
use glam::{Mat4, Vec2, Vec3};
use slotmap::{SlotMap, new_key_type};
use vulkano::pipeline::graphics::viewport;

use crate::{
	drawing::{self},
	font_config,
	gfx::{WGfx, cmd::GfxCommandBuffer},
	renderer_vk::image::{ImagePipeline, ImageRenderer, ImageViewCache},
};

use super::{
	rect::{RectPipeline, RectRenderer},
	text::{
		DEFAULT_METRICS, SWASH_CACHE, TextArea, TextBounds,
		text_atlas::{TextAtlas, TextPipeline},
		text_renderer::TextRenderer,
	},
	viewport::Viewport,
};

struct RendererPass<'a> {
	submitted: bool,
	text_areas: Vec<TextArea<'a>>,
	text_renderer: TextRenderer,
	rect_renderer: RectRenderer,
	image_renderer: ImageRenderer,
	scissor: Option<drawing::Boundary>,
	pixel_scale: f32,
}

impl RendererPass<'_> {
	fn new(
		text_atlas: &mut TextAtlas,
		rect_pipeline: RectPipeline,
		image_pipeline: ImagePipeline,
		scissor: Option<drawing::Boundary>,
		pixel_scale: f32,
	) -> anyhow::Result<Self> {
		let text_renderer = TextRenderer::new(text_atlas)?;
		let rect_renderer = RectRenderer::new(rect_pipeline)?;
		let image_renderer = ImageRenderer::new(image_pipeline)?;

		Ok(Self {
			submitted: false,
			text_renderer,
			rect_renderer,
			image_renderer,
			text_areas: Vec::new(),
			scissor,
			pixel_scale,
		})
	}

	fn submit(
		&mut self,
		font_system: &font_config::WguiFontSystem,
		gfx: &Arc<WGfx>,
		viewport: &mut Viewport,
		cmd_buf: &mut GfxCommandBuffer,
		text_atlas: &mut TextAtlas,
		image_view_cache: &mut ImageViewCache,
	) -> anyhow::Result<()> {
		if self.submitted {
			return Ok(());
		}

		let vk_scissor = match self.scissor {
			Some(scissor) => {
				let mut x = scissor.pos.x;
				let mut y = scissor.pos.y;
				let mut w = scissor.size.x;
				let mut h = scissor.size.y;

				// handle out-of-bounds scissors (x/y < 0)
				if x < 0.0 {
					w += x;
					x = 0.0;
				}

				if y < 0.0 {
					h += y;
					y = 0.0;
				}

				viewport::Scissor {
					offset: [(x * self.pixel_scale) as u32, (y * self.pixel_scale) as u32],
					extent: [(w * self.pixel_scale) as u32, (h * self.pixel_scale) as u32],
				}
			}
			None => viewport::Scissor::default(),
		};

		self.submitted = true;
		self.rect_renderer.render(gfx, viewport, &vk_scissor, cmd_buf)?;
		self
			.image_renderer
			.render(gfx, viewport, &vk_scissor, cmd_buf, image_view_cache)?;

		{
			let mut font_system = font_system.system.lock();
			let mut swash_cache = SWASH_CACHE.lock();

			self.text_renderer.prepare(
				&mut font_system,
				text_atlas,
				viewport,
				std::mem::take(&mut self.text_areas),
				&mut swash_cache,
			)?;
		}

		self.text_renderer.render(text_atlas, viewport, &vk_scissor, cmd_buf)?;

		Ok(())
	}
}

new_key_type! {
	struct SharedContextKey;
}

pub struct SharedContext {
	gfx: Arc<WGfx>,
	atlas_map: SlotMap<SharedContextKey, SharedAtlas>,
	rect_pipeline: RectPipeline,
	text_pipeline: TextPipeline,
	image_pipeline: ImagePipeline,
}

impl SharedContext {
	pub fn new(gfx: Arc<WGfx>) -> anyhow::Result<Self> {
		let rect_pipeline = RectPipeline::new(gfx.clone(), gfx.surface_format)?;
		let text_pipeline = TextPipeline::new(gfx.clone(), gfx.surface_format)?;
		let image_pipeline = ImagePipeline::new(gfx.clone(), gfx.surface_format)?;

		Ok(Self {
			gfx,
			atlas_map: SlotMap::with_key(),
			rect_pipeline,
			text_pipeline,
			image_pipeline,
		})
	}

	fn atlas_for_pixel_scale(&mut self, pixel_scale: f32) -> anyhow::Result<SharedContextKey> {
		for (key, atlas) in &self.atlas_map {
			if (atlas.pixel_scale - pixel_scale).abs() < f32::EPSILON {
				return Ok(key);
			}
		}
		log::debug!("Initializing SharedAtlas for pixel scale {pixel_scale:.2}");
		let text_atlas = TextAtlas::new(self.text_pipeline.clone())?;
		Ok(self.atlas_map.insert(SharedAtlas {
			text_atlas,
			pixel_scale,
		}))
	}
}

struct SharedAtlas {
	text_atlas: TextAtlas,
	pixel_scale: f32,
}

pub struct Context {
	viewport: Viewport,
	shared_ctx_key: SharedContextKey,
	pub dirty: bool,
	pixel_scale: f32,
	empty_text: Rc<RefCell<Buffer>>,
	image_cache: ImageViewCache,
}

pub struct ContextDrawResult {
	pub pass_count: u32,
	pub primitive_commands_count: u32,
}

impl Context {
	pub fn new(shared: &mut SharedContext, pixel_scale: f32) -> anyhow::Result<Self> {
		let viewport = Viewport::new(&shared.gfx)?;
		let shared_ctx_key = shared.atlas_for_pixel_scale(pixel_scale)?;

		Ok(Self {
			viewport,
			shared_ctx_key,
			pixel_scale,
			dirty: true,
			empty_text: Rc::new(RefCell::new(Buffer::new_empty(DEFAULT_METRICS))),
			image_cache: ImageViewCache::new(),
		})
	}

	pub fn update_viewport(
		&mut self,
		shared: &mut SharedContext,
		resolution: [u32; 2],
		pixel_scale: f32,
	) -> anyhow::Result<()> {
		if (self.pixel_scale - pixel_scale).abs() > f32::EPSILON {
			self.pixel_scale = pixel_scale;
			self.shared_ctx_key = shared.atlas_for_pixel_scale(pixel_scale)?;
		}

		if self.viewport.resolution() != resolution {
			self.dirty = true;
		}

		let size = Vec2::new(resolution[0] as f32 / pixel_scale, resolution[1] as f32 / pixel_scale);

		let fov = 0.4;
		let aspect_ratio = size.x / size.y;
		let projection = Mat4::perspective_rh(fov, aspect_ratio, 1.0, 100_000.0);

		let b = size.y / 2.0;
		let angle_half = fov / 2.0;
		let distance = (std::f32::consts::PI / 2.0 - angle_half).tan() * b;

		let view = Mat4::look_at_rh(
			Vec3::new(size.x / 2.0, size.y / 2.0, distance),
			Vec3::new(size.x / 2.0, size.y / 2.0, 0.0),
			Vec3::new(0.0, 1.0, 0.0),
		);

		let fin = projection * view;

		self.viewport.update(resolution, &fin, pixel_scale)?;
		Ok(())
	}

	pub fn draw(
		&mut self,
		font_system: &font_config::WguiFontSystem,
		shared: &mut SharedContext,
		cmd_buf: &mut GfxCommandBuffer,
		primitives: &[drawing::RenderPrimitive],
	) -> anyhow::Result<ContextDrawResult> {
		self.dirty = false;

		let atlas = shared.atlas_map.get_mut(self.shared_ctx_key).unwrap();

		let mut passes = Vec::<RendererPass>::new();
		let mut needs_new_pass = true;
		let mut cur_scissor: Option<drawing::Boundary> = None;

		// drop unreferenced image views to avoid vram leaks
		self.image_cache.retain(|_, v| v.content.strong_count() > 0);

		for primitive in primitives {
			if needs_new_pass {
				passes.push(RendererPass::new(
					&mut atlas.text_atlas,
					shared.rect_pipeline.clone(),
					shared.image_pipeline.clone(),
					cur_scissor,
					self.pixel_scale,
				)?);
				needs_new_pass = false;
			}

			let pass = passes.last_mut().unwrap(); // always safe

			match &primitive {
				drawing::RenderPrimitive::NewPass => {
					needs_new_pass = true;
				}
				drawing::RenderPrimitive::Rectangle(extent, rectangle) => {
					pass
						.rect_renderer
						.add_rect(extent.boundary, *rectangle, &extent.transform);
				}
				drawing::RenderPrimitive::Text(extent, text, shadow) => {
					if let Some(shadow) = shadow {
						pass.text_areas.push(TextArea {
							buffer: text.clone(),
							left: (extent.boundary.pos.x + shadow.x) * self.pixel_scale,
							top: (extent.boundary.pos.y + shadow.y) * self.pixel_scale,
							bounds: TextBounds::default(), //FIXME: just using boundary coords here doesn't work
							scale: self.pixel_scale,
							default_color: cosmic_text::Color::rgb(0, 0, 0),
							override_color: Some(shadow.color.into()),
							custom_glyphs: &[],
							transform: extent.transform,
						});
					}
					pass.text_areas.push(TextArea {
						buffer: text.clone(),
						left: extent.boundary.pos.x * self.pixel_scale,
						top: extent.boundary.pos.y * self.pixel_scale,
						bounds: TextBounds::default(), //FIXME: just using boundary coords here doesn't work
						scale: self.pixel_scale,
						default_color: cosmic_text::Color::rgb(0, 0, 0),
						override_color: None,
						custom_glyphs: &[],
						transform: extent.transform,
					});
				}
				drawing::RenderPrimitive::Sprite(extent, sprites) => {
					pass.text_areas.push(TextArea {
						buffer: self.empty_text.clone(),
						left: extent.boundary.pos.x * self.pixel_scale,
						top: extent.boundary.pos.y * self.pixel_scale,
						bounds: TextBounds::default(),
						scale: self.pixel_scale,
						custom_glyphs: sprites.as_slice(),
						default_color: cosmic_text::Color::rgb(255, 0, 255),
						override_color: None,
						transform: extent.transform,
					});
				}
				drawing::RenderPrimitive::Image(extent, image) => {
					pass
						.image_renderer
						.add_image(extent.boundary, image.clone(), &extent.transform);
				}
				drawing::RenderPrimitive::ScissorSet(boundary) => {
					let skip = if let Some(cur_scissor) = cur_scissor {
						// do not create a new pass if it's not needed (same scissor values)
						cur_scissor == boundary.0
					} else {
						false
					};

					cur_scissor = Some(boundary.0);
					if skip {
						//log::debug!("same scissor boundary, re-using the same pass");
					} else {
						needs_new_pass = true;
					}
				}
			}
		}

		let res = ContextDrawResult {
			pass_count: passes.len() as u32,
			primitive_commands_count: primitives.len() as u32,
		};

		for mut pass in passes {
			pass.submit(
				font_system,
				&shared.gfx,
				&mut self.viewport,
				cmd_buf,
				&mut atlas.text_atlas,
				&mut self.image_cache,
			)?;
		}

		Ok(res)
	}
}
