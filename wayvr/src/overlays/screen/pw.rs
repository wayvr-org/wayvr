use std::{error::Error, path::PathBuf, task, time::Instant};

use serde::{Deserialize, Serialize};
use wlx_capture::{
    WlxCapture,
    pipewire::{PipewireCapture, PipewireSelectScreenResult},
    wayland::WlxOutput,
};
use wlx_common::{
    config::{PwTokenMap, def_pw_tokens},
    config_io,
};

use crate::{state::AppState, subsystem::dbus::DbusConnector};

use super::{
    backend::ScreenBackend,
    capture::{MainThreadWlxCapture, new_wlx_capture},
};

#[cfg(feature = "wayland")]
impl ScreenBackend {
    pub fn new_pw(
        output: &WlxOutput,
        token: Option<&str>,
        app: &mut AppState,
    ) -> anyhow::Result<(Self, Option<String> /* pipewire restore token */)> {
        use crate::overlays::screen::backend::CaptureType;

        let name = output.name.clone();
        let embed_mouse = !app.session.config.double_cursor_fix;

        let select_screen_result = select_pw_screen(
            &format!(
                "Now select: {} {} {} @ {},{}",
                &output.name,
                &output.make,
                &output.model,
                &output.logical_pos.0,
                &output.logical_pos.1
            ),
            token,
            embed_mouse,
            true,
            true,
            false,
        )?;

        log::debug!(
            "{}: PipeWire result streams: {:?}",
            output.name,
            &select_screen_result.streams
        );

        let node_id = select_screen_result.streams.first().unwrap().node_id; // streams guaranteed to have at least one element

        let capture = new_wlx_capture!(
            app.gfx_extras.queue_capture,
            PipewireCapture::new(name, node_id)
        );
        Ok((
            Self::new_raw(
                output.name.clone(),
                app.xr_backend,
                CaptureType::PipeWire,
                capture,
            ),
            select_screen_result.restore_token,
        ))
    }
}

#[allow(clippy::fn_params_excessive_bools)]
pub(super) fn select_pw_screen(
    instructions: &str,
    token: Option<&str>,
    embed_mouse: bool,
    screens_only: bool,
    persist: bool,
    multiple: bool,
) -> Result<PipewireSelectScreenResult, wlx_capture::pipewire::AshpdError> {
    use std::time::Duration;
    use wlx_capture::pipewire::pipewire_select_screen;

    let future = async move {
        let print_at = Instant::now() + Duration::from_millis(250);
        let mut notify = None;

        let f = pipewire_select_screen(token, embed_mouse, screens_only, persist, multiple);
        futures::pin_mut!(f);

        loop {
            match futures::poll!(&mut f) {
                task::Poll::Ready(result) => return result,
                task::Poll::Pending => {
                    if Instant::now() >= print_at {
                        log::info!("{instructions}");
                        if let Ok(id) =
                            DbusConnector::notify_send(instructions, "", 1, 30000, 0, true)
                        {
                            notify = Some(id);
                        }
                        break;
                    }
                    futures::future::lazy(|_| {
                        std::thread::sleep(Duration::from_millis(10));
                    })
                    .await;
                }
            }
        }

        let result = f.await;
        if let Some(id) = notify {
            //safe unwrap; checked above
            let _ = DbusConnector::notify_close(id);
        }
        result
    };

    futures::executor::block_on(future)
}

#[derive(Deserialize, Serialize, Default)]
pub struct TokenConf {
    #[serde(default = "def_pw_tokens")]
    pub pw_tokens: PwTokenMap,
}

fn get_pw_token_path() -> PathBuf {
    let mut path = config_io::ConfigRoot::Generic.get_conf_d_path();
    path.push("pw_tokens.yaml");
    path
}

pub fn save_pw_token_config(tokens: PwTokenMap) -> Result<(), Box<dyn Error>> {
    let conf = TokenConf { pw_tokens: tokens };
    let yaml = serde_yaml::to_string(&conf)?;
    std::fs::write(get_pw_token_path(), yaml)?;
    Ok(())
}

pub fn load_pw_token_config() -> Result<PwTokenMap, Box<dyn Error>> {
    let yaml = std::fs::read_to_string(get_pw_token_path())?;
    let conf: TokenConf = serde_yaml::from_str(yaml.as_str())?;
    Ok(conf.pw_tokens)
}
