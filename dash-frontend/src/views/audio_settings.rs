use std::{collections::HashMap, rc::Rc};

use wgui::{
	assets::AssetPath,
	components::{
		self,
		button::{ButtonClickCallback, ComponentButton},
		checkbox::ComponentCheckbox,
		slider::ComponentSlider,
	},
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	task::Tasks,
	widget::ConstructEssentials,
};

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::pactl_wrapper::{self},
};

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
enum CurrentMode {
	Sinks,
	Sources,
	Cards,
	CardProfileSelector(pactl_wrapper::Card),
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
struct CardAndProfileName {
	card: pactl_wrapper::Card,
	profile_name: String,
}

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
enum ViewTask {
	Remount,
	SetMode(CurrentMode),
	AutoSwitch,
	SetSinkVolume(IndexAndVolume),
	SetSourceVolume(IndexAndVolume),
	SetCardProfile(CardAndProfileName),
}

type ViewTasks = Tasks<ViewTask>;

pub struct View {
	tasks: ViewTasks,
	frontend_tasks: FrontendTasks, // used only for toasts
	on_update: Rc<dyn Fn()>,

	globals: WguiGlobals,

	#[allow(dead_code)]
	state: ParserState,

	//entry: DesktopEntry,
	mode: CurrentMode,

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
	icon_path: &'static str,
}

struct MultiSelectorParams<'a> {
	cells: &'a [SelectorCell],
	def_cell: &'a str,
	ess: &'a mut ConstructEssentials<'a>,
	on_click: Rc<dyn Fn(&str /* key */)>,
}

fn mount_multi_selector(params: MultiSelectorParams) -> anyhow::Result<()> {
	let accent_color = params.ess.layout.state.theme.accent_color;

	for cell in params.cells {
		let highlighted = cell.key == params.def_cell;
		let color = if highlighted { Some(accent_color) } else { None };

		// button
		let (_, button) = components::button::construct(
			params.ess,
			components::button::Params {
				text: Some(Translation::from_raw_text(&cell.display_text)),
				sprite_src: Some(AssetPath::BuiltIn(cell.icon_path)),
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

fn get_card_from_sink<'a>(
	sink: &pactl_wrapper::Sink,
	cards: &'a [pactl_wrapper::Card],
) -> Option<&'a pactl_wrapper::Card> {
	let Some(sink_dev_name) = &sink.properties.get("device.name") else {
		return None;
	};

	cards.iter().find(|&card| **sink_dev_name == card.name).map(|v| v as _)
}

fn get_card_from_source<'a>(
	source: &pactl_wrapper::Source,
	cards: &'a [pactl_wrapper::Card],
) -> Option<&'a pactl_wrapper::Card> {
	let Some(source_dev_name) = &source.properties.device_name else {
		return None;
	};

	cards
		.iter()
		.find(|&card| **source_dev_name == card.name)
		.map(|v| v as _)
}

fn does_string_mention_hmd_sink(input: &str) -> bool {
	let lwr = input.to_lowercase();
	lwr.contains("hmd") || // generic hmd name detected
	lwr.contains("index") || // Valve hardware
	lwr.contains("oculus") || // Oculus
	lwr.contains("rift") || // Also Oculus
	lwr.contains("beyond") ||  // Bigscreen Beyond
	lwr.contains("wivrn") // WiVRn
}

fn does_string_mention_hmd_source(input: &str) -> bool {
	let lwr = input.to_lowercase();
	lwr.contains("hmd") || // generic hmd name detected
	lwr.contains("valve") || // Valve hardware
	lwr.contains("oculus") || // Oculus
	lwr.contains("beyond")  || // Bigscreen Beyond
	lwr.contains("wivrn") // WiVRn
}

fn is_card_mentioning_hmd(card: &pactl_wrapper::Card) -> bool {
	does_string_mention_hmd_sink(&card.properties.device_name)
}

fn is_source_mentioning_hmd(source: &pactl_wrapper::Source) -> bool {
	if let Some(source_card_name) = &source.properties.card_name
		&& does_string_mention_hmd_source(source_card_name)
	{
		return true;
	}

	// WiVRn
	if source.name == "wivrn.source" {
		return true;
	}

	false
}

