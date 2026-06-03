use glam::{Affine3A, Quat, Vec3};
use std::sync::{Arc, LazyLock};
use wgui::event::EventAlterables;
use wgui::parser::Fetchable;
use wlx_common::windowing::{OverlayWindowState, Positioning};

use crate::gui::panel::GuiPanel;
use crate::overlays::watch::WATCH_NAME;
use crate::state::AppState;
use crate::windowing::backend::OverlayEventData;
use crate::windowing::window::OverlayWindowConfig;
use crate::windowing::{Z_ORDER_ANCHOR, Z_ORDER_HELP};

pub static ANCHOR_NAME: LazyLock<Arc<str>> = LazyLock::new(|| Arc::from("anchor"));

pub fn create_anchor(app: &mut AppState) -> anyhow::Result<OverlayWindowConfig> {
    let mut panel = GuiPanel::new_from_template(app, "gui/anchor.xml", (), Default::default())?;
    panel.update_layout(app)?;

    Ok(OverlayWindowConfig {
        name: ANCHOR_NAME.clone(),
        z_order: Z_ORDER_ANCHOR,
        default_state: OverlayWindowState {
            interactable: false,
            grabbable: false,
            positioning: Positioning::Anchored,
            transform: Affine3A::from_scale_rotation_translation(
                Vec3::ONE * 0.1,
                Quat::IDENTITY,
                Vec3::ZERO, // Vec3::NEG_Z * 0.5,
            ),
            ..OverlayWindowState::default()
        },
        global: true,
        ..OverlayWindowConfig::from_backend(Box::new(panel))
    })
}

pub static GRAB_HELP_NAME: LazyLock<Arc<str>> = LazyLock::new(|| Arc::from("grab-help"));

pub fn create_grab_help(app: &mut AppState) -> anyhow::Result<OverlayWindowConfig> {
    let mut panel = GuiPanel::new_from_template(app, "gui/grab-help.xml", (), Default::default())?;
    panel.update_layout(app)?;

    let id_watch = panel.parser_state.data.get_widget_id("grabbing_watch")?;
    let id_static = panel.parser_state.data.get_widget_id("grabbing_static")?;
    let id_anchored = panel.parser_state.data.get_widget_id("grabbing_anchored")?;
    let id_anchored_edit = panel
        .parser_state
        .data
        .get_widget_id("grabbing_anchored_edit")?;
    let id_floating = panel.parser_state.data.get_widget_id("grabbing_floating")?;
    let id_follow = panel.parser_state.data.get_widget_id("grabbing_follow")?;

    let all = [
        id_watch,
        id_static,
        id_anchored,
        id_anchored_edit,
        id_floating,
        id_follow,
    ];

    panel.on_notify = Some(Box::new(move |panel, _app, event_data| {
        let mut alterables = EventAlterables::default();

        let OverlayEventData::OverlayGrabbed { name, pos, editing } = event_data else {
            return Ok(());
        };

        let show_id = match pos {
            Positioning::Static => id_static,
            Positioning::Floating => id_floating,
            Positioning::Anchored => {
                if editing {
                    id_anchored_edit
                } else {
                    id_anchored
                }
            }
            Positioning::FollowHand { .. } if &*name == WATCH_NAME => id_watch,
            Positioning::FollowHead { .. } | Positioning::FollowHand { .. } => id_follow,
        };

        for id in &all {
            alterables.set_widget_visible(*id, *id == show_id);
        }

        panel.layout.process_alterables(alterables)?;

        Ok(())
    }));

    Ok(OverlayWindowConfig {
        name: GRAB_HELP_NAME.clone(),
        z_order: Z_ORDER_HELP,
        default_state: OverlayWindowState {
            interactable: false,
            grabbable: false,
            transform: Affine3A::from_scale_rotation_translation(
                Vec3::ONE * 0.15,
                Quat::IDENTITY,
                Vec3::ZERO,
            ),
            ..OverlayWindowState::default()
        },
        global: true,
        ..OverlayWindowConfig::from_backend(Box::new(panel))
    })
}
