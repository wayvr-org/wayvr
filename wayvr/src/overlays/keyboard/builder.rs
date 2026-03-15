use std::{collections::HashMap, rc::Rc, time::Duration};
use crate::{
    app_misc,
    gui::{
        panel::{GuiPanel, NewGuiPanelParams, apply_custom_command},
        timer::GuiTimer,
    },
    overlays::keyboard::alt_modifier_to_key,
    state::AppState,
    subsystem::hid::XkbKeymap,
    windowing::backend::OverlayEventData,
};
use anyhow::Context;
use glam::{FloatExt, Mat4, Vec2, vec2, vec3};
use slotmap::Key;
use wgui::{
    animation::{Animation, AnimationEasing},
    assets::AssetPath,
    drawing::{self, Color},
    event::{self, CallbackMetadata, EventAlterables, EventListenerKind},
    layout::LayoutUpdateParams,
    log::LogErr,
    parser::{Fetchable, ParseDocumentParams},
    renderer_vk::util,
    taffy::{self, prelude::length},
    widget::{EventResult, div::WidgetDiv, rectangle::WidgetRectangle},
};
use wgui::event::StyleSetRequest;
use wgui::layout::LayoutTask;
use wgui::taffy::Display;
use crate::overlays::keyboard::swipe_type::SwipeTypingManager;
use super::{KeyButtonData, KeyState, KeyboardState, handle_press, handle_release, layout::{self, KeyCapType}, handle_mouse_motion};

const PIXELS_PER_UNIT: f32 = 60.;

fn new_doc_params(panel: &mut GuiPanel<KeyboardState>) -> ParseDocumentParams<'static> {
    ParseDocumentParams {
        globals: panel.layout.state.globals.clone(),
        path: AssetPath::FileOrBuiltIn("gui/keyboard.xml"),
        extra: panel.doc_extra.take().unwrap_or_default(),
    }
}

