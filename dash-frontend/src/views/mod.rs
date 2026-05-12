use wlx_common::{async_executor::AsyncExecutor, config::GeneralConfig, dash_interface::ConfigChangeKind};

pub mod app_launcher;
pub mod audio_settings;
pub mod dialog_box;
pub mod download_file;
pub mod game_cover;
pub mod game_launcher;
pub mod game_list;
pub mod remote_skymap_downloader;
pub mod remote_skymap_list;
pub mod running_games_list;
pub mod skymap_list;
pub mod skymap_list_cell;

pub struct ViewUpdateParams<'a> {
	pub layout: &'a mut wgui::layout::Layout,
	pub executor: &'a AsyncExecutor,
	pub general_config: &'a mut GeneralConfig,
	pub config_change_kind: &'a mut Option<ConfigChangeKind>,
}

pub trait ViewTrait {
	fn update(&mut self, par: &mut ViewUpdateParams) -> anyhow::Result<()>;
}
