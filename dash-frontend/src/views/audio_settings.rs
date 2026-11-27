use crate::frontend::{FrontendTask, FrontendTasks};
use std::collections::HashMap;
use std::{rc::Rc, sync::Arc};
use wgui::{
	assets::AssetPathSource,
	color::{WguiColor, WguiColorName},
	components::{
		self,
		button::{ButtonClickCallback, ComponentButton},
		checkbox::ComponentCheckbox,
		slider::ComponentSlider,
	},
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState, TemplateParams, strip_path_as},
	task::Tasks,
	widget::{ConstructEssentials, rectangle::WidgetRectangle},
};
use wlx_common::audio_devices::{self as ad, AudioDevice, AudioDeviceProfile, AudioNodeType, AudioSnapshot, AudioTask};

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
enum CurrentMode {
	Sinks,
	Sources,
	Cards,
	CardProfileSelector(ad::AudioDevice),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchType {
	Sink,
	#[allow(dead_code)]
	Source, // might be useful in the future
}

#[derive(Clone)]
struct IndexAndVolume {
	idx: u32,
	volume: f32,
}

#[derive(Clone)]
struct CardAndProfile {
	card: AudioDevice,
	profile_index: i32,
}

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
enum ViewTask {
	Remount,
	SetMode(CurrentMode),
	AutoSwitch,
	SetVolume(IndexAndVolume),
	SetCardProfile(CardAndProfile),
	Audio(AudioTask),
}

type ViewTasks = Tasks<ViewTask>;

pub struct View {
	tasks: ViewTasks,
	frontend_tasks: FrontendTasks, // used only for toasts
	on_update: Rc<dyn Fn()>,

	globals: WguiGlobals,

	state: ParserState,

	//entry: DesktopEntry,
	mode: CurrentMode,

	snapshot: Option<Arc<AudioSnapshot>>,
	mounted_revision: Option<u64>,

	id_devices: WidgetID,

	btn_sinks: Rc<ComponentButton>,
	btn_sources: Rc<ComponentButton>,
	btn_cards: Rc<ComponentButton>,
}

pub struct Params<'a> {
	pub globals: WguiGlobals,
	pub frontend_tasks: FrontendTasks,
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub on_update: Rc<dyn Fn()>,
}

struct ProfileDisplayName {
	name: String,
	icon_path: &'static str,
	is_vr: bool,
}

struct SelectorCell {
	key: String,
	display_text: String,
	icon_path_builtin: &'static str,
}

struct MultiSelectorParams<'a> {
	cells: &'a [SelectorCell],
	def_cell: &'a str,
	ess: &'a mut ConstructEssentials<'a>,
	on_click: Rc<dyn Fn(&str /* key */)>,
}

fn mount_multi_selector(params: MultiSelectorParams) -> anyhow::Result<()> {
	for cell in params.cells {
		let highlighted = cell.key == params.def_cell;
		let color = if highlighted {
			Some(WguiColor::from(WguiColorName::Primary))
		} else {
			None
		};

		// button
		let (_, button) = components::button::construct(
			params.ess,
			components::button::Params {
				text: Some(Translation::from_raw_text(cell.display_text.as_str())),
				sprite_src: Some(strip_path_as(AssetPathSource::BuiltIn, cell.icon_path_builtin).unwrap()),
				color,
				..Default::default()
			},
		)?;

		button.on_click({
			let on_click = params.on_click.clone();
			let key = cell.key.clone();
			Rc::new(move |_, _| {
				(*on_click)(key.as_str());
				Ok(())
			})
		});
	}

	Ok(())
}

fn does_string_mention_hmd_sink(input: &str) -> bool {
	let lwr = input.to_lowercase();
	lwr.contains("hmd") || // generic hmd name detected
	lwr.contains("index") || // Valve hardware
	lwr.contains("oculus") || // Oculus
	lwr.contains("rift") || // Also Oculus
	lwr.contains("beyond") ||  // Bigscreen Beyond
	lwr.contains("pimax") || // Pimax
	lwr.contains("wivrn") // WiVRn
}

fn does_string_mention_hmd_source(input: &str) -> bool {
	let lwr = input.to_lowercase();
	lwr.contains("hmd") || // generic hmd name detected
	lwr.contains("valve") || // Valve hardware
	lwr.contains("oculus") || // Oculus
	lwr.contains("beyond")  || // Bigscreen Beyond
	lwr.contains("pimax") || // Pimax
	lwr.contains("wivrn") // WiVRn
}

