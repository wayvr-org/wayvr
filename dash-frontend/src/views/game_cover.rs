use std::rc::Rc;

use wgui::{
	assets::AssetPath,
	components::{
		self,
		button::ComponentButton,
		tooltip::{TooltipInfo, TooltipSide},
	},
	drawing::{self, GradientMode},
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	renderer_vk::text::{FontWeight, HorizontalAlign, TextShadow, TextStyle, custom_glyph::CustomGlyphData},
	taffy::{
		self, AlignItems, AlignSelf, JustifyContent, JustifySelf,
		prelude::{auto, length, percent},
	},
	widget::{
		ConstructEssentials,
		div::WidgetDiv,
		image::{WidgetImage, WidgetImageParams},
		label::{WidgetLabel, WidgetLabelParams},
		rectangle,
		util::WLength,
	},
};

use crate::util::{
	cached_fetcher::{self, CoverArt},
	steam_utils::{self, AppID},
	various::AsyncExecutor,
};

pub struct ViewCommon {
	img_placeholder: Option<CustomGlyphData>,
	globals: WguiGlobals,
}

pub struct Params<'a, 'b> {
	pub ess: &'a mut ConstructEssentials<'b>,
	pub executor: &'a AsyncExecutor,
	pub manifest: &'a steam_utils::AppManifest,
	pub scale: f32,
	pub on_loaded: Box<dyn FnOnce(CoverArt)>,
}

pub struct View {
	pub button: Rc<ComponentButton>,
	id_image_parent: WidgetID,
	app_name: String,
	app_id: AppID,
}

const BORDER_COLOR_DEFAULT: drawing::Color = drawing::Color::new(0.0, 0.0, 0.0, 0.35);
const BORDER_COLOR_HOVERED: drawing::Color = drawing::Color::new(1.0, 1.0, 1.0, 1.0);

const GAME_COVER_SIZE_X: f32 = 140.0;
const GAME_COVER_SIZE_Y: f32 = 210.0;

impl View {
	async fn request_cover_image(
		executor: AsyncExecutor,
		manifest: steam_utils::AppManifest,
		on_loaded: Box<dyn FnOnce(CoverArt)>,
	) {
		let cover_art = match cached_fetcher::request_image(executor, manifest.app_id.clone()).await {
			Ok(cover_art) => cover_art,
			Err(e) => {
				log::error!("request_cover_image failed: {:?}", e);
				return;
			}
		};

		on_loaded(cover_art)
	}

	fn mount_image(&self, layout: &mut Layout, glyph: &CustomGlyphData) -> anyhow::Result<()> {
		let image = WidgetImage::create(WidgetImageParams {
			round: WLength::Units(10.0),
			glyph_data: Some(glyph.clone()),
			..Default::default()
		});

		let (a, _) = layout.add_child(
			self.id_image_parent,
			image,
			taffy::Style {
				size: taffy::Size {
					width: percent(1.0),
					height: percent(1.0),
				},
				..Default::default()
			},
		)?;
		a.widget.state().flags.new_pass = true;

		Ok(())
	}

	fn mount_placeholder_text(&self, layout: &mut Layout, parent: WidgetID, text: &str) -> anyhow::Result<()> {
		let label = WidgetLabel::create(
			&mut layout.state,
			WidgetLabelParams {
				content: Translation::from_raw_text(text),
				style: TextStyle {
					weight: Some(FontWeight::Bold),
					wrap: true,
					size: Some(16.0),
					align: Some(HorizontalAlign::Center),
					shadow: Some(TextShadow {
						color: drawing::Color::new(0.0, 0.0, 0.0, 1.0),
						x: 2.0,
						y: 2.0,
					}),
					..Default::default()
				},
			},
		);

		layout.add_child(
			parent,
			label,
			taffy::Style {
				position: taffy::Position::Absolute,
				align_self: Some(AlignSelf::Baseline),
				justify_self: Some(JustifySelf::Center),
				margin: taffy::Rect {
					top: length(32.0),
					bottom: auto(),
					left: auto(),
					right: auto(),
				},
				..Default::default()
			},
		)?;
		Ok(())
	}

	pub fn set_cover_art(
		&mut self,
		view_common: &mut ViewCommon,
		layout: &mut Layout,
		cover_art: &CoverArt,
	) -> anyhow::Result<()> {
		if cover_art.compressed_image_data.is_empty() {
			// mount placeholder
			let img = view_common.get_placeholder_image()?.clone();
			self.mount_image(layout, &img)?;
			self.mount_placeholder_text(layout, self.id_image_parent, &self.app_name)?;
		} else {
			// mount image
			let path = format!("app:{:?}", self.app_id);
			let glyph =
				match CustomGlyphData::from_bytes_raster(&view_common.globals, &path, &cover_art.compressed_image_data) {
					Ok(c) => c,
					Err(e) => {
						log::warn!("failed to decode cover art image: {:?}", e);
						return Ok(());
					}
				};
			self.mount_image(layout, &glyph)?;
		}

		Ok(())
	}

