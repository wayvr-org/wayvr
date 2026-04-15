use glam::Affine3A;
use idmap::IdMap;
use smallvec::{SmallVec, smallvec};
use std::rc::Rc;
use std::sync::Arc;
use wgui::log::LogErr;
use wgui::theme::WguiTheme;
use wgui::{
    drawing, font_config::WguiFontConfig, gfx::WGfx, globals::WguiGlobals, parser::parse_color_hex,
    renderer_vk::context::SharedContext as WSharedContext,
};
use wlx_common::async_executor::AsyncExecutor;
use wlx_common::locale::WayVRLangProvider;
use wlx_common::{
    audio,
    config::GeneralConfig,
    config_io::{self, get_config_file_path},
    desktop_finder::DesktopFinder,
    overlays::{ToastDisplayMethod, ToastTopic},
};

#[cfg(feature = "openxr")]
use crate::backend;
use crate::backend::wayvr::WvrServerState;

#[cfg(feature = "osc")]
use crate::subsystem::osc::OscSender;

use crate::{
    backend::{XrBackend, input::InputState, task::TaskContainer},
    config::load_general_config,
    graphics::WGfxExtras,
    gui,
    ipc::{event_queue::SyncEventQueue, ipc_server, signal::WayVRSignal},
    subsystem::{dbus::DbusConnector, input::HidWrapper},
};

pub struct AppState {
    pub session: AppSession,
    pub tasks: TaskContainer,
    pub executor: AsyncExecutor,

    pub gfx: Arc<WGfx>,
    pub gfx_extras: WGfxExtras,
    pub hid_provider: HidWrapper,

    pub audio_system: audio::AudioSystem,
    pub audio_sample_player: audio::SamplePlayer,

    pub wgui_shared: WSharedContext,

    pub input_state: InputState,
    pub screens: SmallVec<[ScreenMeta; 8]>,
    pub anchor: Affine3A,
    pub anchor_grabbed: bool,

    pub wgui_globals: WguiGlobals,
    pub wgui_theme: Rc<WguiTheme>,

    pub dbus: DbusConnector,

    pub xr_backend: XrBackend,

    pub ipc_server: ipc_server::WayVRServer,
    pub wayvr_signals: SyncEventQueue<WayVRSignal>,

    pub desktop_finder: DesktopFinder,

    #[cfg(feature = "osc")]
    pub osc_sender: Option<OscSender>,

    pub wvr_server: Option<WvrServerState>,

    #[cfg(feature = "openxr")]
    pub monado_state: Option<backend::openxr::monado_state::MonadoState>,
}

