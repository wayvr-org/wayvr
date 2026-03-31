use std::{collections::HashMap, rc::Rc};

use wgui::{
    components::button::ComponentButton,
    event::{CallbackDataCommon, EventAlterables},
    layout::Layout,
    parser::{Fetchable, ParseDocumentParams, ParserState},
};

use crate::windowing::backend::OverlayEventData;

#[derive(Default)]
/// Populates `id="sets_root"` by instantiating the `<Set>` template.
/// Passes `idx`, `display` parameters.
pub struct SetList {
    set_buttons: Vec<Rc<ComponentButton>>,
    current_set: Option<usize>,
}

impl SetList {
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
            OverlayEventData::ActiveSetChanged(current_set) => {
                let mut com = CallbackDataCommon {
                    alterables,
                    state: &layout.state,
                };
                if let Some(old_set) = self.current_set.take()
                    && let Some(old_set) = self.set_buttons.get_mut(old_set)
                {
                    old_set.set_sticky_state(&mut com, false);
                }
                if let Some(new_set) = current_set
                    && let Some(new_set) = self.set_buttons.get_mut(*new_set)
                {
                    new_set.set_sticky_state(&mut com, true);
                }
                self.current_set = *current_set;
            }
            OverlayEventData::NumSetsChanged(num_sets) => {
                let sets_root = parser_state.get_widget_id("sets_root")?;
                layout.remove_children(sets_root);
                self.set_buttons.clear();

                for i in 0..*num_sets {
                    let mut params = HashMap::new();
                    params.insert("idx".into(), i.to_string().into());
                    params.insert("display".into(), (i + 1).to_string().into());
                    parser_state
                        .instantiate_template(doc_params, "Set", layout, sets_root, params)?;
                    let set_button =
                        parser_state.fetch_component_as::<ComponentButton>(&format!("set_{i}"))?;
                    if self.current_set == Some(i) {
                        let mut com = CallbackDataCommon {
                            alterables,
                            state: &layout.state,
                        };
                        set_button.set_sticky_state(&mut com, true);
                    }
                    self.set_buttons.push(set_button);
                }
                elements_changed = true;
            }
            _ => {}
        }
        Ok(elements_changed)
    }
}
