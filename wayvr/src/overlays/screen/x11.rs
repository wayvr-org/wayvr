use std::sync::Arc;

use wgui::log::LogErr;

use glam::{DVec2, vec2};
use wlx_capture::{
    WlxCapture,
    frame::Transform,
    xshm::{XshmCapture, XshmScreen},
};
use wlx_common::{astr_containers::AStrMapExt, config::CaptureMethod};

use crate::{
    overlays::screen::{backend::CaptureType, create_screen_from_backend},
    state::{AppState, ScreenMeta},
};

use super::{
    ScreenCreateData,
    backend::ScreenBackend,
    capture::{MainThreadWlxCapture, new_wlx_capture},
};

#[cfg(feature = "pipewire")]
use crate::{overlays::screen::pw::ScreenCastBackend, windowing::backend::OverlayBackend};

impl ScreenBackend {
    pub fn new_xshm(screen: Arc<XshmScreen>, app: &AppState) -> Self {
        let capture = new_wlx_capture!(
            app.gfx_extras.queue_capture,
            XshmCapture::new(screen.clone())
        );
        Self::new_raw(
            screen.name.clone(),
            app.feats.xr_backend,
            CaptureType::Xshm,
            capture,
        )
    }
}

#[cfg(feature = "pipewire")]
pub fn create_screen_renderer_x11pw(
    output: &XshmScreen,
    app: &mut AppState,
) -> anyhow::Result<Box<dyn OverlayBackend>> {
    let display_name = &*output.name;

    // Find existing token by display
    let token = app
        .session
        .pw_tokens
        .arc_get(display_name)
        .map(|x| x.clone().into());

    if token.is_some() {
        log::info!("Found existing Pipewire token for display {display_name}");
    }

    Ok(Box::new(
        ScreenCastBackend::new_x11pw(output, token, app)
            .log_err("Failed to create screen with screen cast backend")?,
    ))
}

#[cfg(feature = "pipewire")]
pub fn create_screens_x11pw(app: &mut AppState) -> anyhow::Result<ScreenCreateData> {
    use wlx_capture::xshm::xshm_get_monitors;

    use crate::{overlays::screen::create_screen_from_backend, state::ScreenMeta};

    use super::ScreenCreateData;

    if !matches!(
        app.session.config.capture_method,
        CaptureMethod::PipeWireCpu | CaptureMethod::PipeWire
    ) {
        anyhow::bail!("Pipewire is not selected as backend");
    }

    let monitors = match xshm_get_monitors() {
        Ok(m) => m,
        Err(e) => {
            anyhow::bail!(e.to_string());
        }
    };

    let mut extent = DVec2::ZERO;
    let mut screens = vec![];
    for m in &monitors {
        if app.screens.iter().any(|s| s.name == m.name) {
            continue;
        }

        log::info!(
            "{}: Init screen of res {}x{}, at {}x{}",
            m.name,
            m.monitor.width(),
            m.monitor.height(),
            m.monitor.x(),
            m.monitor.y(),
        );

        extent.x = extent.x.max((m.monitor.x() + m.monitor.width()) as _);
        extent.y = extent.y.max((m.monitor.y() + m.monitor.height()) as _);

        let backend = create_screen_renderer_x11pw(m, app)?;
        let window_config =
            create_screen_from_backend(m.name.clone(), Transform::Normal, &app.session, backend);

        let meta = ScreenMeta {
            name: m.name.clone(),
            native_handle: 0,
        };

        screens.push((meta, window_config));
    }

    log::info!("Got {} monitors", monitors.len());

    app.hid_provider.inner.set_desktop_extent(extent);
    app.hid_provider.inner.set_desktop_origin(DVec2::ZERO);

    Ok(ScreenCreateData { screens })
}

pub fn create_screens_xshm(app: &mut AppState) -> anyhow::Result<ScreenCreateData> {
    use wlx_capture::xshm::xshm_get_monitors;

    let mut extent = DVec2::ZERO;

    let monitors = match xshm_get_monitors() {
        Ok(m) => m,
        Err(e) => {
            anyhow::bail!(e.to_string());
        }
    };

    let screens = monitors
        .into_iter()
        .map(|s| {
            extent.x = extent.x.max((s.monitor.x() + s.monitor.width()) as _);
            extent.y = extent.y.max((s.monitor.y() + s.monitor.height()) as _);

            let size = (s.monitor.width(), s.monitor.height());
            let pos = (s.monitor.x(), s.monitor.y());
            let mut backend = ScreenBackend::new_xshm(s.clone(), app);

            log::info!(
                "{}: Init X11 screen of res {:?} at {:?}",
                s.name.clone(),
                size,
                pos,
            );

            backend.logical_pos = vec2(s.monitor.x() as f32, s.monitor.y() as f32);
            backend.logical_size = vec2(size.0 as f32, size.1 as f32);
            backend.apply_mouse_transform_with_override(Transform::Undefined);

            let window_data = create_screen_from_backend(
                s.name.clone(),
                Transform::Normal,
                &app.session,
                Box::new(backend),
            );

            let meta = ScreenMeta {
                name: s.name.clone(),
                native_handle: 0,
            };

            (meta, window_data)
        })
        .collect();

    app.hid_provider.inner.set_desktop_extent(extent);
    app.hid_provider.inner.set_desktop_origin(DVec2::ZERO);

    Ok(ScreenCreateData { screens })
}