#[allow(unused_mut)]
impl AppState {
    pub fn from_graphics(
        gfx: Arc<WGfx>,
        gfx_extras: WGfxExtras,
        xr_backend: XrBackend,
    ) -> anyhow::Result<Self> {
        // insert shared resources
        let mut tasks = TaskContainer::new();

        let session = AppSession::load();
        let wvr_signals = SyncEventQueue::new();

        let wvr_server = WvrServerState::new(gfx.clone(), &gfx_extras, wvr_signals.clone())
            .log_err("Could not initialize WayVR Server")
            .ok();

        let (hid_provider, mut hid_error) = HidWrapper::new();

        #[cfg(feature = "osc")]
        let osc_sender = crate::subsystem::osc::OscSender::new(session.config.osc_out_port).ok();

        let wgui_shared = WSharedContext::new(gfx.clone())?;
        let theme_path = session.config.theme_path.clone();

        let mut audio_sample_player = audio::SamplePlayer::new();
        audio_sample_player.register_sample(
            "key_click",
            audio::AudioSample::from_mp3(&audio::AudioSample::bytes_from_config_or_default(
                "sound/key_click.mp3",
                include_bytes!("res/key_click.mp3"),
            ))?,
        )?;

        audio_sample_player.register_sample(
            "toast",
            audio::AudioSample::from_mp3(&audio::AudioSample::bytes_from_config_or_default(
                "sound/toast.mp3",
                include_bytes!("res/toast.mp3"),
            ))?,
        )?;

        audio_sample_player.register_sample(
            "fix_floor",
            audio::AudioSample::from_mp3(&audio::AudioSample::bytes_from_config_or_default(
                "sound/fix_floor.mp3",
                include_bytes!("res/fix_floor.mp3"),
            ))?,
        )?;

        let mut assets = Box::new(gui::asset::GuiAsset {});
        audio_sample_player.register_wgui_samples(assets.as_mut())?;

        let mut theme = WguiTheme::default();

        {
            #[allow(clippy::ref_option)]
            fn apply_color(default: &mut drawing::Color, value: &Option<String>) {
                if let Some(parsed) = value.as_ref().and_then(|c| parse_color_hex(c)) {
                    *default = parsed;
                }
            }

            apply_color(&mut theme.text_color, &session.config.color_text);
            apply_color(&mut theme.accent_color, &session.config.color_accent);
            apply_color(&mut theme.danger_color, &session.config.color_danger);
            apply_color(&mut theme.faded_color, &session.config.color_faded);
            apply_color(&mut theme.bg_color, &session.config.color_background);
        }

        theme.animation_mult = 1. / session.config.ui_animation_speed;
        theme.rounding_mult = session.config.ui_round_multiplier;

        let dbus = DbusConnector::default();

        let ipc_server = ipc_server::WayVRServer::new()?;

        let mut desktop_finder = DesktopFinder::new();
        desktop_finder.refresh();

        let lang_provider = WayVRLangProvider::from_config(&session.config);

        let executor = Rc::new(smol::LocalExecutor::new());

        let mut app_state = Self {
            session,
            tasks,
            executor,
            gfx,
            gfx_extras,
            hid_provider,
            audio_system: audio::AudioSystem::new(),
            audio_sample_player,
            wgui_shared,
            input_state: InputState::new(),
            screens: smallvec![],
            anchor: Affine3A::IDENTITY,
            anchor_grabbed: false,
            wgui_globals: WguiGlobals::new(
                assets,
                &lang_provider,
                &WguiFontConfig::default(),
                get_config_file_path(&theme_path),
            )?,
            wgui_theme: Rc::new(theme),
            dbus,
            xr_backend,
            ipc_server,
            wayvr_signals: wvr_signals,
            desktop_finder,

            #[cfg(feature = "osc")]
            osc_sender,

            wvr_server,

            #[cfg(feature = "openxr")]
            monado_state: None,
        };

        if let Some(error_toast) = hid_error {
            error_toast.submit(&mut app_state);
        }
        Ok(app_state)
    }

    #[cfg(feature = "openxr")]
    pub fn monado_state_init(&mut self) {
        use crate::backend::openxr::monado_state::MonadoState;

        log::debug!("Connecting to Monado IPC");
        self.monado_state = None; // stop connection first

        match MonadoState::new() {
            Ok(m) => {
                self.monado_state = Some(m);
            }
            Err(e) => {
                log::error!("Will not use libmonado: {e:?}");
            }
        }
    }
}

pub struct AppSession {
    pub config: GeneralConfig,
    pub config_dirty: bool,

    pub toast_topics: IdMap<ToastTopic, ToastDisplayMethod>,
}

impl AppSession {
    pub fn load() -> Self {
        let config_root_path = config_io::ConfigRoot::Generic.ensure_dir();
        log::info!("Config root path: {}", config_root_path.display());
        let config = load_general_config();

        let mut toast_topics = IdMap::new();
        toast_topics.insert(ToastTopic::System, ToastDisplayMethod::Center);
        toast_topics.insert(ToastTopic::Error, ToastDisplayMethod::Center);
        toast_topics.insert(ToastTopic::DesktopNotification, ToastDisplayMethod::Center);
        toast_topics.insert(ToastTopic::XSNotification, ToastDisplayMethod::Center);

        config.notification_topics.iter().for_each(|(k, v)| {
            toast_topics.insert(*k, *v);
        });

        Self {
            config,
            toast_topics,
            config_dirty: false,
        }
    }
}

pub struct ScreenMeta {
    pub name: Arc<str>,
    #[allow(dead_code)]
    pub native_handle: u32,
}