fn get_profile_display_name(profile_name: &str, card: &pactl_wrapper::Card) -> ProfileDisplayName {
	let Some(profile) = card.profiles.get(profile_name) else {
		// fallback
		return ProfileDisplayName {
			name: profile_name.into(),
			icon_path: "dashboard/binary.svg",
			is_vr: false,
		};
	};

	let mut out_icon_path: &'static str;
	let mut is_vr = false;

	let prof = profile_name.to_lowercase();
	if prof.contains("analog") {
		out_icon_path = "dashboard/minijack.svg";
	} else if prof.contains("iec" /* digital */) {
		out_icon_path = "dashboard/binary.svg";
	} else if prof.contains("hdmi") {
		out_icon_path = "dashboard/displayport.svg";
	} else if prof.contains("off") {
		out_icon_path = "dashboard/sleep.svg";
	} else if prof.contains("input") {
		out_icon_path = "dashboard/microphone.svg";
	} else {
		out_icon_path = "dashboard/volume.svg"; // Default fallback
	}

	// All ports are tied to this VR headset, assign all of them to the VR icon
	if is_card_mentioning_hmd(card) {
		if prof.contains("mic") {
			// Probably microphone
			out_icon_path = "dashboard/microphone.svg";
		} else {
			out_icon_path = "dashboard/vr.svg";
		}
	}

	let mut out_name: Option<String> = None;

	for port in card.ports.values() {
		// Find profile
		for port_profile in &port.profiles {
			if !port_profile.contains("stereo") {
				continue; // we only want stereo, not surround or other types
			}

			if port_profile != profile_name {
				continue;
			}

			// Exact match! Use its device name
			let Some(product_name) = port.properties.get("device.product.name") else {
				continue;
			};

			out_name = Some(product_name.clone());

			if does_string_mention_hmd_sink(product_name) {
				// VR icon
				out_icon_path = "dashboard/vr.svg";
				is_vr = true;
			} else {
				// Monitor icon
				out_icon_path = "dashboard/displayport.svg";
			}

			break;
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

fn doc_params(globals: &WguiGlobals) -> ParseDocumentParams<'_> {
	ParseDocumentParams {
		globals: globals.clone(),
		path: AssetPath::BuiltIn("gui/view/audio_settings.xml"),
		extra: Default::default(),
	}
}

trait DeviceControl {
	fn on_volume_request(&self) -> anyhow::Result<f32>;
	fn on_check(&self) -> anyhow::Result<()>;
	fn on_mute_toggle(&self) -> anyhow::Result<()>;
	fn on_volume_change(&self, volume: f32) -> anyhow::Result<()>;
}

struct ControlSink {
	tasks: ViewTasks,
	on_update: Rc<dyn Fn()>,
	sink: pactl_wrapper::Sink,
}

impl ControlSink {
	fn new(tasks: ViewTasks, on_update: Rc<dyn Fn()>, sink: pactl_wrapper::Sink) -> Self {
		Self { tasks, sink, on_update }
	}
}

impl DeviceControl for ControlSink {
	fn on_volume_request(&self) -> anyhow::Result<f32> {
		let volume = pactl_wrapper::get_sink_volume(&self.sink)?;
		Ok(volume)
	}

	fn on_check(&self) -> anyhow::Result<()> {
		pactl_wrapper::set_default_sink(self.sink.index)?;
		self.tasks.push(ViewTask::Remount);
		(*self.on_update)();
		Ok(())
	}

	fn on_mute_toggle(&self) -> anyhow::Result<()> {
		pactl_wrapper::set_sink_mute(self.sink.index, !self.sink.mute)?;
		self.tasks.push(ViewTask::Remount);
		(*self.on_update)();
		Ok(())
	}

	fn on_volume_change(&self, volume: f32) -> anyhow::Result<()> {
		self.tasks.push(ViewTask::SetSinkVolume(IndexAndVolume {
			idx: self.sink.index,
			volume,
		}));
		(*self.on_update)();
		Ok(())
	}
}

struct ControlSource {
	tasks: ViewTasks,
	on_update: Rc<dyn Fn()>,
	source: pactl_wrapper::Source,
}

impl ControlSource {
	fn new(tasks: ViewTasks, on_update: Rc<dyn Fn()>, source: pactl_wrapper::Source) -> Self {
		Self {
			tasks,
			source,
			on_update,
		}
	}
}

impl DeviceControl for ControlSource {
	fn on_volume_request(&self) -> anyhow::Result<f32> {
		let volume = pactl_wrapper::get_source_volume(&self.source)?;
		Ok(volume)
	}

	fn on_check(&self) -> anyhow::Result<()> {
		pactl_wrapper::set_default_source(self.source.index)?;
		self.tasks.push(ViewTask::Remount);
		(*self.on_update)();
		Ok(())
	}

	fn on_mute_toggle(&self) -> anyhow::Result<()> {
		pactl_wrapper::set_source_mute(self.source.index, !self.source.mute)?;
		self.tasks.push(ViewTask::Remount);
		(*self.on_update)();
		Ok(())
	}

	fn on_volume_change(&self, volume: f32) -> anyhow::Result<()> {
		self.tasks.push(ViewTask::SetSourceVolume(IndexAndVolume {
			idx: self.source.index,
			volume,
		}));
		(*self.on_update)();
		Ok(())
	}
}

struct MountCardParams<'a> {
	layout: &'a mut Layout,
	card: &'a pactl_wrapper::Card,
}

struct MountDeviceSliderParams<'a> {
	layout: &'a mut Layout,
	control: Rc<dyn DeviceControl>,
	checked: bool,
	muted: bool,
	disp: Option<ProfileDisplayName>,
	alt_desc: String,
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
	card: &pactl_wrapper::Card,
	profile_name: &str,
	name: &ProfileDisplayName,
) -> anyhow::Result<()> {
	let card_index = card.index;
	let profile = profile_name.to_string();
	pactl_wrapper::set_card_profile(card_index, &profile)?;
	let sinks = pactl_wrapper::list_sinks()?;

	let mut sink_found = false;
	for sink in &sinks {
		if let Some(device_name) = sink.properties.get("device.name")
			&& device_name == &card.name
		{
			pactl_wrapper::set_default_sink(sink.index)?;
			sink_found = true;
			break;
		}
	}

	if sink_found {
		push_popup_speakers_set_successfully(globals, frontend_tasks, &name.name);
	} else {
		frontend_tasks.push(FrontendTask::PushToast(Translation::from_translation_key(
			format!("Card found ({}), but no matching speakers found", name.name).as_str(),
		)));
	}

	Ok(())
}

