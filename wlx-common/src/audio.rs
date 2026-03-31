use std::{collections::HashMap, io::Cursor, rc::Rc};

use rodio::Source;
use wgui::{assets::AssetProvider, sound::WguiSoundType};

use std::io::Read;

pub struct AudioSystem {
	audio_stream: Option<rodio::OutputStream>,
	first_try: bool,
}

pub struct AudioSample {
	buffer: rodio::buffer::SamplesBuffer,
}

pub struct SamplePlayer {
	samples: HashMap<String, AudioSample>,
}

fn get_sample_name_from_wgui_sound_type(sound: WguiSoundType) -> &'static str {
	match sound {
		WguiSoundType::ButtonMouseEnter => "wgui_mouse_enter",
		WguiSoundType::ButtonPress => "wgui_button_press",
		WguiSoundType::ButtonRelease => "wgui_button_release",
		WguiSoundType::CheckboxCheck => "wgui_checkbox_check",
		WguiSoundType::CheckboxUncheck => "wgui_checkbox_uncheck",
	}
}

impl SamplePlayer {
	pub fn new() -> Self {
		Self {
			samples: HashMap::new(),
		}
	}

	pub fn register_sample(&mut self, sample_name: &str, sample: AudioSample) -> anyhow::Result<()> {
		// load only once
		if self.samples.contains_key(sample_name) {
			log::debug!("Audio sample '{sample_name}' already exists.");
			return Ok(());
		}

		log::debug!("Registering audio sample '{sample_name}'");
		self.samples.insert(String::from(sample_name), sample);
		Ok(())
	}

	pub fn register_wgui_samples(&mut self, assets: &mut dyn AssetProvider) -> anyhow::Result<()> {
		let mut load = |sound: WguiSoundType| -> anyhow::Result<()> {
			let sample_name = get_sample_name_from_wgui_sound_type(sound);
			let path = &format!("sound/{}.mp3", sample_name);

			// try loading a custom sound; if one doesn't exist (or it failed to load), use the built-in asset
			let sound_bytes = match AudioSample::try_bytes_from_config(path) {
				Ok(bytes) => bytes,
				Err(_) => assets.load_from_path(path)?.into(),
			};

			self.register_sample(sample_name, AudioSample::from_mp3(&sound_bytes)?)?;
			Ok(())
		};

		load(WguiSoundType::ButtonPress)?;
		load(WguiSoundType::ButtonRelease)?;
		load(WguiSoundType::ButtonMouseEnter)?;
		load(WguiSoundType::CheckboxCheck)?;
		load(WguiSoundType::CheckboxUncheck)?;
		Ok(())
	}

	pub fn play_sample(&mut self, system: &mut AudioSystem, sample_name: &str) {
		let Some(sample) = self.samples.get(sample_name) else {
			log::error!("Failed to play sample by name '{}'", sample_name);
			return;
		};

		system.play_sample(sample);
	}

	pub fn play_wgui_samples(&mut self, system: &mut AudioSystem, samples: Vec<WguiSoundType>) {
		for sample in samples {
			self.play_sample(system, get_sample_name_from_wgui_sound_type(sample));
		}
	}
}

impl Default for SamplePlayer {
	fn default() -> Self {
		Self::new()
	}
}

impl AudioSystem {
	pub const fn new() -> Self {
		Self {
			audio_stream: None,
			first_try: true,
		}
	}

	fn get_handle(&mut self) -> Option<&rodio::OutputStream> {
		if self.audio_stream.is_none() && self.first_try {
			self.first_try = false;
			if let Ok(stream) = rodio::OutputStreamBuilder::open_default_stream() {
				self.audio_stream = Some(stream);
			} else {
				log::error!("Failed to open audio stream. Audio will not work.");
				return None;
			}
		}
		self.audio_stream.as_ref()
	}

	pub fn play_sample(&mut self, sample: &AudioSample) -> Option<()> {
		let handle = self.get_handle()?;
		handle.mixer().add(sample.buffer.clone());
		Some(())
	}
}

impl Default for AudioSystem {
	fn default() -> Self {
		Self::new()
	}
}

impl AudioSample {
	pub fn from_mp3(encoded_bin: &[u8]) -> anyhow::Result<Self> {
		// SAFETY: this is safe
		// rodio requires us to provide 'static data to decode it
		// we are casting &T into &'static T just to prevent unnecessary memory copy into Vec<u8>.
		// `encoded_bin` data will be always valid, because we are dropping `decoder` in this scope afterwards.
		// Compliant and slower version would be: Cursor::new(encoded_bin.to_vec())
		let cursor = unsafe { Cursor::new(std::mem::transmute::<&[u8], &'static [u8]>(encoded_bin)) };

		let decoder = rodio::Decoder::new_mp3(cursor)?;
		Ok(Self {
			buffer: rodio::buffer::SamplesBuffer::new(
				decoder.channels(),
				decoder.sample_rate(),
				decoder.collect::<Vec<rodio::Sample>>(),
			),
		})
	}

	pub fn try_bytes_from_config(path: &str) -> anyhow::Result<Rc<[u8]>> {
		let real_path = crate::config_io::get_config_root().join(path);

		let mut file = std::fs::File::open(&real_path)
			.inspect_err(|e| log::debug!("Could not open file '{}': {e:?}", real_path.display()))?;
		let mut file_buffer = vec![];
		file
			.read_to_end(&mut file_buffer)
			.inspect_err(|e| log::debug!("Could not read file '{}': {e:?}", real_path.display()))?;
		Ok(file_buffer.into())
	}

	pub fn bytes_from_config_or_default(path: &str, default: &'static [u8]) -> Rc<[u8]> {
		match AudioSample::try_bytes_from_config(path) {
			Ok(value) => value,
			Err(_) => {
				log::trace!("File '{}' not found, using default.", path);
				default.into()
			}
		}
	}
}
