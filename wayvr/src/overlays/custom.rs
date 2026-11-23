use glam::{Affine3A, Quat, Vec3, vec3};
use std::{sync::Arc, time::Duration};
use wgui::{assets::AssetPathRc, globals::expand_env_vars};
use wlx_common::{
    config_io::get_gui_root,
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
    let mut params = NewGuiPanelParams {
        external_xml: true,
        ..NewGuiPanelParams::default()
    };

    let candidates = {
        let gui_folder = get_gui_root().join(name.as_ref());
        let v2_layout = gui_folder.join("panel.xml");
        [
            (Some(gui_folder), v2_layout),
            (None, get_gui_root().join(format!("{name}.xml"))),
        ]
    };

    let Some((root, path)) = candidates.iter().find_map(|(r, p)| {
        if p.is_file()
            && let Some(p_str) = p.to_str()
        {
            Some((r, p_str))
        } else {
            None
        }
    }) else {
        log::error!("No GUI definition file for custom panel {name}. Tried: {candidates:?}");
        return None;
    };

    if let Some(root) = root {
        params.external_dir = Some(AssetPathRc::File(root.as_path().into()));
    }

    let mut panel = GuiPanel::new_from_template(app, path, CustomPanelState, params)
        .inspect_err(|e| log::warn!("Error creating '{name}': {e:?}"))
        .ok()?;

    if let Some(icon) = panel.parser_state.data.var_map.get("_panel_icon") {
        let icon = if let Some(root) = root
            && let Some(prefixed) = icon.strip_prefix("@/")
        {
            let path = root.join(prefixed);
            expand_env_vars(path.to_string_lossy().as_ref())
        } else {
            expand_env_vars(icon.as_ref())
        };

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
