use glam::{DVec2, vec2};
use wgui::log::LogErr;
use wlx_capture::{
    WlxCapture,
    frame::Transform,
    wayland::{WlxClient, WlxOutput},
    wlr_screencopy::WlrScreencopyCapture,
};
use wlx_common::{astr_containers::AStrMapExt, config::CaptureMethod};

use crate::{
    overlays::screen::{backend::CaptureType, create_screen_from_backend, pw::ScreenCastBackend},
    state::{AppState, ScreenMeta},
    windowing::backend::OverlayBackend,
};

use super::{
    ScreenCreateData,
    backend::ScreenBackend,
    capture::{MainThreadWlxCapture, new_wlx_capture},
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

pub fn create_screen_renderer_wl(
    output: &WlxOutput,
    has_wlr_screencopy: bool,
    app: &mut AppState,
) -> anyhow::Result<Box<dyn OverlayBackend>> {
    if matches!(
        app.session.config.capture_method,
        CaptureMethod::ScreenCopyCpu | CaptureMethod::ScreenCopyGpu | CaptureMethod::Auto
    ) && has_wlr_screencopy
    {
        if let Some(mut backend) = ScreenBackend::new_wlr_screencopy(output, app) {
            log::info!("{}: Using ScreenCopy capture", &output.name);
            backend.logical_pos = vec2(output.logical_pos.0 as f32, output.logical_pos.1 as f32);
            backend.logical_size = vec2(output.logical_size.0 as f32, output.logical_size.1 as f32);
            backend.apply_mouse_transform_with_override(Transform::Undefined);
            return Ok(Box::new(backend));
        }
    }

    log::info!("{}: Using Pipewire capture", &output.name);
    let display_name = &*output.name;

    // Find existing token by display
    let token = app
        .session
        .pw_tokens
        .arc_get(display_name)
        .map(|x| x.to_string().into());

    if token.is_some() {
        log::info!("Found existing Pipewire token for display {display_name}");
    }

    Ok(Box::new(
        ScreenCastBackend::new_wl(output, token, app)
            .log_err("Failed to create screen with screen cast backend")?,
    ))
}

pub fn create_screens_wayland(
    wl: &mut WlxClient,
    app: &mut AppState,
) -> anyhow::Result<ScreenCreateData> {
    let mut screens = vec![];
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

        let backend = create_screen_renderer_wl(output, has_wlr_screencopy, app)?;
        let window_config = create_screen_from_backend(
            output.name.clone(),
            output.transform,
            &app.session,
            backend,
        );

        let meta = ScreenMeta {
            name: wl.outputs[id].name.clone(),
            native_handle: *id,
        };

        screens.push((meta, window_config));
    }

    let extent = wl.get_desktop_extent();
    let origin = wl.get_desktop_origin();

    app.hid_provider
        .inner
        .set_desktop_extent(DVec2::new(extent.0 as _, extent.1 as _));
    app.hid_provider
        .inner
        .set_desktop_origin(DVec2::new(origin.0 as _, origin.1 as _));

    Ok(ScreenCreateData { screens })
}
