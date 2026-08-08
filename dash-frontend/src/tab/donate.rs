use std::{marker::PhantomData, rc::Rc};

use wgui::{
	assets::AssetPath,
	components::button::ComponentButton,
	globals::WguiGlobals,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState, TemplateParams},
	taffy::{self, style_helpers::length},
	task::Tasks,
	widget::div::WidgetDiv,
};

use crate::{
	frontend::{Frontend, FrontendTask},
	tab::{Tab, TabType},
	util::cached_fetcher,
};

#[allow(clippy::enum_variant_names)]
enum Task {
	SetSupporters(cached_fetcher::Supporters),
}

pub struct TabDonate<T> {
	#[allow(dead_code)]
	pub state: ParserState,
	marker: PhantomData<T>,
	#[allow(dead_code)]
	tasks: Tasks<Task>,

	id_current_supporters: WidgetID,
}

impl<T> Tab<T> for TabDonate<T> {
	fn get_type(&self) -> TabType {
		TabType::Donate
	}

	fn update(&mut self, frontend: &mut Frontend<T>, _time_ms: u32, _user_data: &mut T) -> anyhow::Result<()> {
		while !self.tasks.is_empty() {
			for task in self.tasks.drain() {
				match task {
					Task::SetSupporters(supporters) => self.set_supporters(&mut frontend.layout, supporters)?,
				}
			}
		}

		Ok(())
	}
}

fn doc_params(globals: &WguiGlobals) -> ParseDocumentParams<'_> {
	ParseDocumentParams {
		globals: globals.clone(),
		path: AssetPath::BuiltIn("gui/tab/donate.xml"),
		extra: Default::default(),
	}
}

async fn request_supporters(tasks: Tasks<Task>) {
	if let Some(supporters) = cached_fetcher::request_supporters().await {
		tasks.push(Task::SetSupporters(supporters))
	}
}

fn tier_pretty_print(tier: &str) -> String {
	format!("{} Tier", wlx_common::locale::capitalize_string(tier))
}

fn tier_color(tier: &str) -> &'static str {
	match tier {
		"platinum" => "#aaffff",
		"gold" => "#ffffaa",
		"silver" => "#cccccc",
		"bronze" => "#ffaa66",
		_ => "on_background",
	}
}

impl<T> TabDonate<T> {
	fn set_supporters(&mut self, layout: &mut Layout, supporters: cached_fetcher::Supporters) -> anyhow::Result<()> {
		let globals = layout.state.globals.clone();
		layout.remove_children(self.id_current_supporters);

		let mut current_tier = "";
		let mut tier_parent = WidgetID::default();

		for supporter in &supporters.supporters {
			if supporter.tier != current_tier {
				current_tier = &supporter.tier;

				let mut params = TemplateParams::new();
				params.insert_str("text", tier_pretty_print(&supporter.tier));
				params.insert("color", tier_color(&supporter.tier));
				self.state.realize_template(
					&doc_params(&globals),
					"TierCell",
					layout,
					self.id_current_supporters,
					params,
				)?;

				tier_parent = layout
					.add_child(
						self.id_current_supporters,
						WidgetDiv::create(),
						taffy::Style {
							gap: length(8.0_f32),
							flex_wrap: taffy::FlexWrap::Wrap,
							..Default::default()
						},
					)?
					.0
					.id;
			}

			let mut params = TemplateParams::new();
			params.insert("username", &supporter.username);
			self
				.state
				.realize_template(&doc_params(&globals), "SupporterCell", layout, tier_parent, params)?;
		}

		Ok(())
	}

	pub fn new(frontend: &mut Frontend<T>, parent_id: WidgetID, _data: &mut T) -> anyhow::Result<Self> {
		let state = wgui::parser::parse_from_assets(&doc_params(&frontend.globals), &mut frontend.layout, parent_id)?;
		let id_current_supporters = state.get_widget_id("current_supporters")?;

		frontend.tasks.handle_button(
			&state.fetch_component_as::<ComponentButton>("btn_donate")?,
			FrontendTask::OpenURL(Rc::from("https://opencollective.com/wayvr-org")),
		);

		let tasks = Tasks::<Task>::new();

		frontend.executor.spawn(request_supporters(tasks.clone())).detach();

		Ok(Self {
			state,
			marker: PhantomData,
			tasks,
			id_current_supporters,
		})
	}
}
