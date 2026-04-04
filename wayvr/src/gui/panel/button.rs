use std::{
    cell::RefCell,
    collections::HashMap,
    process::{Child, Command, Stdio},
    rc::Rc,
    str::FromStr,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use anyhow::Context;
use wgui::{
    components::button::ComponentButton,
    event::{
        CallbackData, CallbackMetadata, EventCallback, EventListenerKind, MouseButtonIndex,
        StyleSetRequest,
    },
    layout::Layout,
    log::LogErr,
    parser::{self, AttribPair, CustomAttribsInfoOwned, Fetchable, ParserState},
    taffy,
    widget::EventResult,
    windowing::context_menu::{Blueprint, ContextMenu, OpenParams},
};
use wlx_common::{config::HandsfreePointer, overlays::ToastTopic};

use crate::{
    RESTART, RUNNING,
    backend::{
        task::{OverlayTask, PlayspaceTask, TaskType, ToggleMode},
        wayvr::process::KillSignal,
    },
    gui::panel::{log_cmd_invalid_arg, log_cmd_missing_arg},
    overlays::{custom::create_custom, toast::Toast, wayvr::WvrCommand},
    state::AppState,
    subsystem::hid::VirtualKey,
    windowing::{OverlaySelector, backend::OverlayEventData, window::OverlayCategory},
};

pub const BUTTON_EVENT_SUFFIX: &[&str] = &["", "2", "3", "4", "5", "6", "7", "8", "9"];

#[allow(clippy::type_complexity)]
pub const BUTTON_EVENTS: [(
    &str,
    EventListenerKind,
    fn(&mut CallbackData) -> bool,
    fn(&ComponentButton, &AppState) -> bool,
); 16] = [
    (
        "_press",
        EventListenerKind::MousePress,
        button_any,
        ignore_duration,
    ),
    (
        "_release",
        EventListenerKind::MouseRelease,
        button_any,
        ignore_duration,
    ),
    (
        "_press_left",
        EventListenerKind::MousePress,
        button_left,
        ignore_duration,
    ),
    (
        "_release_left",
        EventListenerKind::MouseRelease,
        button_left,
        ignore_duration,
    ),
    (
        "_press_right",
        EventListenerKind::MousePress,
        button_right,
        ignore_duration,
    ),
    (
        "_release_right",
        EventListenerKind::MouseRelease,
        button_right,
        ignore_duration,
    ),
    (
        "_press_middle",
        EventListenerKind::MousePress,
        button_middle,
        ignore_duration,
    ),
    (
        "_release_middle",
        EventListenerKind::MouseRelease,
        button_middle,
        ignore_duration,
    ),
    (
        "_short_release",
        EventListenerKind::MouseRelease,
        button_any,
        short_duration,
    ),
    (
        "_short_release_left",
        EventListenerKind::MouseRelease,
        button_left,
        short_duration,
    ),
    (
        "_short_release_right",
        EventListenerKind::MouseRelease,
        button_right,
        short_duration,
    ),
    (
        "_short_release_middle",
        EventListenerKind::MouseRelease,
        button_middle,
        short_duration,
    ),
    (
        "_long_release",
        EventListenerKind::MouseRelease,
        button_any,
        long_duration,
    ),
    (
        "_long_release_left",
        EventListenerKind::MouseRelease,
        button_left,
        long_duration,
    ),
    (
        "_long_release_right",
        EventListenerKind::MouseRelease,
        button_right,
        long_duration,
    ),
    (
        "_long_release_middle",
        EventListenerKind::MouseRelease,
        button_middle,
        long_duration,
    ),
];

const fn button_any(_: &mut CallbackData) -> bool {
    true
}

const fn button_left(data: &mut CallbackData) -> bool {
    if let CallbackMetadata::MouseButton(b) = data.metadata
        && matches!(b.index, MouseButtonIndex::Left)
    {
        true
    } else {
        false
    }
}
const fn button_right(data: &mut CallbackData) -> bool {
    if let CallbackMetadata::MouseButton(b) = data.metadata
        && matches!(b.index, MouseButtonIndex::Right)
    {
        true
    } else {
        false
    }
}
const fn button_middle(data: &mut CallbackData) -> bool {
    if let CallbackMetadata::MouseButton(b) = data.metadata
        && matches!(b.index, MouseButtonIndex::Middle)
    {
        true
    } else {
        false
    }
}

const fn ignore_duration(_btn: &ComponentButton, _app: &AppState) -> bool {
    true
}
fn long_duration(btn: &ComponentButton, app: &AppState) -> bool {
    btn.get_time_since_last_pressed().as_secs_f32() > app.session.config.long_press_duration
}
fn short_duration(btn: &ComponentButton, app: &AppState) -> bool {
    btn.get_time_since_last_pressed().as_secs_f32() < app.session.config.long_press_duration
}

pub(super) fn setup_custom_button<S: 'static>(
    layout: &mut Layout,
    parser_state: &ParserState,
    attribs: &CustomAttribsInfoOwned,
    context_menu: &Rc<RefCell<ContextMenu>>,
    on_custom_attribs: &parser::OnCustomAttribsFunc,
    button: Rc<ComponentButton>,
) {
    const TAG: &str = "Button";

    for (name, kind, test_button, test_duration) in &BUTTON_EVENTS {
        for suffix in BUTTON_EVENT_SUFFIX {
            let name = &format!("{name}{suffix}");
            let Some(action) = attribs.get_value(name) else {
                //if no _press2 then don't attempt _press3 etc
                break;
            };

            let mut args = action.split_whitespace();
            let Some(command) = args.next() else {
                continue;
            };

            let button = button.clone();

            let callback: EventCallback<AppState, S> = match command {
                "::ContextMenuOpen" => {
                    let Some(template_name) = args.next() else {
                        log_cmd_missing_arg(parser_state, TAG, name, command);
                        return;
                    };

                    // pass attribs with key `_context_{name}` to the context_menu template
                    let mut template_params = HashMap::new();
                    for AttribPair { attrib, value } in &attribs.pairs {
                        const PREFIX: &str = "_context_";
                        #[allow(clippy::manual_strip)]
                        if attrib.starts_with(PREFIX) {
                            template_params.insert(attrib[PREFIX.len()..].into(), value.clone());
                        }
                    }

                    let template_name: Rc<str> = template_name.into();
                    let context_menu = context_menu.clone();
                    let on_custom_attribs = on_custom_attribs.clone();

                    Box::new({
                        move |_common, data, app, _| {
                            if !test_button(data) || !test_duration(&button, app) {
                                return Ok(EventResult::Pass);
                            }

                            context_menu.borrow_mut().open(OpenParams {
                                on_custom_attribs: Some(on_custom_attribs.clone()),
                                blueprint: Blueprint::Template {
                                    template_name: template_name.clone(),
                                    template_params: template_params.clone(),
                                },
                                position: data.metadata.get_mouse_pos_absolute().unwrap(), //want panic
                            });
                            Ok(EventResult::Consumed)
                        }
                    })
                }
                "::ContextMenuClose" => {
                    let context_menu = context_menu.clone();

                    Box::new(move |_common, data, app, _| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        context_menu.borrow_mut().close();

                        Ok(EventResult::Consumed)
                    })
                }
                "::ElementSetDisplay" => {
                    let (Some(id), Some(value)) = (args.next(), args.next()) else {
                        log_cmd_missing_arg(parser_state, TAG, name, command);
                        return;
                    };

                    let Ok(widget_id) = parser_state.data.get_widget_id(id) else {
                        let msg = format!("no element with ID \"{id}\"");
                        log_cmd_invalid_arg(parser_state, TAG, name, command, &msg);
                        return;
                    };

                    let display = match value {
                        "none" => taffy::Display::None,
                        "flex" => taffy::Display::Flex,
                        "block" => taffy::Display::Block,
                        "grid" => taffy::Display::Grid,
                        _ => {
                            let msg = format!("unexpected \"{value}\"");
                            log_cmd_invalid_arg(parser_state, TAG, name, command, &msg);
                            return;
                        }
                    };

                    Box::new(move |common, data, app, _| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        common
                            .alterables
                            .set_style(widget_id, StyleSetRequest::Display(display));
                        Ok(EventResult::Consumed)
                    })
                }
                "::DashToggle" => Box::new(move |_common, data, app, _| {
                    if !test_button(data) || !test_duration(&button, app) {
                        return Ok(EventResult::Pass);
                    }

                    app.tasks
                        .enqueue(TaskType::Overlay(OverlayTask::ToggleDashboard));
                    Ok(EventResult::Consumed)
                }),
                "::SetToggle" => {
                    let arg = args.next().unwrap_or_default();
                    let Ok(set_idx) = arg.parse() else {
                        let msg = format!("expected integer, found \"{arg}\"");
                        log_cmd_invalid_arg(parser_state, TAG, name, command, &msg);
                        return;
                    };
                    Box::new(move |_common, data, app, _| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        app.tasks
                            .enqueue(TaskType::Overlay(OverlayTask::ToggleSet(set_idx)));
                        Ok(EventResult::Consumed)
                    })
                }
                "::SetSwitch" => {
                    let arg = args.next().unwrap_or_default();
                    let Ok(set_idx) = arg.parse::<i32>() else {
                        let msg = format!("expected integer, found \"{arg}\"");
                        log_cmd_invalid_arg(parser_state, TAG, name, command, &msg);
                        return;
                    };
                    let maybe_set = if set_idx < 0 {
                        None
                    } else {
                        Some(set_idx as usize)
                    };
                    Box::new(move |_common, data, app, _| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        app.tasks
                            .enqueue(TaskType::Overlay(OverlayTask::SwitchSet(maybe_set)));
                        Ok(EventResult::Consumed)
                    })
                }
                "::OverlayReset" => {
                    let arg: Arc<str> = args.collect::<Vec<_>>().join(" ").into();
                    if arg.is_empty() {
                        log_cmd_missing_arg(parser_state, TAG, name, command);
                        return;
                    }

                    Box::new(move |_common, data, app, _| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        app.tasks
                            .enqueue(TaskType::Overlay(OverlayTask::ResetOverlay(
                                OverlaySelector::Name(arg.clone()),
                            )));
                        Ok(EventResult::Consumed)
                    })
                }
                "::OverlayToggle" => {
                    let arg: Arc<str> = args.collect::<Vec<_>>().join(" ").into();
                    if arg.is_empty() {
                        log_cmd_missing_arg(parser_state, TAG, name, command);
                        return;
                    }

                    Box::new(move |_common, data, app, _| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        app.tasks
                            .enqueue(TaskType::Overlay(OverlayTask::ToggleOverlay(
                                OverlaySelector::Name(arg.clone()),
                                ToggleMode::Toggle,
                            )));
                        Ok(EventResult::Consumed)
                    })
                }
                "::OverlayDrop" => {
                    let arg: Arc<str> = args.collect::<Vec<_>>().join(" ").into();
                    if arg.is_empty() {
                        log_cmd_missing_arg(parser_state, TAG, name, command);
                        return;
                    }

                    Box::new(move |_common, data, app, _| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Drop(
                            OverlaySelector::Name(arg.clone()),
                        )));
                        Ok(EventResult::Consumed)
                    })
                }
                "::DeleteSet" => Box::new(move |_common, data, app, _state| {
                    if !test_button(data) || !test_duration(&button, app) {
                        return Ok(EventResult::Pass);
                    }

                    app.tasks
                        .enqueue(TaskType::Overlay(OverlayTask::DeleteActiveSet));
                    Ok(EventResult::Consumed)
                }),
                "::AddSet" => Box::new(move |_common, data, app, _state| {
                    if !test_button(data) || !test_duration(&button, app) {
                        return Ok(EventResult::Pass);
                    }

                    app.tasks.enqueue(TaskType::Overlay(OverlayTask::AddSet));
                    Ok(EventResult::Consumed)
                }),
                "::CustomOverlayReload" => {
                    let arg: Arc<str> = args.collect::<Vec<_>>().join(" ").into();
                    if arg.is_empty() {
                        log_cmd_missing_arg(parser_state, TAG, name, command);
                        return;
                    }

                    Box::new(move |_common, data, app, _| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
                            OverlaySelector::Name(arg.clone()),
                            Box::new(|app, owc| {
                                if !matches!(owc.category, OverlayCategory::Panel) {
                                    return;
                                }
                                let name = owc.name.clone();
                                app.tasks.enqueue(TaskType::Overlay(OverlayTask::Drop(
                                    OverlaySelector::Name(name.clone()),
                                )));
                                app.tasks.enqueue(TaskType::Overlay(OverlayTask::Create(
                                    OverlaySelector::Name(owc.name.clone()),
                                    Box::new(move |app| {
                                        if let Some(mut owc) = create_custom(app, name) {
                                            owc.show_on_spawn = true;
                                            Some(owc)
                                        } else {
                                            None
                                        }
                                    }),
                                )));
                            }),
                        )));
                        Ok(EventResult::Consumed)
                    })
                }
                "::WvrOverlayCloseWindow" => {
                    let arg: Arc<str> = args.collect::<Vec<_>>().join(" ").into();
                    if arg.is_empty() {
                        log_cmd_missing_arg(parser_state, TAG, name, command);
                        return;
                    }
                    Box::new(move |_common, data, app, _| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
                            OverlaySelector::Name(arg.clone()),
                            Box::new(move |app, owc| {
                                let _ = owc
                                    .backend
                                    .notify(
                                        app,
                                        OverlayEventData::WvrCommand(WvrCommand::CloseWindow),
                                    )
                                    .log_warn("Could not close window");
                            }),
                        )));
                        Ok(EventResult::Consumed)
                    })
                }
                "::WvrOverlayKillProcess" | "::WvrOverlayTermProcess" => {
                    let arg: Arc<str> = args.collect::<Vec<_>>().join(" ").into();
                    if arg.is_empty() {
                        log_cmd_missing_arg(parser_state, TAG, name, command);
                        return;
                    }

                    let signal = if command == "::WvrOverlayKillProcess" {
                        KillSignal::Kill
                    } else {
                        KillSignal::Term
                    };

                    Box::new(move |_common, data, app, _| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
                            OverlaySelector::Name(arg.clone()),
                            Box::new(move |app, owc| {
                                let _ = owc
                                    .backend
                                    .notify(
                                        app,
                                        OverlayEventData::WvrCommand(WvrCommand::KillProcess(
                                            signal,
                                        )),
                                    )
                                    .log_warn("Could not kill process");
                            }),
                        )));
                        Ok(EventResult::Consumed)
                    })
                }
                "::EditToggle" => Box::new(move |_common, data, app, _| {
                    if !test_button(data) || !test_duration(&button, app) {
                        return Ok(EventResult::Pass);
                    }

                    app.tasks
                        .enqueue(TaskType::Overlay(OverlayTask::ToggleEditMode));
                    Ok(EventResult::Consumed)
                }),
                #[cfg(feature = "wayland")]
                "::NewMirror" => Box::new(move |_common, data, app, _| {
                    if !test_button(data) || !test_duration(&button, app) {
                        return Ok(EventResult::Pass);
                    }

                    let name = crate::overlays::screen::mirror::new_mirror_name();
                    app.tasks.enqueue(TaskType::Overlay(OverlayTask::Create(
                        OverlaySelector::Name(name.clone()),
                        Box::new(move |app| {
                            Some(crate::overlays::screen::mirror::new_mirror(
                                name,
                                &app.session,
                            ))
                        }),
                    )));
                    Ok(EventResult::Consumed)
                }),
                "::CleanupMirrors" => Box::new(move |_common, data, app, _| {
                    if !test_button(data) || !test_duration(&button, app) {
                        return Ok(EventResult::Pass);
                    }

                    app.tasks
                        .enqueue(TaskType::Overlay(OverlayTask::CleanupMirrors));
                    Ok(EventResult::Consumed)
                }),
                "::PlayspaceReset" => Box::new(move |_common, data, app, _| {
                    if !test_button(data) || !test_duration(&button, app) {
                        return Ok(EventResult::Pass);
                    }

                    app.tasks.enqueue(TaskType::Playspace(PlayspaceTask::Reset));
                    Ok(EventResult::Consumed)
                }),
                "::PlayspaceRecenter" => Box::new(move |_common, data, app, _| {
                    if !test_button(data) || !test_duration(&button, app) {
                        return Ok(EventResult::Pass);
                    }

                    app.tasks
                        .enqueue(TaskType::Playspace(PlayspaceTask::Recenter));
                    Ok(EventResult::Consumed)
                }),
                "::PlayspaceFixFloor" => Box::new(move |_common, data, app, _| {
                    if !test_button(data) || !test_duration(&button, app) {
                        return Ok(EventResult::Pass);
                    }

                    let globals = app.wgui_globals.clone();

                    let duration_secs = 5;
                    let now = Instant::now();

                    for i in 0..duration_secs {
                        Toast::new(
                            ToastTopic::System,
                            globals.i18n().translate_and_replace(
                                "TOAST.FIXING_FLOOR_IN_X_SECS",
                                ("{SECONDS}", &format!("{}", duration_secs - i)),
                            ),
                            "TOAST.ONE_CONTROLLER_ON_FLOOR".into(),
                        )
                        .with_timeout(1.0)
                        .with_lerp_amount(1.0)
                        .with_sound(i == 0)
                        .submit_at(app, now + Duration::from_secs(i as _));
                    }

                    app.audio_sample_player
                        .play_sample(&mut app.audio_system, "fix_floor");

                    let deadline = now + Duration::from_secs(duration_secs);

                    app.tasks
                        .enqueue_at(TaskType::Playspace(PlayspaceTask::FixFloor), deadline);

                    Toast::new(ToastTopic::System, "DONE".into(), String::new())
                        .with_timeout(2.0)
                        .submit_at(app, deadline);
                    Ok(EventResult::Consumed)
                }),
                "::Shutdown" => Box::new(move |_common, data, app, _| {
                    if !test_button(data) || !test_duration(&button, app) {
                        return Ok(EventResult::Pass);
                    }

                    RUNNING.store(false, Ordering::Relaxed);
                    Ok(EventResult::Consumed)
                }),
                "::Restart" => Box::new(move |_common, data, app, _| {
                    if !test_button(data) || !test_duration(&button, app) {
                        return Ok(EventResult::Pass);
                    }
                    RUNNING.store(false, Ordering::Relaxed);
                    RESTART.store(true, Ordering::Relaxed);

                    Ok(EventResult::Consumed)
                }),
                "::HandsfreeMode" => {
                    let Some(arg) = args.next() else {
                        log_cmd_missing_arg(parser_state, TAG, name, command);
                        return;
                    };

                    let Ok(val) = HandsfreePointer::from_str(arg) else {
                        let msg = format!("expected HandsfreePointer, found \"{arg}\"");
                        log_cmd_invalid_arg(parser_state, TAG, name, command, &msg);
                        return;
                    };

                    Box::new(move |_common, data, app, _| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }
                        app.session.config.handsfree_pointer = val;
                        Ok(EventResult::Consumed)
                    })
                }
                "::SendKey" => {
                    let Some(arg) = args.next() else {
                        log_cmd_missing_arg(parser_state, TAG, name, command);
                        return;
                    };
                    let Ok(key) = VirtualKey::from_str(arg) else {
                        let msg = format!("expected VirtualKey, found \"{arg}\"");
                        log_cmd_invalid_arg(parser_state, TAG, name, command, &msg);
                        return;
                    };
                    let Some(arg) = args.next() else {
                        log_cmd_missing_arg(parser_state, TAG, name, command);
                        return;
                    };
                    let down = match arg.to_lowercase().as_str() {
                        "down" => true,
                        "up" => false,
                        _ => {
                            let msg = format!("expected \"down\" or \"up\", found \"{arg}\"");
                            log_cmd_invalid_arg(parser_state, TAG, name, command, &msg);
                            return;
                        }
                    };
                    Box::new(move |_common, data, app, _| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        app.hid_provider
                            .send_key_routed(app.wvr_server.as_mut(), key, down);
                        Ok(EventResult::Consumed)
                    })
                }
                "::ShellExec" => {
                    let state = Rc::new(ShellButtonState {
                        button: button.clone(),
                        exec: args.fold(String::new(), |c, n| c + " " + n),
                        mut_state: RefCell::new(ShellButtonMutableState::default()),
                    });

                    layout.add_event_listener::<AppState, S>(
                        attribs.widget_id,
                        EventListenerKind::InternalStateChange,
                        Box::new({
                            let state = state.clone();
                            move |_, _, _, _| {
                                shell_on_tick(&state);
                                Ok(EventResult::Pass)
                            }
                        }),
                    );

                    Box::new(move |_common, data, app, _| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        let _ = shell_on_action(&state).inspect_err(|e| log::error!("{e:?}"));
                        Ok(EventResult::Consumed)
                    })
                }
                #[cfg(feature = "osc")]
                "::OscSend" => {
                    use crate::subsystem::osc::parse_osc_value;

                    let Some(address) = args.next().map(std::string::ToString::to_string) else {
                        log_cmd_missing_arg(parser_state, TAG, name, command);
                        return;
                    };

                    let mut osc_args = vec![];

                    // collect arguments specified in the initial string
                    for arg in args {
                        if let Ok(osc_arg) = parse_osc_value(arg).inspect_err(|e| {
                            let msg = format!("Could not parse OSC value \"{arg}\": {e:?}");
                            log_cmd_invalid_arg(parser_state, TAG, name, command, &msg);
                        }) {
                            osc_args.push(osc_arg);
                        }
                    }

                    // collect arguments from _arg<n> attributes.
                    let mut arg_index = 0;
                    while let Some(arg) = attribs.get_value(&format!("_arg{arg_index}"))
                        && let Ok(osc_arg) = parse_osc_value(arg).inspect_err(|e| {
                            let msg = format!("Could not parse OSC value \"{arg}\": {e:?}");
                            log_cmd_invalid_arg(parser_state, TAG, name, command, &msg);
                        })
                    {
                        osc_args.push(osc_arg);
                        arg_index += 1;
                    }

                    Box::new(move |_common, data, app, _| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        let Some(sender) = app.osc_sender.as_mut() else {
                            log::error!("OscSend: sender is not available.");
                            return Ok(EventResult::Consumed);
                        };

                        let _ = sender
                            .send_message(address.clone(), osc_args.clone())
                            .inspect_err(|e| log::error!("OscSend: Could not send message: {e:?}"));

                        Ok(EventResult::Consumed)
                    })
                }
                // shell
                _ => return,
            };

            let id = layout.add_event_listener(attribs.widget_id, *kind, callback);
            log::debug!("Registered {action} on {:?} as {id:?}", attribs.widget_id);
        }
    }
}