fn switch_source(
	globals: &WguiGlobals,
	frontend_tasks: &FrontendTasks,
	source: &pactl_wrapper::Source,
) -> anyhow::Result<()> {
	match pactl_wrapper::set_default_source(source.index) {
		Ok(()) => {
			push_popup_microphone_set_successfully(
				globals,
				frontend_tasks,
				if let Some(card_name) = &source.properties.card_name {
					card_name
				} else {
					&source.description
				},
			);
			Ok(())
		}
		Err(e) => {
			Translation::from_translation_key(format!("[AUDIO.FAILED_TO_SWITCH_MICROPHONE]: {:?}", e).as_str());
			Err(e)
		}
	}
}

fn switch_to_vr_microphone(globals: &WguiGlobals, frontend_tasks: &FrontendTasks) -> anyhow::Result<()> {
	let sources = pactl_wrapper::list_sources()?;
	let mut switched = false;

	for source in &sources {
		if is_source_mentioning_hmd(source) {
			switch_source(globals, frontend_tasks, source)?;
			switched = true;
			break;
		}
	}

	if !switched {
		frontend_tasks.push(FrontendTask::PushToast(Translation::from_translation_key(
			"AUDIO.NO_VR_MICROPHONE_SWITCH_MANUALLY",
		)));
	}

	Ok(())
}

#[derive(Debug, Clone)]
struct CardPriorityResult<'a> {
	priority: u32,
	name: String,
	card: &'a pactl_wrapper::Card,
}

