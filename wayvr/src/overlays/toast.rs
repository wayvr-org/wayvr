use std::{
    ops::Add,
    sync::{Arc, LazyLock},
    time::Instant,
};

use anyhow::Context;
use glam::{Affine3A, Quat, Vec2, Vec3, vec3};
use wgui::{
    animation::{Animation, AnimationDuration, AnimationEasing, AnimationStartDelay},
    i18n::{I18n, Translation},
    parser::Fetchable,
    widget::label::WidgetLabel,
};
use wlx_common::{
    common::LeftRight,
    overlays::{ToastDisplayMethod, ToastTopic},
    windowing::{OverlayWindowState, Positioning},
};

use crate::{
    backend::task::{OverlayTask, SpawnPos, TaskType},
    gui::panel::{GuiPanel, NewGuiPanelParams, OnCustomIdFunc},
    state::AppState,
    windowing::{OverlaySelector, PIXELS_TO_METERS, Z_ORDER_TOAST, window::OverlayWindowConfig},
};

static TOAST_NAME: LazyLock<Arc<str>> = LazyLock::new(|| "toast".into());

#[derive(Clone)]
pub struct ToastParams {
    pub opacity: f32,
    pub timeout: f32,
    pub lerp_amount: f32,
    pub animate: bool,
    pub sound: bool,
    pub topic: ToastTopic,
}

pub struct Toast {
    pub title: Option<Translation>,
    pub body: Translation,
    pub params: ToastParams,
}

pub struct BakedToast {
    title_raw: String, // ready-to-display text
    body_raw: String,  // ready-to-display text
    params: ToastParams,
}

#[allow(dead_code)]
impl Toast {
    pub const fn new(topic: ToastTopic, title: Option<Translation>, body: Translation) -> Self {
        Self {
            title,
            body,
            params: ToastParams {
                opacity: 1.0,
                lerp_amount: 0.1,
                timeout: 3.0,
                animate: true,
                sound: false,
                topic,
            },
        }
    }

    pub const fn with_timeout(mut self, timeout: f32) -> Self {
        self.params.timeout = timeout;
        self
    }

    pub const fn with_lerp_amount(mut self, lerp: f32) -> Self {
        self.params.lerp_amount = lerp;
        self
    }

    pub const fn with_opacity(mut self, opacity: f32) -> Self {
        self.params.opacity = opacity;
        self
    }

    pub const fn with_sound(mut self, sound: bool) -> Self {
        self.params.sound = sound;
        self
    }

    pub const fn with_animate(mut self, animate: bool) -> Self {
        self.params.animate = animate;
        self
    }

    pub fn submit(self, app: &mut AppState) {
        self.submit_at(app, Instant::now());
    }

    pub fn submit_at(self, app: &mut AppState, instant: Instant) {
        let globals = app.wgui_globals.clone();
        let baked_toast = self.build(&mut globals.i18n());
        baked_toast.submit_at(app, instant);
    }

    // bake it
    pub fn build(&self, lang: &mut I18n) -> BakedToast {
        let title = if let Some(title) = &self.title {
            title.clone()
        } else {
            Translation::from_translation_key("TOAST.DEFAULT_TITLE")
        };

        BakedToast {
            title_raw: String::from(title.generate(lang).as_ref()),
            body_raw: String::from(self.body.generate(lang).as_ref()),
            params: self.params.clone(),
        }
    }

    pub fn build_raw(&self) -> BakedToast {
        BakedToast {
            title_raw: String::from(if let Some(title) = &self.title {
                title.text.as_ref()
            } else {
                ""
            }),
            body_raw: String::from(self.body.text.as_ref()),
            params: self.params.clone(),
        }
    }
}