fn is_card_mentioning_hmd(card: &ad::AudioDevice) -> bool {
	does_string_mention_hmd_sink(&card.name)
}

#[allow(clippy::needless_bool)]
fn is_source_mentioning_hmd(source: &ad::Source) -> bool {
	does_string_mention_hmd_source(&source.name)
}

fn get_profile_display_name(profile: &AudioDeviceProfile, card: &AudioDevice) -> ProfileDisplayName {
	let mut out_icon_path: &'static str;
	let mut is_vr = false;

	let prof = profile.name.to_lowercase();
	if prof.contains("analog") {
		out_icon_path = "@/dashboard/minijack.svg";
	} else if prof.contains("iec" /* digital */) {
		out_icon_path = "@/dashboard/binary.svg";
	} else if prof.contains("hdmi") {
		out_icon_path = "@/dashboard/displayport.svg";
	} else if prof.contains("off") {
		out_icon_path = "@/dashboard/sleep.svg";
	} else if prof.contains("input") {
		out_icon_path = "@/dashboard/microphone.svg";
	} else {
		out_icon_path = "@/dashboard/volume.svg"; // Default fallback
	}

	// All ports are tied to this VR headset, assign all of them to the VR icon
	if is_card_mentioning_hmd(card) {
		if prof.contains("mic") {
			// Probably microphone
			out_icon_path = "@/dashboard/microphone.svg";
		} else {
			out_icon_path = "@/dashboard/vr.svg";
		}
	}

	let mut out_name: Option<String> = None;

	if profile.name.contains("stereo") {
		let product_name = card
			.routes
			.iter()
			.filter(|r| r.profile_ids.contains(&profile.index))
			.find_map(|r| r.product_name.as_deref());

		if let Some(product_name) = product_name {
			out_name = Some(product_name.to_owned());

			if does_string_mention_hmd_sink(product_name) {
				// VR icon
				out_icon_path = "@/dashboard/vr.svg";
				is_vr = true;
			} else {
				// Monitor icon
				out_icon_path = "@/dashboard/displayport.svg";
			}
		}
	}

	ProfileDisplayName {
		name: if let Some(name) = out_name {
			name
		} else {
			profile.description.clone()
		},
		icon_path: out_icon_path,
		is_vr,
	}
}

const DOC_PARAMS: ParseDocumentParams = ParseDocumentParams::builtin("gui/view/audio_settings.xml");

trait DeviceControl {
	fn on_volume_request(&self) -> anyhow::Result<f32>;
	fn on_check(&self) -> anyhow::Result<()>;
	fn on_mute_toggle(&self) -> anyhow::Result<()>;
	fn on_volume_change(&self, volume: f32) -> anyhow::Result<()>;
}

struct ControlNode {
	tasks: ViewTasks,
	on_update: Rc<dyn Fn()>,
	node: ad::AudioNode,
}

impl ControlNode {
	fn push(&self, task: ViewTask) {
		self.tasks.push(task);
		(*self.on_update)();
	}
}

impl DeviceControl for ControlNode {
	fn on_volume_request(&self) -> anyhow::Result<f32> {
		Ok(self.node.volume)
	}

	fn on_check(&self) -> anyhow::Result<()> {
		self.push(ViewTask::Audio(AudioTask::SetDefaultNode(
			self.node.node_type,
			self.node.index,
		)));
		Ok(())
	}

	fn on_mute_toggle(&self) -> anyhow::Result<()> {
		self.push(ViewTask::Audio(AudioTask::SetMute(self.node.index, !self.node.mute)));
		Ok(())
	}

	fn on_volume_change(&self, volume: f32) -> anyhow::Result<()> {
		self.push(ViewTask::SetVolume(IndexAndVolume {
			idx: self.node.index,
			volume,
		}));
		Ok(())
	}
}

struct MountCardParams<'a> {
	layout: &'a mut Layout,
	card: &'a ad::AudioDevice,
}

struct MountDeviceSliderParams<'a> {
	layout: &'a mut Layout,
	control: Rc<dyn DeviceControl>,
	checked: bool,
	muted: bool,
	disp: Option<ProfileDisplayName>,
	device_desc: String,
}

fn push_popup_speakers_set_successfully(globals: &WguiGlobals, frontend_tasks: &FrontendTasks, name: &str) {
	frontend_tasks.push(FrontendTask::PushToast(Translation::from_translation_key(
		format!(
			"{}: {}",
			globals.i18n().translate("AUDIO.SPEAKERS_SET_SUCCESSFULLY"),
			name
		)
		.as_str(),
	)));
}

