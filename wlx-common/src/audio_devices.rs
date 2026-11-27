#[derive(Debug, PartialEq, Clone, Copy)]
pub enum AudioNodeType {
	Source,
	Sink,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioNode {
	pub index: u32,
	pub node_type: AudioNodeType,
	pub name: String,
	pub nick: Option<String>,
	pub description: String,
	pub mute: bool,
	pub volume: f32,
	pub device_id: Option<u32>,
}

pub type Source = AudioNode;

#[derive(Clone, Debug, PartialEq)]
pub struct AudioDeviceProfile {
	pub index: i32,
	pub name: String,
	pub description: String,
	pub priority: i32,
	pub source_count: usize,
	pub sink_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioDeviceRoute {
	pub index: i32,
	pub name: String,
	pub description: String,
	pub device_ids: Vec<i32>,
	pub profile_ids: Vec<i32>,
	pub product_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioDevice {
	pub index: u32,
	pub description: String,
	pub name: String,
	pub nick: Option<String>,
	pub profiles: Vec<AudioDeviceProfile>,
	pub active_profile_index: Option<i32>,
	pub routes: Vec<AudioDeviceRoute>,
}
impl AudioDevice {
	pub fn profile_by_index(&self, index: i32) -> Option<&AudioDeviceProfile> {
		self.profiles.iter().find(|p| p.index == index)
	}
	pub fn profile_by_name(&self, name: &str) -> Option<&AudioDeviceProfile> {
		self.profiles.iter().find(|p| p.name == name)
	}
	pub fn active_profile(&self) -> Option<&AudioDeviceProfile> {
		self
			.active_profile_index
			.and_then(|index| self.profiles.iter().find(|p| p.index == index))
	}
}

#[derive(Clone, Debug, Default)]
pub struct AudioSnapshot {
	pub nodes: Vec<AudioNode>,
	pub devices: Vec<AudioDevice>,
	pub default_sink: Option<String>,   // node name
	pub default_source: Option<String>, // node name

	// bumped on every change except volume levels
	pub layout_revision: u64,
}

impl AudioSnapshot {
	pub fn nodes(&self, node_type: AudioNodeType) -> impl Iterator<Item = &AudioNode> {
		self.nodes.iter().filter(move |n| n.node_type == node_type)
	}

	pub fn get_default_node(&self, node_type: AudioNodeType) -> Option<&AudioNode> {
		let name = match node_type {
			AudioNodeType::Sink => self.default_sink.as_deref(),
			AudioNodeType::Source => self.default_source.as_deref(),
		}?;
		self
			.nodes
			.iter()
			.find(move |s| s.name == name && s.node_type == node_type)
	}

	pub fn get_device(&self, id: u32) -> Option<&AudioDevice> {
		self.devices.iter().find(|d| d.index == id)
	}

	pub fn same_layout(&self, other: &Self) -> bool {
		fn strip(nodes: &[AudioNode]) -> Vec<(u32, &str, &str, bool)> {
			nodes
				.iter()
				.map(|n| (n.index, n.name.as_str(), n.description.as_str(), n.mute))
				.collect()
		}

		strip(&self.nodes) == strip(&other.nodes)
			&& self.devices == other.devices
			&& self.default_sink == other.default_sink
			&& self.default_source == other.default_source
	}
}

type NodeIndex = u32;

#[derive(Clone, Debug)]
pub enum AudioTask {
	SetDefaultNode(AudioNodeType, NodeIndex),
	SetVolume(NodeIndex, f32),
	SetMute(NodeIndex, bool),
	AddDefaultSinkVolume(f32),
	SetCardProfile {
		card: NodeIndex,
		profile: i32,
		make_default_sink: bool,
	},
}