impl BakedToast {
    pub fn submit(self, app: &mut AppState) {
        self.submit_at(app, Instant::now());
    }
    pub fn submit_at(self, app: &mut AppState, instant: Instant) {
        let selector = OverlaySelector::Name(TOAST_NAME.clone());

        let destroy_at = instant.add(std::time::Duration::from_secs_f32(self.params.timeout));

        if self.params.sound && app.session.config.notifications_sound_enabled {
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
            TaskType::Overlay(OverlayTask::Spawn(
                selector.clone(),
                SpawnPos::Fixed,
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

fn new_toast(toast: BakedToast, app: &mut AppState) -> Option<OverlayWindowConfig> {
    let current_method = app
        .session
        .toast_topics
        .get(toast.params.topic)
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
                lerp: toast.params.lerp_amount,
            },
        ),
        ToastDisplayMethod::Watch => {
            let relative_to = Positioning::FollowHand {
                hand: LeftRight::Left,
                lerp: 0.1,
            };
            (vec3(0., 0., 0.), Quat::IDENTITY, relative_to)
        }
    };

    let title = Translation::from_raw_text(&toast.title_raw);
    let body = Translation::from_raw_text(&toast.body_raw);

    let on_custom_id: OnCustomIdFunc<()> =
        Box::new(move |id, widget, _doc_params, layout, _parser_state, ()| {
            if &*id == "label_title" {
                let mut label = layout
                    .state
                    .widgets
                    .get_as::<WidgetLabel>(widget)
                    .context("toast.xml: missing element with id: label_title")?;
                let mut globals = layout.state.globals.get();
                label.set_text_simple(&mut globals, title.clone());
            }
            if &*id == "label_body" {
                let mut label = layout
                    .state
                    .widgets
                    .get_as::<WidgetLabel>(widget)
                    .context("toast.xml: missing element with id: label_body")?;
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

    // animations
    if toast.params.animate {
        let id_div_title = panel.parser_state.get_widget_id("div_title").ok()?;
        let id_label_body = panel.parser_state.get_widget_id("label_body").ok()?;
        let id_label_title = panel.parser_state.get_widget_id("label_title").ok()?;
        let id_rect = panel.parser_state.get_widget_id("rect").ok()?;
        let id_rect_separator = panel.parser_state.get_widget_id("rect_separator").ok()?;
        let id_sprite_bell = panel.parser_state.get_widget_id("sprite_bell").ok()?;

        // schedule timeouts
        let time_window = 0.5;
        if toast.params.timeout > time_window && toast.params.timeout < 150.0 {
            let dur = AnimationDuration::SecondsFixed(time_window);
            let easing = AnimationEasing::OutQuad;

            let mut add = |a: Animation| {
                a.reversed()
                    .delayed(AnimationStartDelay::SecondsFixed(
                        toast.params.timeout - time_window,
                    ))
                    .with_id(1)
                    .submit_l(&mut panel.layout);
            };

            add(Animation::effect_rectangle_fade_in(id_rect, dur, easing));
            add(Animation::effect_scale(
                id_rect_separator,
                dur,
                easing,
                Vec2::new(0.0, 1.0),
                Vec2::new(1.0, 1.0),
            ));
            add(Animation::effect_label_fade_in(id_label_title, dur, easing));
            add(Animation::effect_label_fade_in(id_label_body, dur, easing));
            add(Animation::effect_sprite_fade_in(
                id_sprite_bell,
                dur,
                easing,
            ));
        }

        // separator scale
        Animation::effect_scale(
            id_rect_separator,
            AnimationDuration::Seconds(1.0),
            AnimationEasing::OutBack,
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
        )
        .submit_l(&mut panel.layout);

        // rectangle opacity
        Animation::effect_rectangle_fade_in(
            id_rect,
            AnimationDuration::Seconds(0.5),
            AnimationEasing::OutQuad,
        )
        .submit_l(&mut panel.layout);

        // title animation
        Animation::effect_slide(
            id_div_title,
            AnimationDuration::Seconds(1.0),
            AnimationEasing::OutQuint,
            Vec2::new(15.0, 0.0),
        )
        .submit_l(&mut panel.layout);

        Animation::effect_label_fade_in(
            id_label_title,
            AnimationDuration::Seconds(0.5),
            AnimationEasing::OutQuad,
        )
        .submit_l(&mut panel.layout);

        Animation::effect_sprite_fade_in(
            id_sprite_bell,
            AnimationDuration::Seconds(0.5),
            AnimationEasing::OutQuad,
        )
        .submit_l(&mut panel.layout);

        // body animation
        Animation::effect_slide(
            id_label_body,
            AnimationDuration::Seconds(1.0),
            AnimationEasing::OutQuint,
            Vec2::new(15.0, 0.0),
        )
        .delayed(AnimationStartDelay::Seconds(0.3))
        .submit_l(&mut panel.layout);

        Animation::effect_label_fade_in(
            id_label_body,
            AnimationDuration::Seconds(0.5),
            AnimationEasing::OutQuint,
        )
        .delayed(AnimationStartDelay::Seconds(0.3))
        .submit_l(&mut panel.layout);
    }

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