fn push_popup_microphone_set_successfully(globals: &WguiGlobals, frontend_tasks: &FrontendTasks, name: &str) {
	frontend_tasks.push(FrontendTask::PushToast(Translation::from_translation_key(
		format!(
			"{}: {}",
			globals.i18n().translate("AUDIO.MICROPHONE_SET_SUCCESSFULLY"),
			name
		)
		.as_str(),
	)));
}

fn switch_sink_card(
	globals: &WguiGlobals,
	frontend_tasks: &FrontendTasks,
	audio_tasks: &mut Vec<AudioTask>,
	card: &ad::AudioDevice,
	profile_index: i32,
	name: &ProfileDisplayName,
) {
	audio_tasks.push(AudioTask::SetCardProfile {
		card: card.index,
		profile: profile_index,
		make_default_sink: true,
	});
	push_popup_speakers_set_successfully(globals, frontend_tasks, &name.name);
}

fn switch_source(
	globals: &WguiGlobals,
	frontend_tasks: &FrontendTasks,
	audio_tasks: &mut Vec<AudioTask>,
	source: &ad::AudioNode,
) {
	audio_tasks.push(AudioTask::SetDefaultNode(AudioNodeType::Source, source.index));
	push_popup_microphone_set_successfully(globals, frontend_tasks, &source.description);
}

fn switch_to_vr_microphone(
	globals: &WguiGlobals,
	frontend_tasks: &FrontendTasks,
	audio_tasks: &mut Vec<AudioTask>,
	snapshot: &AudioSnapshot,
) {
	if let Some(source) = snapshot
		.nodes(AudioNodeType::Source)
		.find(|s| is_source_mentioning_hmd(s))
	{
		switch_source(globals, frontend_tasks, audio_tasks, source);
	} else {
		frontend_tasks.push(FrontendTask::PushToast(Translation::from_translation_key(
			"AUDIO.NO_VR_MICROPHONE_SWITCH_MANUALLY",
		)));
	}
}

#[derive(Debug, Clone)]
struct CardPriorityResult<'a> {
	card: &'a AudioDevice,
	profile: &'a AudioDeviceProfile,
}

fn get_card_best_profile<'a>(card: &'a AudioDevice, search_type: SearchType) -> Option<CardPriorityResult<'a>> {
	let mut best_profile: Option<&'a AudioDeviceProfile> = None;

	for profile in &card.profiles {
		match search_type {
			SearchType::Sink if profile.sink_count == 0 => continue,
			SearchType::Source if profile.source_count == 0 => continue,
			_ => {}
		}

		if best_profile.is_none_or(|p| profile.priority > p.priority) {
			best_profile = Some(profile);
		}
	}

	Some(CardPriorityResult {
		card,
		profile: best_profile?,
	})
}

fn get_best_profile_from_array<'a>(arr: &[CardPriorityResult<'a>]) -> Option<CardPriorityResult<'a>> {
	arr.iter().max_by_key(|r| r.profile.priority).cloned()
}

fn switch_to_vr_speakers(
	globals: &WguiGlobals,
	frontend_tasks: &FrontendTasks,
	audio_tasks: &mut Vec<AudioTask>,
	snapshot: &AudioSnapshot,
) {
	let cards = &snapshot.devices;
	let mut best_profiles = Vec::new();

	// Check for WiVRn presence
	if let Some(sink) = snapshot.nodes(AudioNodeType::Sink).find(|s| s.name.contains("wivrn")) {
		audio_tasks.push(AudioTask::SetDefaultNode(AudioNodeType::Sink, sink.index));
		push_popup_speakers_set_successfully(globals, frontend_tasks, "WiVRn");
		return;
	}

	for card in cards {
		if !is_card_mentioning_hmd(card) {
			continue;
		}
		if let Some(best_profile) = get_card_best_profile(card, SearchType::Sink) {
			best_profiles.push(best_profile);
		}
	}

	if !best_profiles.is_empty() {
		let best_profile = get_best_profile_from_array(&best_profiles).unwrap();
		let name = get_profile_display_name(best_profile.profile, best_profile.card);
		switch_sink_card(
			globals,
			frontend_tasks,
			audio_tasks,
			best_profile.card,
			best_profile.profile.index,
			&name,
		);
		return;
	}

	// There aren't any cards which mention VR explicitly. Time for plan B.
	for card in cards {
		for profile in &card.profiles {
			let name = get_profile_display_name(profile, card);
			if !name.is_vr {
				continue;
			}
			switch_sink_card(globals, frontend_tasks, audio_tasks, card, profile.index, &name);
			return;
		}
	}

	frontend_tasks.push(FrontendTask::PushToast(Translation::from_translation_key(
		"AUDIO.NO_VR_SPEAKERS_FOUND_SWITCH_MANUALLY",
	)));
}

