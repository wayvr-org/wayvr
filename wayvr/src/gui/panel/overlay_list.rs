use std::{collections::HashMap, rc::Rc};

use slotmap::{Key, SecondaryMap};
use wgui::{
    components::button::ComponentButton,
    event::{CallbackDataCommon, EventAlterables},
    layout::Layout,
    parser::{Fetchable, ParseDocumentParams, ParserState},
};

use crate::windowing::{OverlayID, backend::OverlayEventData, window::OverlayCategory};

#[derive(Default)]
/// Helper for managing a list of overlays
/// Populates `id="panels_root"` with `<Screen>`, `<Mirror>`, `<Panel>` templates
/// Populates `id="apps_root"` with `<App>` templates (optional)
/// Uses the following parameters: `name` (All), `display` (Screen, Mirror), `icon` (App, Panel)
pub struct OverlayList {
    overlay_buttons: SecondaryMap<OverlayID, Rc<ComponentButton>>,
}

impl OverlayList {
    pub fn on_notify(
        &mut self,
        layout: &mut Layout,
        parser_state: &mut ParserState,
        event_data: &OverlayEventData,
        alterables: &mut EventAlterables,
        doc_params: &ParseDocumentParams,
    ) -> anyhow::Result<bool> {
        let mut elements_changed = false;
        match event_data {
            OverlayEventData::OverlaysChanged(metas) => {
                let panels_root = parser_state
                    .get_widget_id("panels_root")
                    .unwrap_or_default();
                let apps_root = parser_state.get_widget_id("apps_root").unwrap_or_default();

                layout.remove_children(panels_root);
                layout.remove_children(apps_root);
                self.overlay_buttons.clear();

                for (i, meta) in metas.iter().enumerate() {
                    let mut params = HashMap::new();

                    let (template, root) = match meta.category {
                        OverlayCategory::Screen => {
                            params.insert(
                                "display".into(),
                                format!(
                                    "{}{}",
                                    (*meta.name).chars().next().unwrap_or_default(),
                                    (*meta.name).chars().last().unwrap_or_default()
                                )
                                .into(),
                            );
                            ("Screen", panels_root)
                        }
                        OverlayCategory::Mirror => {
                            params.insert(
                                "display".into(),
                                (*meta.name).chars().last().unwrap().to_string().into(),
                            );
                            ("Mirror", panels_root)
                        }
                        OverlayCategory::Panel => {
                            let icon: Rc<str> = if let Some(icon) = meta.icon.as_ref() {
                                icon.to_string().into()
                            } else {
                                "edit/panel.svg".into()
                            };

                            params.insert("icon".into(), icon);
                            ("Panel", panels_root)
                        }
                        OverlayCategory::WayVR => {
                            params.insert(
                                "icon".into(),
                                meta.icon
                                    .as_ref()
                                    .expect("WayVR overlay without Icon attribute!")
                                    .as_ref()
                                    .into(),
                            );
                            ("App", apps_root)
                        }
                        OverlayCategory::Dashboard | OverlayCategory::Keyboard => {
                            let key = if matches!(meta.category, OverlayCategory::Dashboard) {
                                "btn_dashboard"
                            } else {
                                "btn_keyboard"
                            };

                            let Ok(overlay_button) =
                                parser_state.fetch_component_as::<ComponentButton>(key)
                            else {
                                continue;
                            };

                            if meta.visible {
                                let mut com = CallbackDataCommon {
                                    alterables,
                                    state: &layout.state,
                                };
                                overlay_button.set_sticky_state(&mut com, true);
                            }
                            self.overlay_buttons.insert(meta.id, overlay_button);
                            continue;
                        }
                        _ => continue,
                    };

                    if root.is_null() {
                        continue;
                    }

                    params.insert("idx".into(), i.to_string().into());
                    params.insert("name".into(), meta.name.as_ref().into());
                    parser_state
                        .instantiate_template(doc_params, template, layout, root, params)?;
                    let overlay_button = parser_state
                        .fetch_component_as::<ComponentButton>(&format!("overlay_{i}"))?;
                    if meta.visible {
                        let mut com = CallbackDataCommon {
                            alterables,
                            state: &layout.state,
                        };
                        overlay_button.set_sticky_state(&mut com, true);
                    }
                    self.overlay_buttons.insert(meta.id, overlay_button);
                }
                elements_changed = true;
            }
            OverlayEventData::VisibleOverlaysChanged(overlays) => {
                let mut com = CallbackDataCommon {
                    alterables,
                    state: &layout.state,
                };
                let mut overlay_buttons = self.overlay_buttons.clone();

                for visible in overlays.as_ref() {
                    if let Some(btn) = overlay_buttons.remove(*visible) {
                        btn.set_sticky_state(&mut com, true);
                    }
                }

                for btn in overlay_buttons.values() {
                    btn.set_sticky_state(&mut com, false);
                }
            }
            _ => {}
        }

        Ok(elements_changed)
    }
}
