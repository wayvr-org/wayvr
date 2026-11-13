use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct VolumeChannel {
	pub value: u32,            // 48231
	pub value_percent: String, // "80%"
	pub db: String,            // "-5.81 dB"
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Volume {
	// WiVRn and other devices
	pub aux0: Option<VolumeChannel>,
	pub aux1: Option<VolumeChannel>,

	// Analog and HDMI devices
	#[serde(rename = "front-left")]
	pub front_left: Option<VolumeChannel>,

	#[serde(rename = "front-right")]
	pub front_right: Option<VolumeChannel>,

	#[serde(rename = "mono")]
	pub mono: Option<VolumeChannel>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Sink {
	pub index: u32,          // 123
	pub state: String,       // "RUNNING" / "SUSPENDED"
	pub name: String,        // alsa_output.pci-0000_0c_00.4.analog-stereo
	pub description: String, // Starship/Matisse HD Audio Controller Analog Stereo
	pub mute: bool,          // false
	pub volume: Volume,
	pub properties: HashMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SourceProperties {
	#[serde(rename = "device.name")]
	pub device_name: Option<String>, // "alsa_card.pci-0000_0b_00.1"
	#[serde(rename = "device.class")]
	pub device_class: Option<String>, // "monitor", "sound"
	#[serde(rename = "alsa.card_name")]
	pub card_name: Option<String>, // "Valve VR Radio & HMD Mic"
	#[serde(rename = "alsa.components")]
	pub components: Option<String>, // USB28de:2102
	#[serde(rename = "device.vendor_name")]
	pub vendor_name: Option<String>, // Valve Software
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Source {
	pub index: u32,          // 123
	pub state: String,       // "RUNNING" / "SUSPENDED"
	pub name: String,        // alsa_input.pci-0000_0c_00.4.analog-stereo
	pub description: String, // Valve VR Radio & HMD Mic Mono
	pub mute: bool,          // false
	pub volume: Volume,
	pub properties: SourceProperties,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CardProperties {
	#[serde(rename = "device.description")]
	pub device_description: String, // Starship/Matisse HD Audio Controller

	#[serde(rename = "device.name")]
	pub device_name: String, // alsa_card.pci-0000_0c_00.4

	#[serde(rename = "device.nick")]
	pub device_nick: Option<String>, // HD-Audio Generic
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CardProfile {
	pub description: String, // "Digital Stereo (HDMI 2) Output", "Analog Stereo Output",
	pub sinks: u32,          // 1
	pub sources: u32,        // 0
	pub priority: u32,       // 6500
	pub available: bool,     // true
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CardPort {
	pub description: String,   // "HDMI / DisplayPort 2"
	pub r#type: String,        // "HDMI"
	pub profiles: Vec<String>, // "output:hdmi-stereo-extra1", "output:hdmi-surround-extra1", "output:analog-stereo", "output:analog-stereo+input:analog-stereo"

	// example:
	// "port.type": "hdmi"
	// "device.product_name": "Index HMD"
	pub properties: HashMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Card {
	pub index: u32,             // 57
	pub name: String,           // alsa_card.pci-0000_0c_00.4
	pub active_profile: String, // output:analog-stereo
	pub properties: CardProperties,
	pub profiles: BTreeMap<String, CardProfile>, // key: "output:analog-stereo"
	pub ports: BTreeMap<String, CardPort>,       // key: "analog-output-lineout"
}

// ########################################
//                ~ sinks ~
// ########################################

pub fn list_sinks() -> anyhow::Result<Vec<Sink>> {
	let output = std::process::Command::new("pactl")
		.arg("--format=json")
		.arg("list")
		.arg("sinks")
		.output()?;

	if !output.status.success() {
		anyhow::bail!("pactl exit status {}", output.status);
	}

	let json_str = std::str::from_utf8(&output.stdout)?;
	let sinks: Vec<Sink> = serde_json::from_str(json_str)?;
	Ok(sinks)
}

pub fn get_default_sink(sinks: &[Sink]) -> anyhow::Result<Option<Sink>> {
	let output = std::process::Command::new("pactl").arg("get-default-sink").output()?;

	let utf8_name = std::str::from_utf8(&output.stdout)?.trim();

	for sink in sinks {
		if sink.name == utf8_name {
			return Ok(Some(sink.clone()));
		}
	}

	Ok(None)
}

pub fn set_default_sink(sink_index: u32) -> anyhow::Result<()> {
	std::process::Command::new("pactl")
		.arg("set-default-sink")
		.arg(format!("{}", sink_index))
		.output()?;

	Ok(())
}

pub fn get_sink_volume(sink: &Sink) -> anyhow::Result<f32> {
	let volume_channel = {
		if let Some(front_left) = &sink.volume.front_left {
			front_left
		} else if let Some(aux0) = &sink.volume.aux0 {
			aux0
		} else {
			return Ok(0.0); // fail silently
		}
	};

	let Some(pair) = volume_channel.value_percent.split_once("%") else {
		anyhow::bail!("volume percentage invalid"); // shouldn't happen
	};

	let percent_num: f32 = pair.0.parse().unwrap_or(0.0);
	Ok(percent_num / 100.0)
}

pub fn set_sink_volume(sink_index: u32, volume: f32) -> anyhow::Result<()> {
	let target_vol = (volume * 100.0).clamp(0.0, 150.0); // limit to 150%

	std::process::Command::new("pactl")
		.arg("set-sink-volume")
		.arg(format!("{}", sink_index))
		.arg(format!("{}%", target_vol))
		.output()?;

	Ok(())
}

pub fn set_sink_mute(sink_index: u32, mute: bool) -> anyhow::Result<()> {
	std::process::Command::new("pactl")
		.arg("set-sink-mute")
		.arg(format!("{}", sink_index))
		.arg(format!("{}", mute as i32))
		.output()?;

	Ok(())
}

// ########################################
//               ~ sources ~
// ########################################

pub fn list_sources() -> anyhow::Result<Vec<Source>> {
	let output = std::process::Command::new("pactl")
		.arg("--format=json")
		.arg("list")
		.arg("sources")
		.output()?;

	if !output.status.success() {
		anyhow::bail!("pactl exit status {}", output.status);
	}

	let json_str = std::str::from_utf8(&output.stdout)?;
	let mut sources: Vec<Source> = serde_json::from_str(json_str)?;

	// exclude all monitor sources
	sources.retain(|source| match &source.properties.device_class {
		Some(c) => c != "monitor",
		None => true, /* unknown, pass it */
	});

	Ok(sources)
}

pub fn get_default_source(sources: &[Source]) -> anyhow::Result<Option<Source>> {
	let output = std::process::Command::new("pactl").arg("get-default-source").output()?;

	let utf8_name = std::str::from_utf8(&output.stdout)?.trim();

	for source in sources {
		if source.name == utf8_name {
			return Ok(Some(source.clone()));
		}
	}

	Ok(None)
}

pub fn set_default_source(source_index: u32) -> anyhow::Result<()> {
	std::process::Command::new("pactl")
		.arg("set-default-source")
		.arg(format!("{}", source_index))
		.output()?;

	Ok(())
}

pub fn get_source_volume(source: &Source) -> anyhow::Result<f32> {
	let volume_channel = {
		if let Some(front_left) = &source.volume.front_left {
			front_left
		} else if let Some(aux0) = &source.volume.aux0 {
			aux0
		} else if let Some(mono) = &source.volume.mono {
			mono
		} else {
			return Ok(0.0); // fail silently
		}
	};

	let Some(pair) = volume_channel.value_percent.split_once("%") else {
		anyhow::bail!("volume percentage invalid"); // shouldn't happen
	};

	let percent_num: f32 = pair.0.parse().unwrap_or(0.0);
	Ok(percent_num / 100.0)
}

pub fn set_source_volume(source_index: u32, volume: f32) -> anyhow::Result<()> {
	let target_vol = (volume * 100.0).clamp(0.0, 150.0); // limit to 150%

	std::process::Command::new("pactl")
		.arg("set-source-volume")
		.arg(format!("{}", source_index))
		.arg(format!("{}%", target_vol))
		.output()?;

	Ok(())
}

pub fn set_source_mute(source_index: u32, mute: bool) -> anyhow::Result<()> {
	std::process::Command::new("pactl")
		.arg("set-source-mute")
		.arg(format!("{}", source_index))
		.arg(format!("{}", mute as i32))
		.output()?;

	Ok(())
}

// ########################################
//                ~ cards ~
// ########################################

pub fn list_cards() -> anyhow::Result<Vec<Card>> {
	let output = std::process::Command::new("pactl")
		.arg("--format=json")
		.arg("list")
		.arg("cards")
		.output()?;

	if !output.status.success() {
		anyhow::bail!("pactl exit status {}", output.status);
	}

	let json_str = std::str::from_utf8(&output.stdout)?;
	let mut cards: Vec<Card> = serde_json::from_str(json_str)?;

	// exclude card which has "Loopback" in name
	cards.retain(|card| {
		if let Some(nick) = &card.properties.device_nick {
			return nick != "Loopback";
		};
		true
	});

	Ok(cards)
}

pub fn set_card_profile(card_index: u32, profile: &str) -> anyhow::Result<()> {
	std::process::Command::new("pactl")
		.arg("set-card-profile")
		.arg(format!("{}", card_index))
		.arg(profile)
		.output()?;

	Ok(())
}