pub(super) fn update_swipe_prediction_bar(
    panel: &mut GuiPanel<KeyboardState>,
    app: &mut AppState
) -> anyhow::Result<bool> {
    let mut elements_changed = false;

    let (accent_color, anim_mult) = {
        let theme = &app.wgui_theme;
        (theme.accent_color, theme.animation_mult)
    };

    let new_swipe_candidates;
    if let Some(manager) = panel.state.swipe_typing_manager.as_mut()
        && manager.swipe_candidates != panel.state.swipe_bar_candidates {
        panel.state.swipe_bar_candidates = manager.swipe_candidates.clone();
        new_swipe_candidates = manager.swipe_candidates.clone();
    } else {
        return Ok(elements_changed)
    }

    let predictions_root = panel.parser_state
        .get_widget_id("swipe_predictions_root")
        .unwrap_or_default();

    if predictions_root.is_null() {
        return Ok(elements_changed)
    }
    let doc_params = new_doc_params(panel);

    panel.layout.remove_children(predictions_root);

    for (i, prediction) in new_swipe_candidates.iter().enumerate() {
        let mut params = HashMap::new();
        let id: Rc<str> = Rc::from(format!("Prediction-{i}"));
        params.insert("id".into(), id.clone());
        params.insert("text".into(), prediction.clone().into());

        panel.parser_state.instantiate_template(
            &doc_params,
            "KeyPrediction",
            &mut panel.layout,
            predictions_root,
            params
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
                            on_press_anim(k.clone(), common, data)
                        }
                        Ok(EventResult::Pass)
                    }
                })
            );
            panel.add_event_listener(
                widget_id,
                EventListenerKind::MouseEnter,
                Box::new({
                    let k = key_state.clone();
                    move |common, data, _app, _state| {
                        on_enter_anim(
                            k.clone(),
                            common,
                            data,
                            accent_color,
                            anim_mult,
                            0.0,
                        );
                        Ok(EventResult::Pass)
                    }
                })
            );
            panel.add_event_listener(
                widget_id,
                EventListenerKind::MouseLeave,
                Box::new({
                    let k = key_state.clone();
                    move |common, data, _app, _state | {
                        on_leave_anim(
                            k.clone(),
                            common,
                            data,
                            accent_color,
                            anim_mult,
                            0.0,
                        );
                        Ok(EventResult::Pass)
                    }
                })
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
                })
            );
        }
    }
    elements_changed = true;
    Ok(elements_changed)
}
#[allow(clippy::too_many_lines, clippy::significant_drop_tightening)]
pub(super) fn create_keyboard_panel(
    app: &mut AppState,
    keymap: Option<&XkbKeymap>,
    state: KeyboardState,
    layout: &layout::Layout,
) -> anyhow::Result<GuiPanel<KeyboardState>> {
    let mut panel =
        GuiPanel::new_from_template(app, "gui/keyboard.xml", state, NewGuiPanelParams::default())?;

    let doc_params = new_doc_params(&mut panel);

    let (accent_color, anim_mult) = {
        let theme = &app.wgui_theme;
        (theme.accent_color, theme.animation_mult)
    };

    let root = panel
        .parser_state
        .get_widget_id("keyboard_root")
        .context("Element with id 'keyboard_root' not found; keyboard.xml may be out of date.")?;

    let has_altgr = keymap.as_ref().is_some_and(|m| XkbKeymap::has_altgr(m));

    for row in 0..layout.key_sizes.len() {
        let (div, _) = panel.layout.add_child(
            root,
            WidgetDiv::create(),
            taffy::Style {
                flex_direction: taffy::FlexDirection::Row,
                ..Default::default()
            },
        )?;

        for col in 0..layout.key_sizes[row].len() {
            let my_size_f32 = layout.key_sizes[row][col];

            let key_width = PIXELS_PER_UNIT * my_size_f32;
            let key_height = PIXELS_PER_UNIT;

            let taffy_size = taffy::Size {
                width: length(key_width),
                height: length(PIXELS_PER_UNIT),
            };

            let Some(key) = layout.get_key_data(keymap, has_altgr, col, row) else {
                let _ = panel.layout.add_child(
                    div.id,
                    WidgetDiv::create(),
                    taffy::Style {
                        size: taffy_size,
                        min_size: taffy_size,
                        max_size: taffy_size,
                        ..Default::default()
                    },
                )?;
                continue;
            };

            let my_id: Rc<str> = Rc::from(format!("key-{row}-{col}"));

            let my_modifier = match key.button_state {
                KeyButtonData::Modifier { modifier, .. } => Some(modifier),
                _ => None,
            };

            // todo: make this easier to maintain somehow
            let mut params: HashMap<Rc<str>, Rc<str>> = HashMap::new();
            params.insert(Rc::from("id"), my_id.clone());
            params.insert(Rc::from("width"), Rc::from(key_width.to_string()));
            params.insert(Rc::from("height"), Rc::from(key_height.to_string()));

            let mut label = key.label.clone().into_iter();
            label
                .next()
                .and_then(|s| params.insert("text".into(), s.into()));

            match key.cap_type {
                KeyCapType::LetterAltGr => {
                    label
                        .next()
                        .and_then(|s| params.insert("text_altgr".into(), s.into()));
                }
                KeyCapType::Symbol => {
                    label
                        .next()
                        .and_then(|s| params.insert("text_shift".into(), s.into()));
                }
                KeyCapType::SymbolAltGr => {
                    label
                        .next()
                        .and_then(|s| params.insert("text_shift".into(), s.into()));
                    label
                        .next()
                        .and_then(|s| params.insert("text_altgr".into(), s.into()));
                }
                _ => {}
            }

            let template_key = format!("Key{:?}", key.cap_type);
            panel.parser_state.instantiate_template(
                &doc_params,
                &template_key,
                &mut panel.layout,
                div.id,
                params,
            )?;

            if let Ok(widget_id) = panel.parser_state.get_widget_id(&my_id) {
                let key_state = {
                    let rect = panel
                        .layout
                        .state
                        .widgets
                        .get_as::<WidgetRectangle>(widget_id)
                        .unwrap(); // want panic

                    Rc::new(KeyState {
                        button_state: key.button_state,
                        color: rect.params.color,
                        color2: rect.params.color2,
                        base_border_color: rect.params.border_color,
                        cur_border_color: rect.params.border_color.into(),
                        border: rect.params.border,
                        drawn_state: false.into(),
                    })
                };

                let key_cap_type: Rc<KeyCapType> = Rc::from(key.cap_type);
                let key_label: Rc<Vec<String>> = Rc::from(key.label);

                let width_mul = 1. / my_size_f32;

                panel.add_event_listener(
                    widget_id,
                    EventListenerKind::MouseEnter,
                    Box::new({
                        let k = key_state.clone();
                        move |common, data, _app, state| {
                            common.alterables.trigger_haptics();
                            on_enter_anim(
                                k.clone(),
                                common,
                                data,
                                accent_color,
                                anim_mult,
                                width_mul,
                            );


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
                            common.alterables.trigger_haptics();
                            on_leave_anim(
                                k.clone(),
                                common,
                                data,
                                accent_color,
                                anim_mult,
                                width_mul,
                            );

                            Ok(EventResult::Pass)
                        }
                    }),
                );
                panel.add_event_listener(
                    widget_id,
                    EventListenerKind::MousePress,
                    Box::new({
                        let k = key_state.clone();
                        let k_label = key_label.clone();
                        let k_cap_type = key_cap_type.clone();
                        move |common, data, app, state| {
                            let CallbackMetadata::MouseButton(button) = data.metadata else {
                                panic!("CallbackMetadata should contain MouseButton!");
                            };
                            let within_key_pos = data.metadata.get_mouse_pos_normalized(&common.alterables.transform_stack);

                            handle_press(app, &k, &k_label, &k_cap_type, &within_key_pos, state, button, button.device);
                            on_press_anim(k.clone(), common, data);
                            Ok(EventResult::Pass)
                        }
                    }),
                );
                panel.add_event_listener(
                    widget_id,
                    EventListenerKind::MouseMotion,
                    Box::new({
                        let k = key_state.clone();
                        let k_label = key_label.clone();
                        let k_cap_type = key_cap_type.clone();
                        move |common, data, app, state| {
                            let within_key_pos = data.metadata.get_mouse_pos_normalized(&common.alterables.transform_stack);
                            let CallbackMetadata::MousePosition(position) = data.metadata else {
                                panic!("CallbackMetadata should contain MousePosition!");
                            };

                            handle_mouse_motion(&k, &k_label, &k_cap_type, state, &within_key_pos, position.device);
                            Ok(EventResult::Pass)
                        }
                    }),
                );
                panel.add_event_listener(
                    widget_id,
                    EventListenerKind::MouseRelease,
                    Box::new({
                        let k = key_state.clone();
                        let k_cap_type = key_cap_type.clone();
                        move |common, data, app, state| {
                            if handle_release(app, &k, &k_cap_type, state) {
                                on_release_anim(k.clone(), common, data);
                            }

                            Ok(EventResult::Pass)
                        }
                    }),
                );

                if let Some(modifier) = my_modifier {
                    panel.add_event_listener(
                        widget_id,
                        EventListenerKind::InternalStateChange,
                        Box::new({
                            let k = key_state.clone();
                            move |common, data, _app, state| {
                                if (state.modifiers & modifier) != 0 {
                                    on_press_anim(k.clone(), common, data);
                                } else {
                                    on_release_anim(k.clone(), common, data);
                                }
                                Ok(EventResult::Pass)
                            }
                        }),
                    );
                }
            } else {
                log::warn!("No ID for key at ({row}, {col})");
            }
        }
    }

    panel.on_notify = Some(Box::new({
        let name = "kbd";
        move |panel, app, event_data| {
            let mut alterables = EventAlterables::default();

            let mut elems_changed = panel.state.overlay_list.on_notify(
                &mut panel.layout,
                &mut panel.parser_state,
                &event_data,
                &mut alterables,
                &doc_params,
            )?;

            elems_changed |= panel.state.set_list.on_notify(
                &mut panel.layout,
                &mut panel.parser_state,
                &event_data,
                &mut alterables,
                &doc_params,
            )?;

            match event_data {
                OverlayEventData::SettingsChanged => {
                    panel.state.alt_modifier =
                        alt_modifier_to_key(app.session.config.keyboard_middle_click_mode);

                    if app.session.config.clock_12h != panel.state.clock_12h {
                        panel.state.clock_12h = app.session.config.clock_12h;

                        if let Ok(clock_root) = panel
                            .parser_state
                            .get_widget_id("clock_root")
                            .log_warn("keyboard.xml may be out of date")
                        {
                            panel.layout.remove_children(clock_root);
                            panel.parser_state.instantiate_template(
                                &doc_params,
                                "Clock",
                                &mut panel.layout,
                                clock_root,
                                Default::default(),
                            )?;
                            elems_changed = true;
                        }
                    }
                    if !app.session.config.swipe_to_type_enabled {
                        panel.state.swipe_typing_manager = None;
                        let predictions_root = panel.parser_state
                            .get_widget_id("swipe_predictions_root")
                            .unwrap_or_default();

                        if !predictions_root.is_null() {
                            panel.layout.remove_children(predictions_root);

                            panel.layout.tasks.push(LayoutTask::SetWidgetStyle(
                                predictions_root,
                                StyleSetRequest::Display(Display::None),
                            ));

                        }
                    }
                    if app.session.config.swipe_to_type_enabled && panel.state.swipe_typing_manager.is_none() {
                        panel.state.swipe_typing_manager = match SwipeTypingManager::new() {
                            Ok(engine) => Some(engine),
                            Err(e) => {
                                log::error!("Error occurred while trying to load swipe engine: {:?}", e);
                                None
                            }
                        };
                        let predictions_root = panel.parser_state
                            .get_widget_id("swipe_predictions_root")
                            .unwrap_or_default();

                        if !predictions_root.is_null() {
                            panel.layout.tasks.push(LayoutTask::SetWidgetStyle(
                                predictions_root,
                                StyleSetRequest::Display(Display::Flex),
                            ));

                        }
                    }
                }

                OverlayEventData::CustomCommand { element, command } => {
                    if let Err(e) = apply_custom_command(panel, app, &element, &command) {
                        log::warn!("Could not apply {command:?} on {name}/{element}: {e:?}");
                    }
                }

                _ => {}
            }

            if elems_changed {
                panel.process_custom_elems(app);
            }

            panel.layout.process_alterables(alterables)?;
            Ok(())
        }
    }));

    panel
        .timers
        .push(GuiTimer::new(Duration::from_millis(100), 0));

    app_misc::process_layout_result(
        app,
        panel.layout.update(&mut LayoutUpdateParams {
            size: vec2(2048., 2048.),
            timestep_alpha: 0.0,
        })?,
    );

    Ok(panel)
}