#[derive(Default)]
struct ShellButtonMutableState {
    child: Option<Child>,
}

struct ShellButtonState {
    #[allow(dead_code)] // preserve lifetime of this button component
    button: Rc<ComponentButton>,
    exec: String,
    mut_state: RefCell<ShellButtonMutableState>,
}

fn shell_on_action(state: &ShellButtonState) -> anyhow::Result<()> {
    let mut mut_state = state.mut_state.borrow_mut();

    if let Some(child) = mut_state.child.as_mut()
        && matches!(child.try_wait(), Ok(None))
    {
        log::info!("ShellExec triggered while child is still running; sending SIGUSR1");
        let _ = Command::new("kill")
            .arg("-s")
            .arg("USR1")
            .arg(child.id().to_string())
            .spawn()
            .unwrap()
            .wait();

        return Ok(());
    }

    let child = Command::new("sh")
        .arg("-c")
        .arg(&state.exec)
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to run shell script: '{}'", &state.exec))?;

    mut_state.child = Some(child);

    Ok(())
}

fn shell_on_tick(state: &ShellButtonState) {
    let mut mut_state = state.mut_state.borrow_mut();

    let Some(child) = mut_state.child.as_mut() else {
        return;
    };

    if let Ok(Some(_)) = child.try_wait() {
        mut_state.child = None;
    }
}
