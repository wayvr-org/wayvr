use std::{
    sync::{Arc, LazyLock},
    time::Instant,
};

use anyhow::Context;
use glam::{Affine3A, Mat4, Quat, Vec2, Vec3, vec3};
use wgui::{
    animation::{Animation, AnimationDuration, AnimationEasing, AnimationStartDelay},
    color::WguiColorName,
    i18n::{I18n, Translation},
    parser::Fetchable,
    widget::{label::WidgetLabel, rectangle::WidgetRectangle},
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
    windowing::{
        OverlaySelector, PIXELS_TO_METERS, Z_ORDER_TOAST,
        window::{OverlayLifetime, OverlayWindowConfig},
    },
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

    pub fn extend_display_time(app: &mut AppState, seconds: f32) {
        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
            OverlaySelector::Name(TOAST_NAME.clone()),
            Box::new(move |_app, overlay| overlay.extend_lifetime(seconds)),
        )));
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

        if self.params.sound && app.session.config.notifications_sound_enabled {
            app.audio.play_sample("toast");
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
                selector,
                SpawnPos::Fixed,
                Box::new(move |app| new_toast(self, app)),
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

    let fade_duration = if toast.params.animate
        && toast.params.timeout > 0.5
        && toast.params.timeout < 150.0
    {
        0.5
    } else {
        0.0
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

    let id_rect = panel.parser_state.get_widget_id("rect").ok()?;
    let id_rect_separator = panel.parser_state.get_widget_id("rect_separator").ok()?;
    let base_border_color = panel
        .layout
        .state
        .widgets
        .get_as::<WidgetRectangle>(id_rect)?
        .get_border_color();
    let paused_border_color = WguiColorName::Tertiary.to_wgui_color();
    let mut last_progress = 1.0;
    let mut paused_border_mix = 0.0f32;

    panel.on_lifetime_update = Some(Box::new(move |panel, update| {
        let progress = update.progress();
        if (progress - last_progress).abs() > f32::EPSILON {
            if let Some(widget) = panel.layout.state.widgets.get(id_rect_separator).cloned() {
                widget.state().data.transform =
                    Mat4::from_scale(Vec3::new(progress, 1.0, 1.0));
                panel.layout.alterables.mark_redraw();
            }
            last_progress = progress;
        }

        let target_mix = if update.paused { 1.0 } else { 0.0 };
        let fade_duration = (0.25 * panel.layout.state.theme.animation_mult).max(0.001);
        let step = update.elapsed / fade_duration;
        let new_mix = if target_mix > paused_border_mix {
            (paused_border_mix + step).min(target_mix)
        } else {
            (paused_border_mix - step).max(target_mix)
        };

        if (new_mix - paused_border_mix).abs() > f32::EPSILON {
            let border_color = if new_mix <= 0.0 {
                base_border_color
            } else if new_mix >= 1.0 {
                paused_border_color
            } else {
                let globals = panel.layout.state.globals.get();
                base_border_color.lerp(&globals.palette, &paused_border_color, new_mix)
            };

            if let Some(widget) = panel.layout.state.widgets.get(id_rect).cloned()
                && let Some(mut rect) = widget.get_as::<WidgetRectangle>()
            {
                let mut common = panel.layout.common();
                rect.set_border_color(&mut common, border_color);
            }
            paused_border_mix = new_mix;
        }
    }));

    // animations
    if toast.params.animate {
        let id_div_title = panel.parser_state.get_widget_id("div_title").ok()?;
        let id_label_body = panel.parser_state.get_widget_id("label_body").ok()?;
        let id_label_title = panel.parser_state.get_widget_id("label_title").ok()?;
        let id_sprite_bell = panel.parser_state.get_widget_id("sprite_bell").ok()?;

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
            alpha: toast.params.opacity,
            transform: Affine3A::from_scale_rotation_translation(
                Vec3::ONE * panel.layout.content_size.x * PIXELS_TO_METERS,
                spawn_rotation,
                spawn_point,
            ),
            ..OverlayWindowState::default()
        },
        lifetime: Some(OverlayLifetime::new(
            toast.params.timeout,
            fade_duration,
            toast.params.opacity,
            true,
        )),
        global: true,
        z_order: Z_ORDER_TOAST,
        show_on_spawn: true,
        ..OverlayWindowConfig::from_backend(Box::new(panel))
    })
}
