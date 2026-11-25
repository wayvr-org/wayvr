use std::{
    any::Any,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use anyhow::Context;
use glam::{Affine3A, Mat4, Quat, Vec3, vec3};
use wgui::parser::Fetchable;
use wlx_common::{
    common::LeftRight,
    overlays::{ToastDisplayMethod, ToastTopic},
    windowing::{OverlayWindowState, Positioning},
};

use crate::{
    backend::task::{ModifyPanelCommand, OverlayTask, SpawnPos, TaskType},
    gui::panel::{GuiPanel, NewGuiPanelParams, apply_custom_command},
    state::AppState,
    windowing::{
        OverlaySelector, PIXELS_TO_METERS, Z_ORDER_OSD,
        window::{OverlayLifetime, OverlayWindowConfig},
    },
};

pub static OSD_NAME: LazyLock<Arc<str>> = LazyLock::new(|| "osd".into());

const TIMEOUT: f32 = 1.5;
const FADE_DURATION: f32 = 0.3;

/// on-screen display for e.g. volume, ipd. Rather than queueing like toasts, it replaces the value
/// and extends its lifetime
pub struct Osd {
    pub topic: ToastTopic,
    pub title: String,
    pub value: String,
    // 0..1, shown as a bar
    pub level: Option<f32>,
}

pub struct OsdData {
    topic: ToastTopic,
    display_method: ToastDisplayMethod,
    expiry: Instant,
}

#[derive(Default)]
pub struct OsdState {
    current: Option<OsdData>,
}

struct OsdPanel;

impl Osd {
    pub fn show(self, app: &mut AppState) {
        let display_method = app
            .session
            .toast_topics
            .get(self.topic)
            .copied()
            .unwrap_or(ToastDisplayMethod::Hide);
        if matches!(display_method, ToastDisplayMethod::Hide) {
            return;
        }

        let selector = OverlaySelector::Name(OSD_NAME.clone());
        let now = Instant::now();
        let expiry = now + Duration::from_secs_f32(TIMEOUT);

        let can_update = app.osd.current.as_ref().is_some_and(|current| {
            current.topic == self.topic
                && current.display_method == display_method
                && current.expiry > now
        });
        app.osd.current = Some(OsdData {
            topic: self.topic,
            display_method,
            expiry,
        });

        if can_update {
            app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
                selector,
                Box::new(move |app, owc| {
                    let backend: &mut dyn Any = owc.backend.as_mut();
                    let Some(panel) = backend.downcast_mut::<GuiPanel<OsdPanel>>() else {
                        return;
                    };
                    if let Err(e) = self.apply(panel, app) {
                        log::warn!("Could not update OSD: {e:?}");
                    }
                    owc.lifetime = Some(new_lifetime());
                }),
            )));
            return;
        }

        app.tasks
            .enqueue(TaskType::Overlay(OverlayTask::Drop(selector.clone())));
        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Spawn(
            selector,
            SpawnPos::Fixed,
            Box::new(move |app| {
                self.create(app, display_method)
                    .inspect_err(|e| log::error!("Could not create OSD: {e:?}"))
                    .ok()
            }),
        )));
    }

    fn get_sprite(topic: ToastTopic) -> String {
        match topic {
            ToastTopic::IpdChange => "icons/vr.svg",
            _ => "icons/bell.svg",
        }
        .to_string()
    }

    fn apply(&self, panel: &mut GuiPanel<OsdPanel>, app: &mut AppState) -> anyhow::Result<()> {
        let set_text = |text: &str| ModifyPanelCommand::SetText(text.to_string());
        apply_custom_command(
            panel,
            app,
            "sprite",
            &ModifyPanelCommand::SetImage(Self::get_sprite(self.topic)),
        )?;
        apply_custom_command(panel, app, "label_title", &set_text(&self.title))?;
        apply_custom_command(panel, app, "label_value", &set_text(&self.value))?;
        apply_custom_command(
            panel,
            app,
            "rect_track",
            &ModifyPanelCommand::SetVisible(self.level.is_some()),
        )?;

        if let Some(level) = self.level {
            let id = panel.parser_state.get_widget_id("rect_level")?;
            if let Some(widget) = panel.layout.state.widgets.get(id).cloned() {
                widget.state().data.transform =
                    Mat4::from_scale(Vec3::new(level.clamp(0.0, 1.0), 1.0, 1.0));
                panel.layout.alterables.mark_redraw();
            }
        }
        Ok(())
    }

    fn create(
        &self,
        app: &mut AppState,
        method: ToastDisplayMethod,
    ) -> anyhow::Result<OverlayWindowConfig> {
        let (spawn_point, positioning) = match method {
            ToastDisplayMethod::Hide => anyhow::bail!("hidden"),
            ToastDisplayMethod::Center => {
                (vec3(0., 0.1, -0.5), Positioning::FollowHead { lerp: 0.1 })
            }
            ToastDisplayMethod::Watch => (
                vec3(0., -0.06, 0.),
                Positioning::FollowHand {
                    hand: LeftRight::Left,
                    lerp: 0.1,
                },
            ),
        };

        let mut panel = GuiPanel::new_from_template(
            app,
            "gui/osd.xml",
            OsdPanel,
            NewGuiPanelParams::default(),
        )?;
        self.apply(&mut panel, app)?;
        panel.update_layout(app).context("layout update failed")?;

        Ok(OverlayWindowConfig {
            name: OSD_NAME.clone(),
            default_state: OverlayWindowState {
                positioning,
                transform: Affine3A::from_scale_rotation_translation(
                    Vec3::ONE * panel.layout.content_size.x * PIXELS_TO_METERS,
                    Quat::IDENTITY,
                    spawn_point,
                ),
                ..OverlayWindowState::default()
            },
            lifetime: Some(new_lifetime()),
            global: true,
            z_order: Z_ORDER_OSD,
            show_on_spawn: true,
            ..OverlayWindowConfig::from_backend(Box::new(panel))
        })
    }
}

fn new_lifetime() -> OverlayLifetime {
    OverlayLifetime::new(TIMEOUT, FADE_DURATION, 1.0, false)
}