const ONE_HUNDRED_PERCENT: f32 = 100.0;
const REVISION_UNAVAILABLE: u64 = u64::MAX;
const VOLUME_MULT: f32 = 1.0 / ONE_HUNDRED_PERCENT;

impl View {
	fn handle_func_button_click(&self, task: ViewTask) -> ButtonClickCallback {
		let tasks = self.tasks.clone();
		let on_update = self.on_update.clone();
		Rc::new(move |_common, _evt| {
			tasks.push(task.clone());
			(*on_update)();
			Ok(())
		})
	}

	pub fn new(params: Params) -> anyhow::Result<Self> {
		let tasks = ViewTasks::new();

		let state = wgui::parser::parse_from_assets(&DOC_PARAMS, params.layout, params.parent_id)?;

		let id_devices = state.get_widget_id("devices")?;

		let btn_sinks = state.fetch_component_as::<ComponentButton>("btn_sinks")?;
		let btn_sources = state.fetch_component_as::<ComponentButton>("btn_sources")?;
		let btn_cards = state.fetch_component_as::<ComponentButton>("btn_cards")?;
		let btn_auto = state.fetch_component_as::<ComponentButton>("btn_auto")?;

		let res = Self {
			globals: params.globals,
			frontend_tasks: params.frontend_tasks,
			state,
			mode: CurrentMode::Sinks,
			snapshot: None,
			mounted_revision: None,
			id_devices,
			tasks,
			on_update: params.on_update,
			btn_sinks: btn_sinks.clone(),
			btn_sources: btn_sources.clone(),
			btn_cards: btn_cards.clone(),
		};

		btn_sinks.on_click(res.handle_func_button_click(ViewTask::SetMode(CurrentMode::Sinks)));
		btn_sources.on_click(res.handle_func_button_click(ViewTask::SetMode(CurrentMode::Sources)));
		btn_cards.on_click(res.handle_func_button_click(ViewTask::SetMode(CurrentMode::Cards)));
		btn_auto.on_click(res.handle_func_button_click(ViewTask::AutoSwitch));

		// the device list gets mounted on the first update()
		res.update_button_highlights(params.layout)?;

		Ok(res)
	}

	fn update_button_highlights(&self, layout: &mut Layout) -> anyhow::Result<()> {
		let num: u8 = match &self.mode {
			CurrentMode::Sinks => 0,
			CurrentMode::Sources => 1,
			CurrentMode::Cards => 2,
			CurrentMode::CardProfileSelector(_) => 255,
		};

		let mut com = layout.common();

		let mut perform = |btn_num: u8, btn: &Rc<ComponentButton>| {
			let color = if num == btn_num {
				WguiColorName::Primary
			} else {
				WguiColorName::BackgroundVariant
			};
			btn.set_color(&mut com, color.into());
		};

		perform(0, &self.btn_sinks);
		perform(1, &self.btn_sources);
		perform(2, &self.btn_cards);

		Ok(())
	}

