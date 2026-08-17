use image::ImageBuffer;
use taffy::prelude::percent;

use crate::{
	animation::Animation,
	assets::AssetPathRef,
	color::WguiColorName,
	components::{Component, ComponentBase, ComponentTrait, RefreshData},
	event::EventAlterables,
	globals::WguiGlobals,
	layout::{Layout, WidgetID, WidgetPair},
	renderer_vk::text::custom_glyph::{CustomGlyphContent, CustomGlyphData},
	time::get_millis,
	video_dec::{self, Av1Decoder, IvfReader},
	widget::{
		ConstructEssentials,
		image::WidgetImage,
		rectangle::{WidgetRectangle, WidgetRectangleParams},
	},
};
use std::{
	cell::RefCell,
	rc::{Rc, Weak},
	sync::Arc,
};

#[derive(Default)]
pub struct Params<'a> {
	pub style: taffy::Style,
	pub src: Option<AssetPathRef<'a>>,
	pub looping: bool,
	pub speed: f32,
}

struct PlayingSource {
	demuxer: IvfReader,
	decoder: video_dec::Av1Decoder,
	cur_frame: u32,
}

struct State {
	source: Option<PlayingSource>,
	self_ref: Weak<ComponentVideo>,
	playing: bool,
	play_requested: bool,
}

struct Data {
	#[allow(dead_code)]
	id_container: WidgetID,
	id_image: WidgetID,

	looping: bool,
	speed: f32,
}

#[allow(dead_code)]
pub struct ComponentVideo {
	base: ComponentBase,
	data: Rc<Data>,
	state: Rc<RefCell<State>>,
}

impl ComponentTrait for ComponentVideo {
	fn base(&self) -> &ComponentBase {
		&self.base
	}

	fn base_mut(&mut self) -> &mut ComponentBase {
		&mut self.base
	}

	fn refresh(&self, data: &mut RefreshData) {
		let mut state = self.state.borrow_mut();

		if state.play_requested {
			state.play_requested = false;
			if let Err(e) = self.play(&mut state, data.layout) {
				log::error!("play failed: {e:?}");
			}
		}
	}
}

const PLAYBACK_ANIMATION_ID: u32 = 1000;

impl ComponentVideo {
	fn play(&self, state: &mut State, layout: &mut Layout) -> anyhow::Result<()> {
		let Some(source) = &mut state.source else {
			// no source available, do nothing
			return Ok(());
		};

		state.playing = false;
		source.decoder = Av1Decoder::new()?;
		source.demuxer.rewind();
		source.cur_frame = 0;

		if let Some(component) = state.self_ref.upgrade() {
			layout.defer_component_refresh(Component(component));
		}

		layout
			.animations
			.stop_by_widget(self.data.id_image, Some(PLAYBACK_ANIMATION_ID));

		state.playing = true;

		let framerate = source.demuxer.framerate * self.data.speed;
		let looping = self.data.looping;
		let id_image = self.data.id_image;
		let start_time = get_millis();
		// log::info!("num frames: {}", source.demuxer.num_frames);

		layout.animations.add(Animation::new_ex(
			self.data.id_image,
			PLAYBACK_ANIMATION_ID,
			u32::MAX, // infinity
			crate::animation::AnimationEasing::Linear,
			Box::new({
				let state_ref = self.state.clone();

				move |common, data| {
					let mut state = state_ref.borrow_mut();
					if !state.playing {
						*data.stop_me = true;
						return;
					}

					loop {
						let Some(source) = &mut state.source else {
							return;
						};

						let cur_time = get_millis() - start_time;
						let target_frame = ((cur_time as f32) / 1000.0 * framerate) as u32;

						let cur_frame = source.cur_frame;
						if cur_frame >= target_frame {
							break;
						}

						let image = data.obj.cast_mut::<WidgetImage>().unwrap();

						match ComponentVideo::read_next_frame(image, &mut state, common.alterables) {
							Ok(data_available) => {
								if !data_available {
									if looping {
										state.play_requested = true;
										common.alterables.refresh_component_once(&state.self_ref);
										common.mark_widget_dirty(id_image);
									} else {
										state.playing = false;
									}
									break;
								}
							}
							Err(e) => {
								state.playing = false;
								log::error!("read_next_frame failed: {e:?}");
							}
						}
					}
				}
			}),
		));
		Ok(())
	}

	fn read_next_frame(
		image: &mut WidgetImage,
		state: &mut State,
		alterables: &mut EventAlterables,
	) -> anyhow::Result<bool> {
		let Some(source) = &mut state.source else {
			return Ok(false);
		};

		let rgbx_frame = match source.decoder.read_frame(&mut source.demuxer)? {
			video_dec::ReadFrameResult::Ok(rgbx_frame) => rgbx_frame,
			video_dec::ReadFrameResult::EndOfFile => {
				// log::info!("got EOF");
				return Ok(false);
			}
		};

		let Some(buffer) = ImageBuffer::from_raw(rgbx_frame.width.into(), rgbx_frame.height.into(), rgbx_frame.data) else {
			anyhow::bail!("ImageBuffer failed");
		};

		let glyph_content = CustomGlyphContent::Image(buffer);

		image.set_content(
			alterables,
			Some(CustomGlyphData {
				// force-update image content (FIXME: stream image data directly instead)
				id: source.cur_frame as usize,
				content: Arc::new(glyph_content),
			}),
		);

		source.cur_frame += 1;

		Ok(true)
	}
}

impl State {
	fn set_source(&mut self, globals: &WguiGlobals, src: AssetPathRef) -> anyhow::Result<()> {
		let video_data = globals.get_asset(src)?.0;
		let demuxer = IvfReader::new(video_data)?;
		let decoder = Av1Decoder::new()?;

		self.source = Some(PlayingSource {
			demuxer,
			decoder,
			cur_frame: 0,
		});

		Ok(())
	}
}

pub fn construct(ess: &mut ConstructEssentials, params: Params) -> anyhow::Result<(WidgetPair, Rc<ComponentVideo>)> {
	let style = params.style;

	let (root, _) = ess.layout.add_child(
		ess.parent,
		WidgetRectangle::create(WidgetRectangleParams {
			color: WguiColorName::Background.into(),
			..Default::default()
		}),
		style,
	)?;

	let (image, _) = ess.layout.add_child(
		root.id,
		WidgetImage::create(Default::default()),
		taffy::Style {
			size: taffy::Size {
				width: percent(1.0_f32),
				height: percent(1.0_f32),
			},
			..Default::default()
		},
	)?;

	let id_container = root.id;
	let data = Rc::new(Data {
		id_container,
		id_image: image.id,
		looping: params.looping,
		speed: params.speed,
	});

	let state = Rc::new(RefCell::new(State {
		source: None,
		self_ref: Default::default(),
		playing: false,
		play_requested: false,
	}));

	let base = ComponentBase {
		id: root.id,
		lhandles: Default::default(),
	};

	let video = Rc::new(ComponentVideo { base, data, state });

	// configure state and set video source
	{
		let mut state = video.state.borrow_mut();
		state.self_ref = Rc::downgrade(&video);
		if let Some(src) = params.src
			&& let Err(e) = state.set_source(&ess.layout.state.globals, src)
		{
			log::error!("set_source failed: {e:?}");
		}

		state.play_requested = true;
	}

	ess.layout.defer_component_refresh(Component(video.clone()));
	Ok((root, video))
}
