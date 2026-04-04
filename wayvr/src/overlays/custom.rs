use std::{sync::Arc, time::Duration};

use glam::{Affine3A, Quat, Vec3, vec3};
use wgui::globals::expand_env_vars;
use wlx_common::{
    overlays::{BackendAttrib, BackendAttribValue},
    windowing::OverlayWindowState,
};

use crate::{
    gui::{
        panel::{GuiPanel, NewGuiPanelParams, apply_custom_command},
        timer::GuiTimer,
    },
    state::AppState,
    windowing::{
        backend::OverlayEventData,
        window::{OverlayCategory, OverlayWindowConfig},
    },
};

struct CustomPanelState;

pub fn create_custom(app: &mut AppState, name: Arc<str>) -> Option<OverlayWindowConfig> {
    let params = NewGuiPanelParams {
        external_xml: true,
        ..NewGuiPanelParams::default()
    };

    let mut panel =
        GuiPanel::new_from_template(app, &format!("gui/{name}.xml"), CustomPanelState, params)
            .inspect_err(|e| log::warn!("Error creating '{name}': {e:?}"))
            .ok()?;

    if let Some(icon) = panel.parser_state.data.var_map.get("_panel_icon") {
        let icon = expand_env_vars(icon);
        panel
            .extra_attribs
            .insert(BackendAttrib::Icon, BackendAttribValue::Icon(icon.into()));
    }

    panel
        .update_layout(app)
        .inspect_err(|e| log::warn!("Error layouting '{name}': {e:?}"))
        .ok()?;

    panel
        .timers
        .push(GuiTimer::new(Duration::from_millis(100), 0));

    let scale = panel.layout.content_size.x / 40.0 * 0.05;

    panel.on_notify = Some(Box::new({
        let name = name.clone();
        move |panel, app, event_data| {
            let OverlayEventData::CustomCommand { element, command } = event_data else {
                return Ok(());
            };

            if let Err(e) = apply_custom_command(panel, app, &element, &command) {
                log::warn!("Could not apply {command:?} on {name}/{element}: {e:?}");
            }

            Ok(())
        }
    }));

    Some(OverlayWindowConfig {
        name,
        category: OverlayCategory::Panel,
        default_state: OverlayWindowState {
            interactable: true,
            grabbable: true,
            transform: Affine3A::from_scale_rotation_translation(
                Vec3::ONE * scale,
                Quat::IDENTITY,
                vec3(0.0, 0.0, -0.5),
            ),
            ..OverlayWindowState::default()
        },
        ..OverlayWindowConfig::from_backend(Box::new(panel))
    })
}
