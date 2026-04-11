use std::{collections::HashMap, rc::Rc};

use wgui::{
	assets::AssetPath,
	components::button::ComponentButton,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	task::Tasks,
	widget::{
		ConstructEssentials,
		label::{WidgetLabel, WidgetLabelParams},
	},
};
use wlx_common::async_executor::AsyncExecutor;

use crate::{
	frontend::FrontendTasks,
	util::{
		cached_fetcher::CoverArt,
		popup_manager::PopupHolder,
		steam_utils::{self, AppID, SteamUtils},
	},
	views::{self, game_cover},
};

#[derive(Clone)]
enum Task {
	AppManifestClicked(steam_utils::AppManifest),
	SetCoverArt(AppID, Rc<CoverArt>),
	LoadManifests,
	FillPage(u32),
	PrevPage,
	NextPage,
}

pub struct Params<'a> {
	pub globals: WguiGlobals,
	pub executor: AsyncExecutor,
	pub frontend_tasks: FrontendTasks,
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
}

const MAX_GAMES_PER_PAGE: u32 = 30;

pub struct GameCoverCell {
	view_cover: game_cover::View,
}

pub struct View {
	#[allow(dead_code)]
	parser_state: ParserState,
	tasks: Tasks<Task>,
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	id_list_parent: WidgetID,
	game_cover_view_common: game_cover::ViewCommon,
	executor: AsyncExecutor,
	mounted_game_covers: HashMap<AppID, GameCoverCell>,
	all_manifests: Vec<steam_utils::AppManifest>,
	cur_page: u32,
	page_count: u32,
	id_label_page: WidgetID,
	view_launcher: PopupHolder<views::game_launcher::View>,
}

impl View {
	pub fn new(params: Params) -> anyhow::Result<Self> {
		let doc_params = &ParseDocumentParams {
			globals: params.globals.clone(),
			path: AssetPath::BuiltIn("gui/view/game_list.xml"),
			extra: Default::default(),
		};

		let parser_state = wgui::parser::parse_from_assets(doc_params, params.layout, params.parent_id)?;
		let list_parent = parser_state.fetch_widget(&params.layout.state, "list_parent")?;
		let id_label_page = parser_state.get_widget_id("label_page")?;

		let tasks = Tasks::new();

		tasks.handle_button(
			&parser_state.fetch_component_as::<ComponentButton>("btn_prev")?,
			Task::PrevPage,
		);

		tasks.handle_button(
			&parser_state.fetch_component_as::<ComponentButton>("btn_next")?,
			Task::NextPage,
		);

		tasks.push(Task::LoadManifests);

		Ok(Self {
			parser_state,
			tasks,
			frontend_tasks: params.frontend_tasks,
			globals: params.globals.clone(),
			id_list_parent: list_parent.id,
			mounted_game_covers: HashMap::new(),
			game_cover_view_common: game_cover::ViewCommon::new(params.globals.clone()),
			executor: params.executor,
			all_manifests: Vec::new(),
			cur_page: 0,
			page_count: 0,
			id_label_page,
			view_launcher: Default::default(),
		})
	}

	pub fn update(
		&mut self,
		layout: &mut Layout,
		steam_utils: &mut SteamUtils,
		executor: &AsyncExecutor,
	) -> anyhow::Result<()> {
		loop {
			let tasks = self.tasks.drain();
			if tasks.is_empty() {
				break;
			}
			for task in tasks {
				match task {
					Task::LoadManifests => self.load_manifests(steam_utils),
					Task::FillPage(page_idx) => self.fill_page(layout, executor, page_idx)?,
					Task::AppManifestClicked(manifest) => self.action_app_manifest_clicked(manifest)?,
					Task::SetCoverArt(app_id, cover_art) => self.set_cover_art(layout, app_id, cover_art),
					Task::PrevPage => self.page_prev(),
					Task::NextPage => self.page_next(),
				}
			}
		}

		self.view_launcher.with_view_res(|view| view.update(layout))?;

		Ok(())
	}
}