fn get_card_best_profile<'a>(card: &'a pactl_wrapper::Card, search_type: SearchType) -> Option<CardPriorityResult<'a>> {
	let mut best_priority = 0;
	let mut best_profile_name = "";
	let mut best_profile: Option<&'a pactl_wrapper::CardProfile> = None;

	for (profile_name, profile) in &card.profiles {
		match search_type {
			SearchType::Sink if profile.sinks == 0 => continue,
			SearchType::Source if profile.sources == 0 => continue,
			_ => {}
		}

		if profile.priority > best_priority {
			best_priority = profile.priority;
			best_profile = Some(profile);
			best_profile_name = profile_name;
		}
	}

	best_profile?; // do not proceed if no profile was found

	Some(CardPriorityResult {
		priority: best_priority,
		name: best_profile_name.to_string(),
		card,
	})
}

fn get_best_profile_from_array<'a>(arr: &[CardPriorityResult<'a>]) -> Option<CardPriorityResult<'a>> {
	let mut best_priority = 0;
	let mut res: Option<CardPriorityResult<'a>> = None;

	for cell in arr {
		if cell.priority > best_priority {
			best_priority = cell.priority;
			res = Some(cell.clone());
		}
	}

	res
}

fn switch_to_vr_speakers(globals: &WguiGlobals, frontend_tasks: &FrontendTasks) -> anyhow::Result<()> {
	let cards = pactl_wrapper::list_cards()?;
	let sinks = pactl_wrapper::list_sinks()?;
	let mut best_profiles = Vec::new();

	// Check for WiVRn presence
	for sink in sinks {
		if sink.name.contains("wivrn") {
			pactl_wrapper::set_default_sink(sink.index)?;
			push_popup_speakers_set_successfully(globals, frontend_tasks, "WiVRn");
			return Ok(());
		}
	}

	for card in &cards {
		if !is_card_mentioning_hmd(card) {
			continue;
		}
		if let Some(best_profile) = get_card_best_profile(card, SearchType::Sink) {
			best_profiles.push(best_profile);
		}
	}

	if !best_profiles.is_empty() {
		let best_profile = get_best_profile_from_array(&best_profiles).unwrap();
		let name = get_profile_display_name(&best_profile.name, best_profile.card);
		switch_sink_card(globals, frontend_tasks, best_profile.card, &best_profile.name, &name)?;
		return Ok(());
	}

	// There aren't any cards which mention VR explicitly. Time for plan B.
	for card in &cards {
		for profile_name in card.profiles.keys() {
			let name = get_profile_display_name(profile_name, card);
			if !name.is_vr {
				continue;
			}
			switch_sink_card(globals, frontend_tasks, card, profile_name, &name)?;
			return Ok(());
		}
	}

	frontend_tasks.push(FrontendTask::PushToast(Translation::from_translation_key(
		"AUDIO.NO_VR_SPEAKERS_FOUND_SWITCH_MANUALLY",
	)));

	Ok(())
}

