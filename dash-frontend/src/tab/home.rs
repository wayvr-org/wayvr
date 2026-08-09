use std::{marker::PhantomData, rc::Rc};

use wgui::{
	animation::{Animation, AnimationEasing},
	assets::AssetPath,
	color::{WguiColor, WguiColorName, WguiNamedColor},
	components::{ComponentTrait, button::ComponentButton},
	event::CallbackDataCommon,
	i18n::Translation,
	layout::{LayoutTask, LayoutTasks, Widget, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	widget::label::WidgetLabel,
};
use wlx_common::config::GeneralConfig;

use crate::{
	frontend::{Frontend, FrontendTask},
	tab::{Tab, TabType},
	util::cached_fetcher,
	various,
};

pub struct TabHome<T> {
	#[allow(dead_code)]
	pub state: ParserState,
	marker: PhantomData<T>,
}

impl<T> Tab<T> for TabHome<T> {
	fn get_type(&self) -> TabType {
		TabType::Home
	}
}

fn get_supporter_anim(
	btn: Rc<ComponentButton>,
	tasks: LayoutTasks,
	supporters: Rc<cached_fetcher::Supporters>,
	prev_supporter_name: String,
) -> Animation {
	let username = loop {
		let total_tickets: u32 = supporters.supporters.iter().map(|s| s.tickets()).sum();
		let jackpot = rand::random_range(0..total_tickets);
		let mut cumulative = 0u32;
		let random_supporter = supporters.supporters.iter().find(|s| {
			cumulative += s.tickets();
			cumulative >= jackpot
		}).unwrap();
		let username = random_supporter.username.clone();
		if username != prev_supporter_name {
			break username;
		}
	};

	const FADE_SPEED: f32 = 10.0;

	Animation::new_ex(
		btn.base().get_id(),
		1234,
		480,
		AnimationEasing::Linear,
		Box::new(move |common, data| {
			let opacity_in = f32::clamp(data.pos * FADE_SPEED, 0.0, 1.0);
			let opacity_out = f32::clamp((1.0 - data.pos) * FADE_SPEED, 0.0, 1.0);
			let opacity = f32::min(opacity_in, opacity_out);

			btn.set_text_color(
				common,
				WguiColor::Named(WguiNamedColor::with_alpha(WguiColorName::OnPrimary, opacity)),
			);

			// first iter
			if data.pos == 0.0 {
				let translation_key = match rand::random::<u8>() {
					0..=15 => "DONATE.BROUGHT_TO_YOU_VARIANTS_VERY_RARE",
					16..=70 => "DONATE.BROUGHT_TO_YOU_VARIANTS_RARE",
					71..=u8::MAX => "DONATE.BROUGHT_TO_YOU_VARIANTS_OFTEN",
				};

				let translated = common.globals().i18n_builtin.translate(translation_key);
				let variants = translated.split('|').collect::<Vec<&str>>();
				let variant = variants[rand::random_range(0..variants.len() - 1)];

				btn.set_text(
					common,
					Translation::from_raw_text_string(variant.replace("{USER}", &username)),
				);
			}

			// last iter
			if data.pos == 1.0 {
				// infinitely looped animation, re-trigger it.
				tasks.push(LayoutTask::PlayAnimation(get_supporter_anim(
					btn.clone(),
					tasks.clone(),
					supporters.clone(),
					username.clone(), /* prev supporter name */
				)))
			}
		}),
	)
}

async fn config_supporters(btn: Rc<ComponentButton>, tasks: LayoutTasks) {
	let Some(supporters) = cached_fetcher::request_supporters().await else {
		return;
	};

	if supporters.supporters.len() <= 2 {
		return; // Welp.
	}

	let supporters = Rc::new(supporters);

	tasks.push(LayoutTask::PlayAnimation(get_supporter_anim(
		btn,
		tasks.clone(),
		supporters,
		String::new(),
	)));
}

fn configure_label_hello(common: &mut CallbackDataCommon, label_hello: Widget, config: &GeneralConfig) {
	let username = wlx_common::locale::capitalize_string(&various::get_username());

	let translated = if !config.hide_username {
		common.i18n().translate_and_replace("HELLO_USER", ("{USER}", &username))
	} else {
		common.i18n().translate("HELLO").to_string()
	};

	let mut label_hello = label_hello.get_as::<WidgetLabel>().unwrap();
	label_hello.set_text(common, Translation::from_raw_text(&translated));
}

impl<T> TabHome<T> {
	pub fn new(frontend: &mut Frontend<T>, parent_id: WidgetID, data: &mut T) -> anyhow::Result<Self> {
		let state = wgui::parser::parse_from_assets(
			&ParseDocumentParams {
				globals: frontend.layout.state.globals.clone(),
				path: AssetPath::BuiltIn("gui/tab/home.xml"),
				extra: Default::default(),
			},
			&mut frontend.layout,
			parent_id,
		)?;

		let widget_label = state.fetch_widget(&frontend.layout.state, "label_hello")?.widget;
		configure_label_hello(
			&mut frontend.layout.common(),
			widget_label,
			frontend.interface.general_config(data),
		);

		let btn_supporter = state.fetch_component_as::<ComponentButton>("btn_supporter")?;

		let btn_apps = state.fetch_component_as::<ComponentButton>("btn_apps")?;
		let btn_games = state.fetch_component_as::<ComponentButton>("btn_games")?;
		let btn_monado = state.fetch_component_as::<ComponentButton>("btn_monado")?;
		let btn_settings = state.fetch_component_as::<ComponentButton>("btn_settings")?;
		let btn_welcome_screen = state.fetch_component_as::<ComponentButton>("btn_welcome_screen")?;
		let btn_donate = state.fetch_component_as::<ComponentButton>("btn_donate")?;
		let btn_website = state.fetch_component_as::<ComponentButton>("btn_website")?;

		let f_tasks = &mut frontend.tasks;
		f_tasks.handle_button(&btn_apps, FrontendTask::SetTab(TabType::Apps));
		f_tasks.handle_button(&btn_games, FrontendTask::SetTab(TabType::Games));
		f_tasks.handle_button(&btn_monado, FrontendTask::SetTab(TabType::Monado));
		f_tasks.handle_button(&btn_settings, FrontendTask::SetTab(TabType::Settings));
		f_tasks.handle_button(&btn_welcome_screen, FrontendTask::SetTab(TabType::Welcome));
		f_tasks.handle_button(&btn_donate, FrontendTask::SetTab(TabType::Donate));
		f_tasks.handle_button(&btn_supporter, FrontendTask::SetTab(TabType::Donate));
		f_tasks.handle_button(&btn_website, FrontendTask::OpenURL("https://wayvr.org".into()));

		frontend
			.executor
			.spawn(config_supporters(btn_supporter, frontend.layout.tasks.clone()))
			.detach();

		Ok(Self {
			state,
			marker: PhantomData,
		})
	}
}