fn fill_game_list(
	ess: &mut ConstructEssentials,
	executor: &AsyncExecutor,
	mounted_game_covers: &mut HashMap<AppID, GameCoverCell>,
	manifests: &[steam_utils::AppManifest],
	tasks: &Tasks<Task>,
) -> anyhow::Result<()> {
	for manifest in manifests {
		let on_loaded = {
			let app_id = manifest.app_id.clone();
			let tasks = tasks.clone();
			Box::new(move |cover_art: CoverArt| {
				tasks.push(Task::SetCoverArt(app_id, Rc::from(cover_art)));
			})
		};

		let view_cover = game_cover::View::new(game_cover::Params {
			ess,
			executor,
			manifest,
			on_loaded,
			scale: 1.0,
		})?;

		view_cover.button.on_click({
			let tasks = tasks.clone();
			let manifest = manifest.clone();
			Rc::new(move |_, _| {
				tasks.push(Task::AppManifestClicked(manifest.clone()));
				Ok(())
			})
		});

		mounted_game_covers.insert(manifest.app_id.clone(), GameCoverCell { view_cover });
	}

	Ok(())
}

impl View {
	fn load_manifests(&mut self, steam_utils: &mut SteamUtils) {
		match steam_utils.list_installed_games(steam_utils::GameSortMethod::PlayDateDesc) {
			Ok(manifests) => {
				self.page_count = (manifests.len() as u32 + MAX_GAMES_PER_PAGE) / MAX_GAMES_PER_PAGE;
				self.all_manifests = manifests;
				self.tasks.push(Task::FillPage(0));
			}
			Err(e) => {
				log::error!("Failed to list installed games: {e:?}");
			}
		}
	}

	fn page_prev(&mut self) {
		if self.cur_page == 0 {
			return;
		}

		self.cur_page -= 1;
		self.tasks.push(Task::FillPage(self.cur_page));
	}

	fn page_next(&mut self) {
		if self.cur_page >= self.page_count - 1 {
			return;
		}
		self.cur_page += 1;
		self.tasks.push(Task::FillPage(self.cur_page));
	}

	fn fill_page(&mut self, layout: &mut Layout, executor: &AsyncExecutor, page_idx: u32) -> anyhow::Result<()> {
		layout.remove_children(self.id_list_parent);
		self.mounted_game_covers.clear();

		let idx_from = (page_idx * MAX_GAMES_PER_PAGE).min(self.all_manifests.len() as u32);
		let idx_to = ((page_idx + 1) * MAX_GAMES_PER_PAGE).min(self.all_manifests.len() as u32);

		let page_manifests = &self.all_manifests[idx_from as usize..idx_to as usize];

		let mut text: Option<Translation> = None;

		if page_manifests.is_empty() {
			text = Some(Translation::from_translation_key("GAME_LIST.NO_GAMES_FOUND"))
		}

		// set page text
		{
			let mut c = layout.common();
			let mut widget = c.state.widgets.cast_as::<WidgetLabel>(self.id_label_page)?;
			widget.set_text(
				&mut c,
				Translation::from_raw_text_string(format!("{}/{}", self.cur_page + 1, self.page_count)),
			);
		}

		fill_game_list(
			&mut ConstructEssentials {
				layout,
				parent: self.id_list_parent,
			},
			executor,
			&mut self.mounted_game_covers,
			page_manifests,
			&self.tasks,
		)?;

		if let Some(text) = text.take() {
			let label = WidgetLabel::create(
				&mut layout.state,
				WidgetLabelParams {
					content: text,
					..Default::default()
				},
			);
			layout.add_child(self.id_list_parent, label, Default::default())?;
		}

		Ok(())
	}

	fn set_cover_art(&mut self, layout: &mut Layout, app_id: AppID, cover_art: Rc<CoverArt>) {
		let Some(cover) = &mut self.mounted_game_covers.get_mut(&app_id) else {
			return;
		};

		if let Err(e) = cover
			.view_cover
			.set_cover_art(&mut self.game_cover_view_common, layout, &cover_art)
		{
			log::error!("{:?}", e);
		};
	}

	fn action_app_manifest_clicked(&mut self, manifest: steam_utils::AppManifest) -> anyhow::Result<()> {
		views::game_launcher::mount_popup(
			self.frontend_tasks.clone(),
			self.executor.clone(),
			self.globals.clone(),
			manifest,
			self.view_launcher.clone(),
		);

		Ok(())
	}
}