const BUTTON_HOVER_SCALE: f32 = 0.1;

fn get_anim_transform(pos: f32, widget_size: Vec2, width_mult: f32) -> Mat4 {
    let scale = vec3(
        (BUTTON_HOVER_SCALE * width_mult).mul_add(pos, 1.0),
        BUTTON_HOVER_SCALE.mul_add(pos, 1.0),
        1.0,
    );

    util::centered_matrix(widget_size, &Mat4::from_scale(scale))
}

fn set_anim_color(
    key_state: &KeyState,
    rect: &mut WidgetRectangle,
    pos: f32,
    accent_color: drawing::Color,
) {
    // fade to accent color
    rect.params.color.r = key_state.color.r.lerp(accent_color.r, pos);
    rect.params.color.g = key_state.color.g.lerp(accent_color.g, pos);
    rect.params.color.b = key_state.color.b.lerp(accent_color.b, pos);

    // fade to accent color
    rect.params.color2.r = key_state.color2.r.lerp(accent_color.r, pos);
    rect.params.color2.g = key_state.color2.g.lerp(accent_color.g, pos);
    rect.params.color2.b = key_state.color2.b.lerp(accent_color.b, pos);

    // fade to white
    let cur_border_color = key_state.cur_border_color.get();
    rect.params.border_color.r = cur_border_color.r.lerp(1.0, pos);
    rect.params.border_color.g = cur_border_color.g.lerp(1.0, pos);
    rect.params.border_color.b = cur_border_color.b.lerp(1.0, pos);
    rect.params.border_color.a = cur_border_color.a.lerp(1.0, pos);

    rect.params.border = key_state.border.lerp(key_state.border * 1.5, pos);
}

