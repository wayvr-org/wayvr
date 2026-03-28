use wgui::{
	assets::AssetPath,
	components::button::ComponentButton,
	event::StyleSetRequest,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, LayoutTask, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	taffy::Display,
	task::Tasks,
	widget::label::WidgetLabel,
};

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::steam_utils::{self, AppID, AppManifest, GameSortMethod, SteamUtils},
};

#[derive(Clone)]
enum Task {
	Refresh,
	StopGame(AppID, bool /* kill */),
}

pub struct Params<'a> {
	pub globals: WguiGlobals,
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub steam_utils: &'a mut SteamUtils,
	pub frontend_tasks: FrontendTasks,
}

pub struct View {
	#[allow(dead_code)]
	state: ParserState,
	tasks: Tasks<Task>,
	last_update_ms: u32,
	id_list_parent: WidgetID,
	installed_games: Vec<AppManifest>,
	frontend_tasks: FrontendTasks,
	parent_id: WidgetID,
}

fn doc_params(globals: WguiGlobals) -> ParseDocumentParams<'static> {
	ParseDocumentParams {
		globals,
		path: AssetPath::BuiltIn("gui/view/running_games_list.xml"),
		extra: Default::default(),
	}
}

impl View {
	pub fn new(params: Params) -> anyhow::Result<Self> {
		let state = wgui::parser::parse_from_assets(&doc_params(params.globals.clone()), params.layout, params.parent_id)?;
		let btn_refresh = state.fetch_component_as::<ComponentButton>("btn_refresh")?;
		let id_list_parent = state.get_widget_id("list_parent")?;

		let installed_games = params
			.steam_utils
			.list_installed_games(GameSortMethod::None)
			.unwrap_or_default();

		let tasks = Tasks::<Task>::new();

		tasks.handle_button(&btn_refresh, Task::Refresh);
		tasks.push(Task::Refresh);

		Ok(Self {
			state,
			tasks,
			last_update_ms: 0,
			id_list_parent,
			installed_games,
			frontend_tasks: params.frontend_tasks,
			parent_id: params.parent_id,
		})
	}

	pub fn update(&mut self, layout: &mut Layout, time_ms: u32) -> anyhow::Result<()> {
		loop {
			let tasks = self.tasks.drain();
			if tasks.is_empty() {
				break;
			}

			for task in tasks {
				match task {
					Task::Refresh => self.refresh(layout)?,
					Task::StopGame(app_id, kill) => self.stop_game(app_id, kill),
				}
			}
		}

		if self.last_update_ms + 5000 < time_ms {
			self.last_update_ms = time_ms;
			self.tasks.push(Task::Refresh);
		}

		Ok(())
	}

	fn extract_name_from_appid(app_id: &AppID, manifests: &[AppManifest]) -> String {
		for manifest in manifests {
			if manifest.app_id == *app_id {
				return manifest.name.clone();
			}
		}

		format!("Unknown AppID {}", app_id)
	}

	fn fill_list(&mut self, layout: &mut Layout, games: Vec<steam_utils::RunningGame>) -> anyhow::Result<()> {
		if games.is_empty() {
			// hide self
			layout.tasks.push(LayoutTask::SetWidgetStyle(
				self.parent_id,
				StyleSetRequest::Display(Display::None),
			));
			return Ok(());
		}

		layout.tasks.push(LayoutTask::SetWidgetStyle(
			self.parent_id,
			StyleSetRequest::Display(Display::DEFAULT),
		));

		for game in games {
			let game_name = View::extract_name_from_appid(&game.app_id, &self.installed_games);

			let t = self.state.realize_template(
				&doc_params(layout.state.globals.clone()),
				"RunningGameCell",
				layout,
				self.id_list_parent,
				Default::default(),
			)?;

			let mut label_name = t.fetch_widget_as::<WidgetLabel>(&layout.state, "label_name")?;

			self.tasks.handle_button(
				&t.fetch_component_as::<ComponentButton>("btn_stop")?,
				Task::StopGame(game.app_id.clone(), false),
			);

			self.tasks.handle_button(
				&t.fetch_component_as::<ComponentButton>("btn_kill")?,
				Task::StopGame(game.app_id, true),
			);

			label_name.set_text_simple(
				&mut layout.state.globals.get(),
				Translation::from_raw_text_string(game_name),
			);
		}

		Ok(())
	}

	fn refresh(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		log::debug!("refreshing running games list");

		layout.remove_children(self.id_list_parent);

		match steam_utils::list_running_games() {
			Ok(games) => self.fill_list(layout, games)?,
			Err(e) => {
				log::error!("failed to list games: {}", e);
			}
		}

		Ok(())
	}

	fn stop_game(&mut self, app_id: AppID, kill: bool) {
		if let Err(e) = steam_utils::stop(app_id, kill) {
			self
				.frontend_tasks
				.push(FrontendTask::PushToast(Translation::from_raw_text_string(format!(
					"Error: {}",
					e
				))));
		}
	}
}
