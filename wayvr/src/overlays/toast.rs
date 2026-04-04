use std::{
    ops::Add,
    sync::{Arc, LazyLock},
    time::Instant,
};

use anyhow::Context;
use glam::{Affine3A, Quat, Vec3, vec3};
use wgui::{i18n::Translation, widget::label::WidgetLabel};
use wlx_common::{
    common::LeftRight,
    overlays::{ToastDisplayMethod, ToastTopic},
    windowing::{OverlayWindowState, Positioning},
};

use crate::{
    backend::task::{OverlayTask, TaskType},
    gui::panel::{GuiPanel, NewGuiPanelParams, OnCustomIdFunc},
    state::AppState,
    windowing::{OverlaySelector, Z_ORDER_TOAST, window::OverlayWindowConfig},
};

const PIXELS_TO_METERS: f32 = 1. / 2000.;
static TOAST_NAME: LazyLock<Arc<str>> = LazyLock::new(|| "toast".into());

pub struct Toast {
    pub title: String,
    pub body: String,
    pub opacity: f32,
    pub timeout: f32,
    pub lerp_amount: f32,
    pub sound: bool,
    pub topic: ToastTopic,
}

#[allow(dead_code)]
impl Toast {
    pub const fn new(topic: ToastTopic, title: String, body: String) -> Self {
        Self {
            title,
            body,
            opacity: 1.0,
            lerp_amount: 0.1,
            timeout: 3.0,
            sound: false,
            topic,
        }
    }
    pub const fn with_timeout(mut self, timeout: f32) -> Self {
        self.timeout = timeout;
        self
    }
    pub const fn with_lerp_amount(mut self, lerp: f32) -> Self {
        self.lerp_amount = lerp;
        self
    }
    pub const fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }
    pub const fn with_sound(mut self, sound: bool) -> Self {
        self.sound = sound;
        self
    }
    pub fn submit(self, app: &mut AppState) {
        self.submit_at(app, Instant::now());
    }
    pub fn submit_at(self, app: &mut AppState, instant: Instant) {
        let selector = OverlaySelector::Name(TOAST_NAME.clone());

        let destroy_at = instant.add(std::time::Duration::from_secs_f32(self.timeout));

        if self.sound && app.session.config.notifications_sound_enabled {
            app.audio_sample_player
                .play_sample(&mut app.audio_system, "toast");
        }

        // drop any toast that was created before us.
        // (DropOverlay only drops overlays that were
        // created before current frame)
        app.tasks.enqueue_at(
            TaskType::Overlay(OverlayTask::Drop(selector.clone())),
            instant,
        );

        // CreateOverlay only creates the overlay if
        // the selector doesn't exist yet, so in case
        // multiple toasts are submitted for the same
        // frame, only the first one gets created
        app.tasks.enqueue_at(
            TaskType::Overlay(OverlayTask::Create(
                selector.clone(),
                Box::new(move |app| {
                    let maybe_toast = new_toast(self, app);
                    app.tasks.enqueue_at(
                        // at timeout, drop the overlay by ID instead
                        // in order to avoid dropping any newer toasts
                        TaskType::Overlay(OverlayTask::Drop(selector)),
                        destroy_at,
                    );
                    maybe_toast
                }),
            )),
            instant,
        );
    }
}

fn new_toast(toast: Toast, app: &mut AppState) -> Option<OverlayWindowConfig> {
    let current_method = app
        .session
        .toast_topics
        .get(toast.topic)
        .copied()
        .unwrap_or(ToastDisplayMethod::Hide);

    let (spawn_point, spawn_rotation, positioning) = match current_method {
        ToastDisplayMethod::Hide => {
            log::debug!("Not showing toast: filtered out");
            return None;
        }
        ToastDisplayMethod::Center => (
            vec3(0., -0.2, -0.5),
            Quat::IDENTITY,
            Positioning::FollowHead {
                lerp: toast.lerp_amount,
            },
        ),
        ToastDisplayMethod::Watch => {
            let relative_to = Positioning::FollowHand {
                hand: LeftRight::Left,
                lerp: 0.1,
                align_to_hmd: true,
            };
            (vec3(0., 0., 0.), Quat::IDENTITY, relative_to)
        }
    };

    let title = if toast.title.is_empty() {
        Translation::from_translation_key("TOAST.DEFAULT_TITLE")
    } else if matches!(toast.topic, ToastTopic::System | ToastTopic::Error) {
        Translation::from_translation_key(&toast.title)
    } else {
        Translation::from_raw_text(&toast.title)
    };

    let body = if matches!(toast.topic, ToastTopic::System) {
        Translation::from_translation_key(&toast.body)
    } else {
        Translation::from_raw_text(&toast.body)
    };

    let on_custom_id: OnCustomIdFunc<()> =
        Box::new(move |id, widget, _doc_params, layout, _parser_state, ()| {
            if &*id == "toast_title" {
                let mut label = layout
                    .state
                    .widgets
                    .get_as::<WidgetLabel>(widget)
                    .context("toast.xml: missing element with id: toast_title")?;
                let mut globals = layout.state.globals.get();
                label.set_text_simple(&mut globals, title.clone());
            }
            if &*id == "toast_body" {
                let mut label = layout
                    .state
                    .widgets
                    .get_as::<WidgetLabel>(widget)
                    .context("toast.xml: missing element with id: toast_body")?;
                let mut globals = layout.state.globals.get();
                label.set_text_simple(&mut globals, body.clone());
            }
            Ok(())
        });

    let mut panel = GuiPanel::new_from_template(
        app,
        "gui/toast.xml",
        (),
        NewGuiPanelParams {
            on_custom_id: Some(on_custom_id),
            ..Default::default()
        },
    )
    .inspect_err(|e| log::error!("Could not create toast: {e:?}"))
    .ok()?;

    panel
        .update_layout(app)
        .context("layout update failed")
        .ok()?;

    Some(OverlayWindowConfig {
        name: TOAST_NAME.clone(),
        default_state: OverlayWindowState {
            positioning,
            transform: Affine3A::from_scale_rotation_translation(
                Vec3::ONE * panel.layout.content_size.x * PIXELS_TO_METERS,
                spawn_rotation,
                spawn_point,
            ),
            ..OverlayWindowState::default()
        },
        global: true,
        z_order: Z_ORDER_TOAST,
        show_on_spawn: true,
        ..OverlayWindowConfig::from_backend(Box::new(panel))
    })
}

// FIXME: Will these functions be used in the future? Can they be removed?

#[allow(dead_code)]
fn msg_err(app: &mut AppState, message: &str) {
    Toast::new(ToastTopic::Error, "TOAST.ERROR".into(), message.into())
        .with_timeout(3.)
        .submit(app);
}

// Display the same error in the terminal and as a toast in VR.
// Formatted as "Failed to XYZ: Object is not defined"
#[allow(dead_code)]
pub fn error_toast<ErrorType>(app: &mut AppState, title: &str, err: ErrorType)
where
    ErrorType: std::fmt::Display + std::fmt::Debug,
{
    log::error!("{title}: {err:?}"); // More detailed version (use Debug)

    // Brief version (use Display)
    msg_err(app, &format!("{title}: {err}"));
}

#[allow(dead_code)]
pub fn error_toast_str(app: &mut AppState, message: &str) {
    log::error!("{message}");
    msg_err(app, message);
}