	fn process_tasks(&mut self, layout: &mut Layout, audio_tasks: &mut Vec<AudioTask>) -> anyhow::Result<bool> {
		let tasks = self.tasks.drain();
		if tasks.is_empty() {
			return Ok(false);
		}

		let mut set_volume: HashMap<u32, f32> = HashMap::new();

		for task in tasks {
			match task {
				ViewTask::Remount => {
					self.mounted_revision = Some(self.current_revision());
					self.update_button_highlights(layout)?;
					match self.mode.clone() {
						CurrentMode::Sinks | CurrentMode::Sources => {
							let node_type = match self.mode.clone() {
								CurrentMode::Sinks => AudioNodeType::Sink,
								CurrentMode::Sources => AudioNodeType::Source,
								_ => unreachable!(),
							};
							self.init_mode_nodes(layout, node_type)?
						}
						CurrentMode::Cards => self.init_mode_cards(layout)?,
						CurrentMode::CardProfileSelector(card) => {
							// refresh the card, it could've changed or disappeared
							match self
								.snapshot
								.as_ref()
								.and_then(|s| s.devices.iter().find(|c| c.index == card.index))
							{
								Some(card) => self.init_mode_card_selector(layout, card.clone())?,
								None => {
									self.mode = CurrentMode::Cards;
									self.tasks.push(ViewTask::Remount);
								}
							}
						}
					}
				}
				ViewTask::SetVolume(s) => {
					set_volume.insert(s.idx, s.volume);
				}
				ViewTask::SetMode(current_mode) => {
					self.mode = current_mode;
					self.tasks.push(ViewTask::Remount);
				}
				ViewTask::SetCardProfile(c) => {
					audio_tasks.push(AudioTask::SetCardProfile {
						card: c.card.index,
						profile: c.profile_index,
						make_default_sink: false,
					});
				}
				ViewTask::Audio(task) => audio_tasks.push(task),
				ViewTask::AutoSwitch => {
					if let Some(snapshot) = self.snapshot.clone() {
						switch_to_vr_speakers(&self.globals, &self.frontend_tasks, audio_tasks, &snapshot);
						switch_to_vr_microphone(&self.globals, &self.frontend_tasks, audio_tasks, &snapshot);
					}
				}
			}
		}

		// set volume only to the latest event (prevent cpu time starvation
		// due to excessive input motion events)
		for (idx, volume) in set_volume {
			audio_tasks.push(AudioTask::SetVolume(idx, volume));
		}

		Ok(true)
	}

	fn current_revision(&self) -> u64 {
		self
			.snapshot
			.as_ref()
			.map_or(REVISION_UNAVAILABLE, |s| s.layout_revision)
	}

	// call every frame, returns the audio changes requested by the user
	pub fn update(
		&mut self,
		layout: &mut Layout,
		snapshot: Option<Arc<AudioSnapshot>>,
	) -> anyhow::Result<Vec<AudioTask>> {
		self.snapshot = snapshot;
		if self.mounted_revision != Some(self.current_revision()) {
			self.tasks.push(ViewTask::Remount);
		}

		let mut audio_tasks = Vec::new();
		while self.process_tasks(layout, &mut audio_tasks)? {}

		Ok(audio_tasks)
	}

	fn mount_card(&mut self, params: MountCardParams) -> anyhow::Result<()> {
		let desc = &params.card.description;
		let profile_name = params
			.card
			.active_profile()
			.map(|p| get_profile_display_name(p, params.card).name)
			.unwrap_or_default();

		let mut par = TemplateParams::new();
		par.insert("card_name", desc.as_str());
		par.insert("profile_name", profile_name.as_str());

		let data = self
			.state
			.realize_template(&DOC_PARAMS, "Card", params.layout, self.id_devices, par)?;

		let btn_card = data.fetch_component_as::<ComponentButton>("btn_card")?;
		btn_card.on_click({
			let tasks = self.tasks.clone();
			let card = params.card.clone();
			let on_update = self.on_update.clone();
			Rc::new(move |_common, _evt| {
				tasks.push(ViewTask::SetMode(CurrentMode::CardProfileSelector(card.clone())));
				(*on_update)();
				Ok(())
			})
		});

		Ok(())
	}

	fn mount_device_slider(&mut self, params: MountDeviceSliderParams) -> anyhow::Result<()> {
		let mut par = TemplateParams::new();

		if let Some(disp) = &params.disp {
			par.insert("device_name", disp.name.as_str());
			par.insert("device_icon", disp.icon_path);
		} else {
			let icon_path = if params.device_desc.contains("WiVRn") {
				"@/dashboard/wivrn_head_symbolic.svg"
			} else {
				"@/dashboard/binary.svg"
			};

			par.insert("device_name", params.device_desc.as_str());
			par.insert("device_icon", icon_path);
		}

		par.insert(
			"volume_icon",
			if params.muted {
				"@/dashboard/volume_off.svg"
			} else {
				"@/dashboard/volume.svg"
			},
		);

		let data = self
			.state
			.realize_template(&DOC_PARAMS, "DeviceSlider", params.layout, self.id_devices, par)?;

		if params.checked {
			let rect = data.fetch_widget(&params.layout.state, "rect")?;
			let mut rect = rect.widget.cast::<WidgetRectangle>()?;
			rect.set_color(&mut params.layout.common(), WguiColorName::BackgroundVariant.into());
			rect.set_border_color(&mut params.layout.common(), WguiColorName::Primary.into());
		}

		let mut common = params.layout.common();
		let checkbox = data.fetch_component_as::<ComponentCheckbox>("checkbox")?;
		let btn_mute = data.fetch_component_as::<ComponentButton>("btn_mute")?;
		let slider = data.fetch_component_as::<ComponentSlider>("slider")?;

		slider.set_value_primary(&mut common, params.control.on_volume_request()? / VOLUME_MULT);

		checkbox.set_checked(&mut common, params.checked);

		checkbox.on_toggle({
			let control = params.control.clone();
			Box::new(move |_common, _event| {
				control.on_check()?;
				Ok(())
			})
		});

		slider.on_value_changed({
			let control = params.control.clone();
			Box::new(move |_common, event| {
				if let Err(e) = control.on_volume_change(event.value * VOLUME_MULT) {
					log::error!("{:?}", e);
					debug_assert!(false);
				};
			})
		});

		btn_mute.on_click({
			let control = params.control.clone();
			Rc::new(move |_common, _event| {
				control.on_mute_toggle()?;
				Ok(())
			})
		});

		Ok(())
	}

