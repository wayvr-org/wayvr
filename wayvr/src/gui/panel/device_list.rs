use std::collections::HashMap;

use slotmap::Key;
use wgui::{
    layout::Layout,
    parser::{Fetchable, ParseDocumentParams, ParserState},
};

use crate::{
    backend::input::TrackedDeviceRole, state::AppState, windowing::backend::OverlayEventData,
};

#[derive(Default)]
/// Helper for managing a list of overlays
/// Populates `id="devices_root"` with `<Hmd>`, `<LeftHand>`, `<RightHand>`, `<Tracker>` templates
pub struct DeviceList;

impl DeviceList {
    #[allow(clippy::unused_self)]
    pub fn on_notify(
        &mut self,
        app: &AppState,
        layout: &mut Layout,
        parser_state: &mut ParserState,
        event_data: &OverlayEventData,
        doc_params: &ParseDocumentParams,
    ) -> anyhow::Result<bool> {
        let mut elements_changed = false;
        #[allow(clippy::single_match)]
        match event_data {
            OverlayEventData::DevicesChanged => {
                let devices_root = parser_state
                    .get_widget_id("devices_root")
                    .unwrap_or_default();

                if devices_root.is_null() {
                    return Ok(false);
                }

                layout.remove_children(devices_root);

                for (i, device) in app.input_state.devices.iter().enumerate() {
                    let mut params = HashMap::new();

                    if matches!(device.role, TrackedDeviceRole::None) {
                        continue;
                    }

                    let template = device.role.as_ref();

                    params.insert("idx".into(), i.to_string().into());
                    parser_state.instantiate_template(
                        doc_params,
                        template,
                        layout,
                        devices_root,
                        params,
                    )?;
                }
                elements_changed = true;
            }
            _ => {}
        }

        Ok(elements_changed)
    }
}
