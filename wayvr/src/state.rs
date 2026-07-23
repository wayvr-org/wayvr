use glam::Affine3A;
use idmap::IdMap;
use serde::{Deserialize, Serialize};
use smallvec::{SmallVec, smallvec};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use wgui::log::LogErr;
use wgui::theme::WguiTheme;
use wgui::{
    font_config::WguiFontConfig, gfx::WGfx, globals::WguiGlobals,
    renderer_vk::context::SharedContext as WSharedContext,
};
#[cfg(feature = "pipewire")]
use wlx_capture::pipewire::ScreenCastManager;
use wlx_common::config::PwTokenMap;
use wlx_common::locale::WayVRLangProvider;
use wlx_common::palette::load_palette;
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

use crate::subsystem::notifications::NotificationManager;
#[cfg(feature = "osc")]
use crate::subsystem::osc::OscSender;

#[cfg(feature = "whisper")]
use crate::subsystem::whisper_stt::WhisperStt;
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

    pub gfx: Arc<WGfx>,
    pub gfx_extras: WGfxExtras,
    pub hid_provider: HidWrapper,

    pub audio_system: audio::AudioSystem,
    pub audio_sample_player: audio::SamplePlayer,

    pub notifications: NotificationManager,

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

    #[cfg(feature = "whisper")]
    pub whisper_sst: Option<WhisperStt>,

    #[cfg(feature = "openxr")]
    pub monado_state: Option<backend::openxr::monado_state::MonadoState>,

    #[cfg(feature = "pipewire")]
    pub screencast_manager: Option<ScreenCastManager>,

    pub delta_time: f32,
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

        let wvr_server = {
            let mut maybe_wvr = WvrServerState::new(gfx.clone(), &gfx_extras, wvr_signals.clone())
                .log_err("Could not initialize WayVR Server")
                .ok();
            if let Some(wvr) = maybe_wvr.as_mut() {
                wvr.config_changed(&session.config);
            }
            maybe_wvr
        };

        let (hid_provider, mut hid_error) = HidWrapper::new(session.config.input_emulation_method);

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

        audio_sample_player.register_sample(
            "input_grab",
            audio::AudioSample::from_mp3(&audio::AudioSample::bytes_from_config_or_default(
                "sound/wvr_input_capture_grabbed.mp3",
                include_bytes!("assets/sound/wvr_input_capture_grabbed.mp3"),
            ))?,
        )?;

        audio_sample_player.register_sample(
            "input_ungrab",
            audio::AudioSample::from_mp3(&audio::AudioSample::bytes_from_config_or_default(
                "sound/wvr_input_capture_ungrabbed.mp3",
                include_bytes!("assets/sound/wvr_input_capture_ungrabbed.mp3"),
            ))?,
        )?;

        let mut assets = Box::new(gui::asset::GuiAsset {});
        audio_sample_player.register_wgui_samples(assets.as_mut())?;

        let mut theme = WguiTheme::default();

        theme.animation_mult = 1. / session.config.ui_animation_speed;
        theme.rounding_mult = session.config.ui_round_multiplier;

        let dbus = DbusConnector::default();

        let ipc_server = ipc_server::WayVRServer::new()?;

        let mut desktop_finder = DesktopFinder::new();
        desktop_finder.refresh();

        let lang_provider = WayVRLangProvider::from_config(&session.config);

        #[cfg(feature = "pipewire")]
        let screencast_manager = ScreenCastManager::new()
            .log_err(
                // would only fail if session D-bus is unreachable
                "Could not initialize ScreenCastManager. PipeWire screen capture will not work. Check your D-bus setup.",
            )
            .ok();

        let palette = load_palette(&*session.config.color_palette);

        let mut app_state = Self {
            tasks,
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
                palette,
            )?,
            wgui_theme: Rc::new(theme),
            dbus,
            xr_backend,
            ipc_server,
            wayvr_signals: wvr_signals,
            desktop_finder,
            notifications: NotificationManager::new(),

            #[cfg(feature = "osc")]
            osc_sender,

            wvr_server,
            #[cfg(feature = "whisper")]
            whisper_sst: None,

            #[cfg(feature = "openxr")]
            monado_state: None,

            #[cfg(feature = "pipewire")]
            screencast_manager,

            delta_time: 1.0 / 120.0,
            session,
        };

        if let Some(error_toast) = hid_error {
            error_toast.submit(&mut app_state);
        }
        Ok(app_state)
    }

    pub fn late_init(&mut self) {
        self.notifications.run_dbus(&mut self.dbus);
        self.notifications.run_udp();

        #[cfg(feature = "openxr")]
        if matches!(self.xr_backend, XrBackend::OpenXR) {
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

    pub fn tick(&mut self) {
        self.dbus.tick();

        for toast in self.notifications.drain_pending(&self.session) {
            toast.submit(self);
        }

        #[cfg(feature = "whisper")]
        {
            if self.whisper_sst.as_ref().is_some_and(|x| x.should_unload()) {
                log::info!("Unloading Whisper model due to timeout");
                self.whisper_sst = None;
            }
        }
    }
}

pub struct AppSession {
    pub config: GeneralConfig,
    pub config_dirty: bool,

    #[cfg(feature = "pipewire")]
    pub pw_tokens: PwTokenMap,

    pub no_autostart: bool,

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

        #[cfg(feature = "pipewire")]
        let pw_tokens = load_pw_token_config()
            .log_err("Could not load PipeWire tokens")
            .unwrap_or_default();

        Self {
            config,
            toast_topics,
            no_autostart: false,
            config_dirty: false,
            #[cfg(feature = "pipewire")]
            pw_tokens,
        }
    }
}

pub struct ScreenMeta {
    pub name: Arc<str>,
    #[allow(dead_code)]
    pub native_handle: u32,
}

#[cfg(feature = "pipewire")]
#[derive(Deserialize, Serialize, Default)]
struct TokenConf {
    pub pw_tokens: PwTokenMap,
}

#[cfg(feature = "pipewire")]
fn get_pw_token_path() -> PathBuf {
    let mut path = config_io::ConfigRoot::Generic.get_conf_d_path();
    path.push("pw_tokens.yaml");
    path
}

#[cfg(feature = "pipewire")]
pub fn save_pw_token_config(tokens: PwTokenMap) -> anyhow::Result<()> {
    let conf = TokenConf { pw_tokens: tokens };
    let yaml = serde_yaml::to_string(&conf)?;
    std::fs::write(get_pw_token_path(), yaml)?;
    Ok(())
}

#[cfg(feature = "pipewire")]
pub fn load_pw_token_config() -> anyhow::Result<PwTokenMap> {
    let yaml = std::fs::read_to_string(get_pw_token_path())?;
    let conf: TokenConf = serde_yaml::from_str(yaml.as_str())?;
    Ok(conf.pw_tokens)
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct PlayspaceState {
    pub openvr_space_center: Affine3A,
    pub openxr_space_center: Affine3A,
}

pub fn load_playspace_state() -> anyhow::Result<PlayspaceState> {
    let json = std::fs::read_to_string(config_io::get_config_file_path("playspace.json5"))?;
    let state: PlayspaceState = serde_json5::from_str(json.as_str())?;
    Ok(state)
}

pub fn save_playspace_state(state: &PlayspaceState) -> anyhow::Result<()> {
    let json = serde_json5::to_string(state)?;
    std::fs::write(config_io::get_config_file_path("playspace.json5"), json)?;
    Ok(())
}