	fn init_mode_nodes(&mut self, layout: &mut Layout, node_type: AudioNodeType) -> anyhow::Result<()> {
		layout.remove_children(self.id_devices);

		let Some(snapshot) = self.snapshot.clone() else {
			return Ok(());
		};

		let nodes = snapshot.nodes(node_type);
		let default_node = snapshot.get_default_node(node_type);

		for node in nodes {
			let device = node.device_id.and_then(|device_id| snapshot.get_device(device_id));

			let checked = default_node.is_some_and(|d| d.index == node.index);

			let control = Rc::new(ControlNode {
				tasks: self.tasks.clone(),
				on_update: self.on_update.clone(),
				node: node.clone(),
			});

			let mut disp = device.and_then(|card| card.active_profile().map(|p| get_profile_display_name(p, card)));
			if let Some(disp) = &mut disp {
				disp.name = node.nick.clone().unwrap_or_else(|| node.description.clone());
			}

			self.mount_device_slider(MountDeviceSliderParams {
				checked,
				disp,
				device_desc: node.description.clone(),
				layout,
				control,
				muted: node.mute,
			})?;
		}

		Ok(())
	}

	fn init_mode_cards(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		log::info!("refreshing card list");

		layout.remove_children(self.id_devices);

		let Some(snapshot) = self.snapshot.clone() else {
			return Ok(());
		};

		for card in &snapshot.devices {
			self.mount_card(MountCardParams { layout, card })?;
		}

		Ok(())
	}

	fn init_mode_card_selector(&mut self, layout: &mut Layout, card: ad::AudioDevice) -> anyhow::Result<()> {
		log::info!("showing card selector for {}", card.name);

		layout.remove_children(self.id_devices);

		{
			let data = self.state.realize_template(
				&DOC_PARAMS,
				"SelectAudioProfileText",
				layout,
				self.id_devices,
				Default::default(),
			)?;
			let btn_back = data.fetch_component_as::<ComponentButton>("btn_back")?;
			btn_back.on_click({
				let tasks = self.tasks.clone();
				let on_update = self.on_update.clone();
				Rc::new(move |_, _| {
					tasks.push(ViewTask::SetMode(CurrentMode::Cards));
					(*on_update)();
					Ok(())
				})
			});
		}

		let mut cells = Vec::<SelectorCell>::new();
		for profile in &card.profiles {
			if profile.name.contains("surround") {
				continue; // we aren't interested in that
			}

			let disp_name = get_profile_display_name(profile, &card);
			cells.push(SelectorCell {
				key: profile.name.clone(),
				display_text: disp_name.name,
				icon_path_builtin: disp_name.icon_path,
			});
		}

		let mut ess = ConstructEssentials {
			layout,
			parent: self.id_devices,
		};

		mount_multi_selector(MultiSelectorParams {
			cells: &cells,
			def_cell: card.active_profile().map(|p| p.name.as_str()).unwrap_or("Unknown"),
			ess: &mut ess,
			on_click: {
				let card = card.clone();
				let tasks = self.tasks.clone();
				let on_update = self.on_update.clone();
				Rc::new(move |profile_name| {
					let Some(profile) = card.profile_by_name(profile_name) else {
						return;
					};
					tasks.push(ViewTask::SetCardProfile(CardAndProfile {
						card: card.clone(),
						profile_index: profile.index,
					}));
					tasks.push(ViewTask::SetMode(CurrentMode::Cards));
					(*on_update)();
				})
			},
		})?;

		Ok(())
	}
}