const ONE_HUNDRED_PERCENT: f32 = 100.0;
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

		let state = wgui::parser::parse_from_assets(&doc_params(&params.globals), params.layout, params.parent_id)?;

		let id_devices = state.get_widget_id("devices")?;

		let btn_sinks = state.fetch_component_as::<ComponentButton>("btn_sinks")?;
		let btn_sources = state.fetch_component_as::<ComponentButton>("btn_sources")?;
		let btn_cards = state.fetch_component_as::<ComponentButton>("btn_cards")?;
		let btn_auto = state.fetch_component_as::<ComponentButton>("btn_auto")?;

		let mut res = Self {
			globals: params.globals,
			frontend_tasks: params.frontend_tasks,
			state,
			mode: CurrentMode::Sinks,
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

		res.init_mode_sinks(params.layout)?;
		res.update_button_highlights(params.layout)?;

		Ok(res)
	}

	fn update_button_highlights(&self, layout: &mut Layout) -> anyhow::Result<()> {
		let mut c = layout.start_common();
		let mut common = c.common();

		let num: u8 = match &self.mode {
			CurrentMode::Sinks => 0,
			CurrentMode::Sources => 1,
			CurrentMode::Cards => 2,
			CurrentMode::CardProfileSelector(_) => 255,
		};

		let mut perform = |btn_num: u8, btn: &Rc<ComponentButton>| {
			let color = if num == btn_num {
				common.state.theme.accent_color
			} else {
				common.state.theme.button_color
			};
			btn.set_color(&mut common, color);
		};

		perform(0, &self.btn_sinks);
		perform(1, &self.btn_sources);
		perform(2, &self.btn_cards);

		c.finish()?;
		Ok(())
	}

	fn process_tasks(&mut self, layout: &mut Layout) -> anyhow::Result<bool> {
		let tasks = self.tasks.drain();
		if tasks.is_empty() {
			return Ok(false);
		}

		let mut set_sink_volume: Option<IndexAndVolume> = None;
		let mut set_source_volume: Option<IndexAndVolume> = None;

		for task in tasks {
			match task {
				ViewTask::Remount => {
					self.update_button_highlights(layout)?;
					match &self.mode {
						CurrentMode::Sinks => self.init_mode_sinks(layout)?,
						CurrentMode::Sources => self.init_mode_sources(layout)?,
						CurrentMode::Cards => self.init_mode_cards(layout)?,
						CurrentMode::CardProfileSelector(card) => self.init_mode_card_selector(layout, card.clone())?,
					}
				}
				ViewTask::SetSinkVolume(s) => {
					set_sink_volume = Some(s);
				}
				ViewTask::SetSourceVolume(s) => {
					set_source_volume = Some(s);
				}
				ViewTask::SetMode(current_mode) => {
					self.mode = current_mode;
					self.tasks.push(ViewTask::Remount);
				}
				ViewTask::SetCardProfile(c) => {
					pactl_wrapper::set_card_profile(c.card.index, &c.profile_name)?;
				}
				ViewTask::AutoSwitch => {
					switch_to_vr_speakers(&self.globals, &self.frontend_tasks)?;
					switch_to_vr_microphone(&self.globals, &self.frontend_tasks)?;
					self.tasks.push(ViewTask::Remount);
				}
			}
		}

		// set volume only to the latest event (prevent cpu time starvation
		// due to excessive input motion events)
		if let Some(s) = set_sink_volume {
			pactl_wrapper::set_sink_volume(s.idx, s.volume)?;
		}

		if let Some(s) = set_source_volume {
			pactl_wrapper::set_source_volume(s.idx, s.volume)?;
		}

		Ok(true)
	}

	pub fn update(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		while self.process_tasks(layout)? {}

		Ok(())
	}

	fn mount_card(&mut self, params: MountCardParams) -> anyhow::Result<()> {
		let desc = &params.card.properties.device_description;
		let disp_name = get_profile_display_name(&params.card.active_profile, params.card);

		let mut par = HashMap::<Rc<str>, Rc<str>>::new();
		par.insert("card_name".into(), desc.as_str().into());
		par.insert("profile_name".into(), disp_name.name.as_str().into());

		let data = self
			.state
			.parse_template(&doc_params(&self.globals), "Card", params.layout, self.id_devices, par)?;

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

		log::info!("mount card TODO: {}", params.card.name);
		Ok(())
	}

	fn mount_device_slider(&mut self, params: MountDeviceSliderParams) -> anyhow::Result<()> {
		let mut par = HashMap::<Rc<str>, Rc<str>>::new();

		if let Some(disp) = &params.disp {
			par.insert("device_name".into(), disp.name.as_str().into());
			par.insert("device_icon".into(), disp.icon_path.into());
		} else {
			let icon_path = if params.alt_desc.contains("WiVRn") {
				"dashboard/wivrn_head_symbolic.svg"
			} else {
				"dashboard/binary.svg"
			};

			par.insert("device_name".into(), params.alt_desc.into());
			par.insert("device_icon".into(), icon_path.into());
		}

		par.insert(
			"volume_icon".into(),
			if params.muted {
				"dashboard/volume_off.svg".into()
			} else {
				"dashboard/volume.svg".into()
			},
		);

		let data = self.state.parse_template(
			&doc_params(&self.globals),
			"DeviceSlider",
			params.layout,
			self.id_devices,
			par,
		)?;

		let mut c = params.layout.start_common();
		let mut common = c.common();

		let checkbox = data.fetch_component_as::<ComponentCheckbox>("checkbox")?;
		let btn_mute = data.fetch_component_as::<ComponentButton>("btn_mute")?;
		let slider = data.fetch_component_as::<ComponentSlider>("slider")?;

		slider.set_value(&mut common, params.control.on_volume_request()? / VOLUME_MULT);

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
				control.on_volume_change(event.value * VOLUME_MULT)?;
				Ok(())
			})
		});

		btn_mute.on_click({
			let control = params.control.clone();
			Rc::new(move |_common, _event| {
				control.on_mute_toggle()?;
				Ok(())
			})
		});

		c.finish()?;

		Ok(())
	}

	fn init_mode_sinks(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		log::info!("refreshing sink list");

		let sinks = pactl_wrapper::list_sinks()?;
		let cards = pactl_wrapper::list_cards()?;
		let default_sink = pactl_wrapper::get_default_sink(&sinks)?;

		layout.remove_children(self.id_devices);

		for sink in sinks {
			let card = get_card_from_sink(&sink, &cards);

			let checked = if let Some(default_sink) = &default_sink {
				sink.index == default_sink.index
			} else {
				false
			};

			let alt_desc = sink.description.clone();
			let muted = sink.mute;

			let control = Rc::new(ControlSink::new(self.tasks.clone(), self.on_update.clone(), sink));

			let disp = card
				.as_ref()
				.map(|card| get_profile_display_name(&card.active_profile, card));

			self.mount_device_slider(MountDeviceSliderParams {
				checked,
				disp,
				alt_desc,
				layout,
				control,
				muted,
			})?;
		}

		Ok(())
	}

	fn init_mode_sources(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		log::info!("refreshing source list");

		let sources = pactl_wrapper::list_sources()?;
		let cards = pactl_wrapper::list_cards()?;
		let default_source = pactl_wrapper::get_default_source(&sources)?;

		layout.remove_children(self.id_devices);

		for source in sources {
			let card = get_card_from_source(&source, &cards);

			let checked = if let Some(default_source) = &default_source {
				source.index == default_source.index
			} else {
				false
			};

			let alt_desc = source.description.clone();
			let muted = source.mute;

			let control = Rc::new(ControlSource::new(self.tasks.clone(), self.on_update.clone(), source));

			let disp = card
				.as_ref()
				.map(|card| get_profile_display_name(&card.active_profile, card));

			self.mount_device_slider(MountDeviceSliderParams {
				checked,
				disp,
				alt_desc,
				layout,
				control,
				muted,
			})?;
		}

		Ok(())
	}

	fn init_mode_cards(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		log::info!("refreshing card list");

		let cards = pactl_wrapper::list_cards()?;
		layout.remove_children(self.id_devices);

		for card in cards {
			self.mount_card(MountCardParams { layout, card: &card })?;
		}

		Ok(())
	}

	fn init_mode_card_selector(&mut self, layout: &mut Layout, card: pactl_wrapper::Card) -> anyhow::Result<()> {
		log::info!("showing card selector for {}", card.name);

		layout.remove_children(self.id_devices);

		{
			let data = self.state.parse_template(
				&doc_params(&self.globals),
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
		for profile_name in card.profiles.keys() {
			if profile_name.contains("surround") {
				continue; // we aren't interested in that
			}

			let disp_name = get_profile_display_name(profile_name, &card);
			cells.push(SelectorCell {
				key: profile_name.clone(),
				display_text: disp_name.name,
				icon_path: disp_name.icon_path,
			});
		}

		let mut ess = ConstructEssentials {
			layout,
			parent: self.id_devices,
		};

		mount_multi_selector(MultiSelectorParams {
			cells: &cells,
			def_cell: &card.active_profile,
			ess: &mut ess,
			on_click: {
				let card = card.clone();
				let tasks = self.tasks.clone();
				let on_update = self.on_update.clone();
				Rc::new(move |profile_name| {
					tasks.push(ViewTask::SetCardProfile(CardAndProfileName {
						card: card.clone(),
						profile_name: profile_name.to_string(),
					}));
					tasks.push(ViewTask::SetMode(CurrentMode::Cards));
					(*on_update)();
				})
			},
		})?;

		Ok(())
	}
}
