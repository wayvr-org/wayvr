use std::marker::PhantomData;

use wgui::{
	assets::AssetPath,
	layout::WidgetID,
	parser::{Fetchable, ParseDocumentParams, ParserState},
};

use crate::{
	frontend::Frontend,
	tab::{Tab, TabType},
	util::steam_utils::SteamUtils,
	views::{ViewTrait, ViewUpdateParams, game_list, running_games_list},
};

pub struct TabGames<T> {
	#[allow(dead_code)]
	pub state: ParserState,

	view_game_list: game_list::View,
	view_running_games_list: running_games_list::View,
	marker: PhantomData<T>,
}

impl<T> Tab<T> for TabGames<T> {
	fn get_type(&self) -> TabType {
		TabType::Games
	}

	fn update(&mut self, frontend: &mut Frontend<T>, time_ms: u32, data: &mut T) -> anyhow::Result<()> {
		let mut config_change_kind = None;

		self.view_game_list.update(&mut ViewUpdateParams {
			layout: &mut frontend.layout,
			executor: &frontend.executor,
			general_config: frontend.interface.general_config(data),
			config_change_kind: &mut config_change_kind,
		})?;

		if let Some(kind) = config_change_kind {
			frontend.interface.config_changed(data, kind);
		}

		self.view_running_games_list.update(&mut frontend.layout, time_ms)?;
		Ok(())
	}
}

impl<T> TabGames<T> {
	pub fn new(frontend: &mut Frontend<T>, parent_id: WidgetID) -> anyhow::Result<Self> {
		let globals = frontend.layout.state.globals.clone();

		let state = wgui::parser::parse_from_assets(
			&ParseDocumentParams {
				globals: globals.clone(),
				path: AssetPath::BuiltIn("gui/tab/games.xml"),
				extra: Default::default(),
			},
			&mut frontend.layout,
			parent_id,
		)?;

		let game_list_parent = state.get_widget_id("game_list_parent")?;
		let id_running_games_list_parent = state.get_widget_id("running_games_list_parent")?;

		let mut steam_utils = SteamUtils::new()?;

		let view_game_list = game_list::View::new(game_list::Params {
			executor: frontend.executor.clone(),
			frontend_tasks: frontend.tasks.clone(),
			globals: globals.clone(),
			layout: &mut frontend.layout,
			parent_id: game_list_parent,
			steam_utils: &steam_utils,
		})?;

		let view_running_games_list = running_games_list::View::new(running_games_list::Params {
			globals: globals.clone(),
			layout: &mut frontend.layout,
			parent_id: id_running_games_list_parent,
			steam_utils: &mut steam_utils,
			frontend_tasks: frontend.tasks.clone(),
		})?;

		Ok(Self {
			state,
			view_game_list,
			view_running_games_list,
			marker: PhantomData,
		})
	}
}