	pub fn new(params: Params) -> anyhow::Result<Self> {
		let (widget_button, button) = components::button::construct(
			params.ess,
			components::button::Params {
				color: Some(drawing::Color::new(1.0, 1.0, 1.0, 0.0)),
				border_color: Some(BORDER_COLOR_DEFAULT),
				hover_border_color: Some(BORDER_COLOR_HOVERED),
				round: WLength::Units(12.0),
				border: 2.0,
				tooltip: Some(TooltipInfo {
					side: TooltipSide::Bottom,
					text: Translation::from_raw_text(&params.manifest.name),
				}),
				style: taffy::Style {
					position: taffy::Position::Relative,
					align_items: Some(taffy::AlignItems::Center),
					justify_content: Some(taffy::JustifyContent::Center),
					size: taffy::Size {
						width: length(GAME_COVER_SIZE_X * params.scale),
						height: length(GAME_COVER_SIZE_Y * params.scale),
					},
					..Default::default()
				},
				..Default::default()
			},
		)?;

		let (image_parent, _) = params.ess.layout.add_child(
			widget_button.id,
			WidgetDiv::create(),
			taffy::Style {
				position: taffy::Position::Absolute,
				size: taffy::Size {
					width: percent(1.0),
					height: percent(1.0),
				},
				padding: taffy::Rect::length(2.0),
				align_items: Some(AlignItems::Center),
				justify_content: Some(JustifyContent::Center),
				..Default::default()
			},
		)?;

		let rect_gradient = |color: drawing::Color, color2: drawing::Color| {
			rectangle::WidgetRectangle::create(rectangle::WidgetRectangleParams {
				color,
				color2,
				round: WLength::Units(12.0),
				gradient: GradientMode::Vertical,
				..Default::default()
			})
		};

		let rect_gradient_style = |align_self: taffy::AlignSelf, height: f32| taffy::Style {
			position: taffy::Position::Absolute,
			align_self: Some(align_self),
			size: taffy::Size {
				width: percent(1.0),
				height: percent(height),
			},
			..Default::default()
		};

		// top shine
		let (top_shine, _) = params.ess.layout.add_child(
			widget_button.id,
			rect_gradient(
				drawing::Color::new(1.0, 1.0, 1.0, 0.2),
				drawing::Color::new(1.0, 1.0, 1.0, 0.02),
			),
			rect_gradient_style(taffy::AlignSelf::Baseline, 0.05),
		)?;

		// not optimal, this forces us to create a new pass for every created cover art just to overlay various rectangles at the top of the image cover art
		top_shine.widget.state().flags.new_pass = true;

		// top white gradient
		params.ess.layout.add_child(
			widget_button.id,
			rect_gradient(
				drawing::Color::new(1.0, 1.0, 1.0, 0.15),
				drawing::Color::new(1.0, 1.0, 1.0, 0.0),
			),
			rect_gradient_style(taffy::AlignSelf::Baseline, 0.5),
		)?;

		// bottom black gradient
		params.ess.layout.add_child(
			widget_button.id,
			rect_gradient(
				drawing::Color::new(0.0, 0.0, 0.0, 0.0),
				drawing::Color::new(0.0, 0.0, 0.0, 0.25),
			),
			rect_gradient_style(taffy::AlignSelf::End, 0.5),
		)?;

		// bottom shadow
		params.ess.layout.add_child(
			widget_button.id,
			rect_gradient(
				drawing::Color::new(0.0, 0.0, 0.0, 0.1),
				drawing::Color::new(0.0, 0.0, 0.0, 0.9),
			),
			rect_gradient_style(taffy::AlignSelf::End, 0.05),
		)?;

		// request cover image data from the internet or disk cache
		params
			.executor
			.spawn(View::request_cover_image(
				params.executor.clone(),
				params.manifest.clone(),
				Box::new(params.on_loaded),
			))
			.detach();

		Ok(View {
			button,
			id_image_parent: image_parent.id,
			app_name: params.manifest.name.clone(),
			app_id: params.manifest.app_id.clone(),
		})
	}
}

impl ViewCommon {
	pub fn new(globals: WguiGlobals) -> Self {
		Self {
			globals,
			img_placeholder: None,
		}
	}

	fn get_placeholder_image(&mut self) -> anyhow::Result<&CustomGlyphData> {
		if self.img_placeholder.is_none() {
			let c = CustomGlyphData::from_assets(&self.globals, AssetPath::BuiltIn("dashboard/placeholder_cover.png"))?;
			self.img_placeholder = Some(c);
		}

		Ok(self.img_placeholder.as_ref().unwrap()) // safe
	}
}
