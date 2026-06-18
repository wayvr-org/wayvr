use glam::vec2;
use wlx_capture::{
    WlxCapture,
    frame::Transform,
    wayland::{WlxClient, WlxOutput},
    wlr_screencopy::WlrScreencopyCapture,
};
use wlx_common::{
    astr_containers::AStrMapExt,
    config::{CaptureMethod, PwTokenMap},
};

use crate::{
    overlays::screen::{backend::CaptureType, create_screen_from_backend},
    state::{AppState, ScreenMeta},
};

use super::{
    ScreenCreateData,
    backend::ScreenBackend,
    capture::{MainThreadWlxCapture, new_wlx_capture},
    pw::{load_pw_token_config, save_pw_token_config},
};

impl ScreenBackend {
    pub fn new_wlr_screencopy(output: &WlxOutput, app: &AppState) -> Option<Self> {
        let client = WlxClient::new()?;
        let capture = new_wlx_capture!(
            app.gfx_extras.queue_capture,
            WlrScreencopyCapture::new(client, output.id)
        );
        Some(Self::new_raw(
            output.name.clone(),
            app.xr_backend,
            CaptureType::ScreenCopy,
            capture,
        ))
    }
}

#[allow(clippy::useless_let_if_seq)]
pub fn create_screen_renderer_wl(
    output: &WlxOutput,
    has_wlr_screencopy: bool,
    pw_token_store: &mut PwTokenMap,
    app: &mut AppState,
) -> Option<ScreenBackend> {
    let mut capture: Option<ScreenBackend> = None;

    if matches!(
        app.session.config.capture_method,
        CaptureMethod::ScreenCopyCpu | CaptureMethod::ScreenCopyGpu | CaptureMethod::Auto
    ) && has_wlr_screencopy
    {
        log::info!("{}: Using ScreenCopy capture", &output.name);
        capture = ScreenBackend::new_wlr_screencopy(output, app);
    }

    if capture.is_none() {
        log::info!("{}: Using Pipewire capture", &output.name);

        let display_name = &*output.name;

        // Find existing token by display
        let token = pw_token_store
            .arc_get(display_name)
            .map(std::string::String::as_str);

        if let Some(t) = token {
            log::info!("Found existing Pipewire token for display {display_name}: {t}");
        }

        match ScreenBackend::new_pw(output, token, app) {
            Ok((renderer, restore_token)) => {
                capture = Some(renderer);

                if let Some(token) = restore_token
                    && pw_token_store.arc_set(display_name.into(), token.clone())
                {
                    log::info!("Adding Pipewire token {token}");
                }
            }
            Err(e) => {
                log::warn!(
                    "{}: Failed to create Pipewire capture: {:?}",
                    &output.name,
                    e
                );
            }
        }
    }
    capture
}

pub fn create_screens_wayland(wl: &mut WlxClient, app: &mut AppState) -> ScreenCreateData {
    let mut screens = vec![];

    // Load existing Pipewire tokens from file
    let mut pw_tokens: PwTokenMap = load_pw_token_config().unwrap_or_default();

    let pw_tokens_copy = pw_tokens.clone();
    let has_wlr_screencopy = wl.maybe_wlr_screencopy_mgr.is_some();

    for (id, output) in &wl.outputs {
        if app.screens.iter().any(|s| s.name == output.name) {
            continue;
        }

        log::info!(
            "{}: Init screen of res {:?}, logical {:?} at {:?}",
            output.name,
            output.size,
            output.logical_size,
            output.logical_pos,
        );

        if let Some(mut backend) =
            create_screen_renderer_wl(output, has_wlr_screencopy, &mut pw_tokens, app)
        {
            backend.logical_pos = vec2(output.logical_pos.0 as f32, output.logical_pos.1 as f32);
            backend.logical_size = vec2(output.logical_size.0 as f32, output.logical_size.1 as f32);
            // The captured frame's orientation is corrected at render time (texture UV remap),
            // so pointer hits land in the upright/logical space of the output. The compositor's
            // logical coordinates are upright too, hence the mouse maps without rotation.
            backend.mouse_transform_original = Transform::Normal;
            backend.apply_mouse_transform_with_override(Transform::Undefined);

            let window_config = create_screen_from_backend(
                output.name.clone(),
                output.transform,
                &app.session,
                Box::new(backend),
            );

            let meta = ScreenMeta {
                name: wl.outputs[id].name.clone(),
                native_handle: *id,
            };

            screens.push((meta, window_config));
        }
    }

    if pw_tokens_copy != pw_tokens {
        // Token list changed, re-create token config file
        if let Err(err) = save_pw_token_config(pw_tokens) {
            log::error!("Failed to save Pipewire token config: {err}");
        }
    }

    let extent = wl.get_desktop_extent();
    let origin = wl.get_desktop_origin();

    app.hid_provider
        .inner
        .set_desktop_extent(vec2(extent.0 as f32, extent.1 as f32));
    app.hid_provider
        .inner
        .set_desktop_origin(vec2(origin.0 as f32, origin.1 as f32));

    ScreenCreateData { screens }
}