fn on_enter_anim(
    key_state: Rc<KeyState>,
    common: &mut event::CallbackDataCommon,
    data: &event::CallbackData,
    accent_color: drawing::Color,
    anim_mult: f32,
    width_mult: f32,
) {
    common.alterables.animate(Animation::new(
        data.widget_id,
        (10. * anim_mult) as _,
        AnimationEasing::OutBack,
        Box::new(move |common, data| {
            let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap();
            set_anim_color(&key_state, rect, data.pos, accent_color);
            data.data.transform =
                get_anim_transform(data.pos, data.widget_boundary.size, width_mult);
            common.alterables.mark_redraw();
        }),
    ));
}

fn on_leave_anim(
    key_state: Rc<KeyState>,
    common: &mut event::CallbackDataCommon,
    data: &event::CallbackData,
    accent_color: drawing::Color,
    anim_mult: f32,
    width_mult: f32,
) {
    common.alterables.animate(Animation::new(
        data.widget_id,
        (15. * anim_mult) as _,
        AnimationEasing::OutQuad,
        Box::new(move |common, data| {
            let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap();
            set_anim_color(&key_state, rect, 1.0 - data.pos, accent_color);
            data.data.transform =
                get_anim_transform(1.0 - data.pos, data.widget_boundary.size, width_mult);
            common.alterables.mark_redraw();
        }),
    ));
}

fn on_press_anim(
    key_state: Rc<KeyState>,
    common: &mut event::CallbackDataCommon,
    data: &mut event::CallbackData,
) {
    if key_state.drawn_state.get() {
        return;
    }
    let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap();
    key_state
        .cur_border_color
        .set(Color::new(1.0, 1.0, 1.0, 1.0));
    rect.params.border_color = key_state.cur_border_color.get();
    common.alterables.mark_redraw();
    key_state.drawn_state.set(true);
}

fn on_release_anim(
    key_state: Rc<KeyState>,
    common: &mut event::CallbackDataCommon,
    data: &mut event::CallbackData,
) {
    if !key_state.drawn_state.get() {
        return;
    }
    let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap();
    key_state.cur_border_color.set(key_state.base_border_color);
    rect.params.border_color = key_state.cur_border_color.get();
    common.alterables.mark_redraw();
    key_state.drawn_state.set(false);
}
