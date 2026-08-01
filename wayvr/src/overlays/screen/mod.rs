use std::{f32::consts::PI, sync::Arc};

use glam::{Affine3A, Quat, Vec3, vec3};
use wlx_capture::frame::Transform;
use wlx_common::windowing::OverlayWindowState;

use crate::{
    config::none_if_0,
    state::{AppSession, AppState, ScreenMeta},
    subsystem::input::InputFocus,
    windowing::{
        backend::OverlayBackend,
        window::{OverlayCategory, OverlayWindowConfig},
    },
};
use wlx_common::DesktopBackend;

pub mod backend;
pub mod capture;
#[cfg(feature = "wayland")]
pub mod mirror;
#[cfg(feature = "pipewire")]
pub mod pw;
#[cfg(feature = "wayland")]
pub mod wl;
#[cfg(feature = "x11")]
pub mod x11;

fn create_screen_from_backend(
    name: Arc<str>,
    transform: Transform,
    session: &AppSession,
    backend: Box<dyn OverlayBackend>,
) -> OverlayWindowConfig {
    let angle = if session.config.upright_screen_fix {
        match transform {
            Transform::Rotated90 | Transform::Flipped90 => PI / 2.,
            Transform::Rotated180 | Transform::Flipped180 => PI,
            Transform::Rotated270 | Transform::Flipped270 => -PI / 2.,
            _ => 0.,
        }
    } else {
        0.
    };

    OverlayWindowConfig {
        name,
        category: OverlayCategory::Screen,
        default_state: OverlayWindowState {
            grabbable: true,
            positioning: session.config.default_positioning.into(),
            curvature: none_if_0(session.config.default_curvature),
            alpha: session.config.default_opacity,
            interactable: true,
            transform: Affine3A::from_scale_rotation_translation(
                Vec3::ONE * 1.5 * session.config.default_overlay_scale,
                Quat::from_rotation_z(angle),
                vec3(0.0, 0.0, -0.5),
            ),
            ..OverlayWindowState::default()
        },
        input_focus: Some(InputFocus::PhysicalScreen),
        ..OverlayWindowConfig::from_backend(backend)
    }
}

pub struct ScreenCreateData {
    pub screens: Vec<(ScreenMeta, OverlayWindowConfig)>,
}

pub fn create_screens(app: &mut AppState) -> anyhow::Result<(ScreenCreateData, DesktopBackend)> {
    app.screens.clear();

    #[cfg(feature = "wayland")]
    {
        if let Some(mut wl) = wlx_capture::wayland::WlxClient::new() {
            log::info!("Wayland detected.");
            return Ok((
                wl::create_screens_wayland(&mut wl, app)?,
                DesktopBackend::Wayland,
            ));
        }
        log::info!("Wayland not detected, assuming X11.");
    }

    #[cfg(feature = "x11")]
    {
        #[cfg(feature = "pipewire")]
        match x11::create_screens_x11pw(app) {
            Ok(data) => return Ok((data, DesktopBackend::X11)),
            Err(e) => log::info!("Will not use X11 PipeWire capture: {e:?}"),
        }

        return Ok((x11::create_screens_xshm(app)?, DesktopBackend::X11));
    }
    #[cfg(not(feature = "x11"))]
    anyhow::bail!("No backends left to try.")
}
