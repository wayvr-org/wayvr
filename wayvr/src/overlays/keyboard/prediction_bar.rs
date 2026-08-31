use std::{collections::HashMap, rc::Rc};

use super::{
    KeyButtonData, KeyState, KeyboardState,
    builder::{new_doc_params, on_enter_anim, on_leave_anim, on_press_anim, on_release_anim},
};
use crate::{gui::panel::GuiPanel, state::AppState};
use anyhow::bail;
use slotmap::Key;
use wgui::event::StyleSetRequest;
use wgui::{
    event::EventListenerKind,
    layout::LayoutTask,
    parser::{Fetchable, TemplateParams},
    taffy::Display,
    widget::{EventResult, rectangle::WidgetRectangle},
};

/// Root widget that hosts the swipe-to-type prediction candidates.
pub(super) const ROOT: &str = "swipe_predictions_root";

/// Render the prediction candidate bar from the latest received predictions.
pub(super) fn update(
    panel: &mut GuiPanel<KeyboardState>,
    app: &mut AppState,
) -> anyhow::Result<bool> {
    let mut elements_changed = false;

    let anim_mult = app.wgui_theme.animation_mult;

    if let Some(slot) = panel.state.swipe_candidate_slot.as_mut()
        && let Some(candidates) = slot.take()
    {
        let predictions_root = panel.parser_state.get_widget_id(ROOT).unwrap_or_default();

        if predictions_root.is_null() {
            return Ok(elements_changed);
        }
        let doc_params = new_doc_params(panel);

        panel.layout.remove_children(predictions_root);

        let Some(new_suggestions) = candidates else {
            return Ok(elements_changed);
        };

        let mut iter = new_suggestions.iter();
        let Some(best_prediction) = iter.next() else {
            bail!("not enough swipe predictions");
        };
        if let Some(manager) = panel.state.swipe_typing_manager.as_mut() {
            manager.select_word(best_prediction, app, panel.state.modifiers);
        }
        for (i, prediction) in iter.enumerate() {
            let mut params = HashMap::new();
            let id: Rc<str> = Rc::from(format!("Prediction-{i}"));
            params.insert("id".into(), id.clone());
            params.insert("text".into(), prediction.clone().into());

            panel.parser_state.instantiate_template(
                &doc_params,
                "KeyPrediction",
                &mut panel.layout,
                predictions_root,
                TemplateParams::from_hashmap(params),
            )?;

            if let Ok(widget_id) = panel.parser_state.get_widget_id(&id) {
                let key_state = {
                    let rect = panel
                        .layout
                        .state
                        .widgets
                        .get_as::<WidgetRectangle>(widget_id)
                        .unwrap(); // want panic

                    Rc::new(KeyState {
                        // fake button state just so we get key state for anims
                        button_state: KeyButtonData::Modifier {
                            modifier: 0,
                            sticky: core::cell::Cell::new(false),
                        },
                        color: rect.params.color,
                        color2: rect.params.color2,
                        base_border_color: rect.params.border_color,
                        cur_border_color: rect.params.border_color.into(),
                        border: rect.params.border,
                        drawn_state: false.into(),
                        labels: Default::default(),
                        sprites: Default::default(),
                    })
                };
                panel.add_event_listener(
                    widget_id,
                    EventListenerKind::MousePress,
                    Box::new({
                        let k = key_state.clone();
                        let pred = prediction.clone();
                        move |common, data, app, state| {
                            if let Some(manager) = state.swipe_typing_manager.as_mut() {
                                manager.select_alternate_prediction(&pred, app, state.modifiers);
                                on_press_anim(k.clone(), common, data);
                            }
                            Ok(EventResult::Pass)
                        }
                    }),
                );
                panel.add_event_listener(
                    widget_id,
                    EventListenerKind::MouseEnter,
                    Box::new({
                        let k = key_state.clone();
                        move |common, data, _app, _state| {
                            on_enter_anim(k.clone(), common, data, anim_mult, 0.0);
                            Ok(EventResult::Pass)
                        }
                    }),
                );
                panel.add_event_listener(
                    widget_id,
                    EventListenerKind::MouseLeave,
                    Box::new({
                        let k = key_state.clone();
                        move |common, data, _app, _state| {
                            on_leave_anim(k.clone(), common, data, anim_mult, 0.0);
                            Ok(EventResult::Pass)
                        }
                    }),
                );
                panel.add_event_listener(
                    widget_id,
                    EventListenerKind::MouseRelease,
                    Box::new({
                        let k = key_state.clone();
                        move |common, data, _app, _state| {
                            on_release_anim(k.clone(), common, data);
                            Ok(EventResult::Pass)
                        }
                    }),
                );
            }
        }
        elements_changed = true;
    }
    Ok(elements_changed)
}

/// Show or hide the prediction candidate bar.
pub(super) fn set_visible(panel: &mut GuiPanel<KeyboardState>, visible: bool) {
    let predictions_root = panel.parser_state.get_widget_id(ROOT).unwrap_or_default();
    if predictions_root.is_null() {
        return;
    }

    if !visible {
        panel.layout.remove_children(predictions_root);
    }

    panel.layout.tasks.push(LayoutTask::SetWidgetStyle(
        predictions_root,
        StyleSetRequest::Display(if visible {
            Display::Flex
        } else {
            Display::None
        }),
    ));
}
