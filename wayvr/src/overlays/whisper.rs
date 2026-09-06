use std::{cell::RefCell, rc::Rc, time::Duration};

use glam::Affine3A;
use wgui::{
    animation::{Animation, AnimationDuration, AnimationEasing},
    color::WguiColor,
    components::button::ComponentButton,
    drawing::Color,
    event::{CallbackDataCommon, EventCallback, StyleSetRequest},
    i18n::Translation,
    layout::{LayoutTask, WidgetID},
    log::LogErr,
    parser::Fetchable,
    taffy,
    widget::{EventResult, label::WidgetLabel, rectangle::WidgetRectangle},
};
use wlx_common::{
    data_dir,
    overlays::{BackendAttrib, BackendAttribValue, ToastTopic},
    windowing::OverlayWindowState,
};

use crate::{
    backend::task::{OverlayTask, TaskType, ToggleMode},
    gui::{
        panel::{
            GuiPanel, NewGuiPanelParams, OnCustomAttribFunc,
            button::{BUTTON_EVENT_SUFFIX, BUTTON_EVENTS},
        },
        timer::GuiTimer,
    },
    overlays::toast::Toast,
    state::AppState,
    subsystem::{
        clipboard::{self, ClipboardProvider},
        hid::VirtualKey,
        input::InputFocus,
        whisper_stt::{PttProgress, WhisperStt},
    },
    windowing::{
        OverlaySelector,
        window::{OverlayCategory, OverlayWindowConfig},
    },
};
#[cfg(feature = "wayland")]
use wlx_common::DesktopBackend;

const WHISPER_NAME: &str = "whisper";

struct WhisperState {
    clipboard_provider: Option<Box<dyn ClipboardProvider>>,
    last_transcription: Option<Rc<str>>,
    id_rect_vu_meter: WidgetID,
    id_label_progress: WidgetID,
}

impl WhisperState {
    fn set_clipboard_text(&mut self, app: &mut AppState) -> bool {
        let Some(text) = self.last_transcription.as_ref() else {
            return false;
        };

        match app.hid_provider.get_input_focus() {
            InputFocus::WayVR => {
                if let Some(wvr) = app.wvr_server.as_mut() {
                    wvr.set_clipboard(text);
                    return true;
                }
            }
            InputFocus::PhysicalScreen => {
                if let Some(clip) = self.clipboard_provider.as_mut() {
                    clip.set_clipboard_utf8(text);
                    return true;
                }
            }
        }
        false
    }
}

fn start_progress_state(
    common: &mut CallbackDataCommon,
    state: &WhisperState,
    progress_rx: std::sync::mpsc::Receiver<PttProgress>,
) {
    struct S {
        sent_samples: u32,
        processed_samples: u32,
    }

    // TODO: animation user data?
    let st = Rc::new(RefCell::new(S {
        sent_samples: 0,
        processed_samples: 0,
    }));

    let id_rect_vu_meter = state.id_rect_vu_meter;
    let id_label_progress = state.id_label_progress;

    Animation::new_ex(
        id_rect_vu_meter, /* any tbh */
        0,
        AnimationDuration::SecondsFixed(120.0), // max 120 seconds (whisper_stt MAX_DURATION is currently set to 30, account for processing time)
        AnimationEasing::Linear,
        Box::new(move |common, data| {
            let rect_vu_meter = data.obj.cast_mut::<WidgetRectangle>().unwrap();
            let mut st = st.borrow_mut();
            let mut update_text = false;

            while let Ok(msg) = progress_rx.try_recv() {
                match msg {
                    PttProgress::VuVolume(volume) => {
                        common.alterables.set_widget_visible(id_rect_vu_meter, true);
                        let color = if volume < 0.90 {
                            Color::new(0.0, 1.0, 0.0, 1.0) // ok (green)
                        } else if volume < 0.98 {
                            Color::new(1.0, 1.0, 0.0, 1.0) // almost clipping (yellow)
                        } else {
                            Color::new(1.0, 0.0, 0.0, 1.0) // clipping (red)
                        };

                        rect_vu_meter.set_color(common, WguiColor::Raw(color));

                        common.alterables.set_style(
                            id_rect_vu_meter,
                            StyleSetRequest::Width(taffy::prelude::percent(volume.sqrt())),
                        );
                    }
                    PttProgress::SentSamples(count) => {
                        st.sent_samples = count;
                        update_text = true;
                    }
                    PttProgress::ProcessedSamples(count) => {
                        st.processed_samples = count;
                        update_text = true;
                    }
                }
            }

            if update_text
                && let Some(mut label) = common
                    .state
                    .widgets
                    .get_as::<WidgetLabel>(id_label_progress)
            {
                common
                    .alterables
                    .set_widget_visible(id_label_progress, true);
                label.set_text(
                    common,
                    Translation::from_raw_text_string(format!(
                        "{}%",
                        ((st.processed_samples as f32 / st.sent_samples as f32) * 100.0).round()
                            as i32
                    )),
                );
            }
        }),
    )
    .submit(common.alterables);
}

fn reset_progress_state(common: &mut CallbackDataCommon, state: &WhisperState) {
    common
        .alterables
        .set_widget_visible(state.id_rect_vu_meter, false);
    common
        .alterables
        .set_widget_visible(state.id_label_progress, false);

    common
        .alterables
        .tasks
        .push(LayoutTask::StopAnimation(state.id_rect_vu_meter, 0));
}

pub fn create_whisper(app: &mut AppState) -> anyhow::Result<OverlayWindowConfig> {
    let clipboard_provider: Option<Box<dyn ClipboardProvider>> = match app.feats.desktop_backend {
        #[cfg(feature = "wayland")]
        DesktopBackend::Wayland => clipboard::wl::Provider::new()
            .log_err("Could not create Wayland clipboard provider")
            .ok()
            .map(|p| Box::new(p) as Box<dyn ClipboardProvider>),
        #[cfg(feature = "x11")]
        DesktopBackend::X11 => clipboard::x11::Provider::new()
            .log_err("Could not create X11 clipboard provider")
            .ok()
            .map(|p| Box::new(p) as Box<dyn ClipboardProvider>),
        _ => None,
    };

    let state = WhisperState {
        clipboard_provider,
        last_transcription: None,
        id_rect_vu_meter: Default::default(),
        id_label_progress: Default::default(),
    };
    let xml = "gui/whisper.xml";

    let on_custom_attrib: OnCustomAttribFunc = Box::new(move |layout, parser, attribs, _app| {
        let Ok(button) = parser
            .fetch_component_from_widget_id_as::<ComponentButton>(&layout.state, attribs.widget_id)
        else {
            return;
        };

        for (name, kind, test_button, test_duration) in &BUTTON_EVENTS {
            for suffix in BUTTON_EVENT_SUFFIX {
                let name = &format!("{name}{suffix}");
                let Some(action) = attribs.get_value(name) else {
                    break;
                };

                let mut args = action.split_whitespace();
                let Some(command) = args.next() else {
                    continue;
                };

                let button = button.clone();

                let callback: EventCallback<AppState, WhisperState> = match command {
                    "::WhisperTranscribeStart" => Box::new(move |common, data, app, state| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        let whisper = if let Some(x) = app.whisper_sst.as_mut() {
                            x
                        } else {
                            let model_path = data_dir::get_path("whisper")
                                .join(app.session.config.whisper_model.as_ref());
                            if model_path.is_file() {
                                app.whisper_sst = match WhisperStt::new(model_path)
                                    .log_err("Error while starting Whisper engine")
                                {
                                    Ok(x) => Some(x),
                                    Err(e) => {
                                        Toast::new(
                                            ToastTopic::System,
                                            Some(Translation::from_translation_key(
                                                "WHISPER.INIT_ERROR",
                                            )),
                                            Translation::from_raw_text_string(e.to_string()),
                                        )
                                        .with_timeout(5.)
                                        .with_sound(true)
                                        .submit(app);
                                        return Ok(EventResult::Consumed);
                                    }
                                }
                            } else {
                                Toast::new(
                                    ToastTopic::System,
                                    Some(Translation::from_translation_key(
                                        "WHISPER.MODEL_NOT_DOWNLOADED",
                                    )),
                                    Translation::from_translation_key("WHISPER.DOWNLOAD_GUIDANCE"),
                                )
                                .with_timeout(5.)
                                .with_sound(true)
                                .submit(app);
                                return Ok(EventResult::Consumed);
                            }

                            app.whisper_sst.as_mut().unwrap()
                        };

                        match whisper.ptt_start() {
                            Ok(progress_rx) => {
                                start_progress_state(common, state, progress_rx);
                            }
                            Err(e) => log::error!("Could not start Whisper transcription: {e:?}"),
                        }

                        Ok(EventResult::Consumed)
                    }),
                    "::WhisperTranscribeStop" => Box::new(move |_common, data, app, _state| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        if let Some(whisper) = app.whisper_sst.as_mut() {
                            let _ = whisper
                                .ptt_end()
                                .log_err("Could not stop Whisper transcription");
                        }

                        Ok(EventResult::Consumed)
                    }),
                    "::WhisperPaste" => Box::new(move |_common, data, app, state| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        state.set_clipboard_text(app);

                        Ok(EventResult::Consumed)
                    }),
                    "::WhisperPasteAndGo" => Box::new(move |_common, data, app, state| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        if state.set_clipboard_text(app) {
                            // send ctrl-v
                            app.hid_provider.send_key_routed(
                                app.wvr_server.as_mut(),
                                VirtualKey::RCtrl,
                                true,
                            );
                            app.hid_provider.send_key_routed(
                                app.wvr_server.as_mut(),
                                VirtualKey::V,
                                true,
                            );
                            app.hid_provider.send_key_routed(
                                app.wvr_server.as_mut(),
                                VirtualKey::V,
                                false,
                            );
                            app.hid_provider.send_key_routed(
                                app.wvr_server.as_mut(),
                                VirtualKey::RCtrl,
                                false,
                            );
                        }

                        Ok(EventResult::Consumed)
                    }),
                    #[cfg(feature = "osc")]
                    "::WhisperSendOSC" => Box::new(move |_common, data, app, state| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        if let Some(text) = state.last_transcription.as_ref()
                            && let Some(osc) = app.osc_sender.as_mut()
                        {
                            use rosc::OscType;

                            let _ = osc
                                .send_message(
                                    "/chatbox/input".into(),
                                    vec![OscType::String(text.to_string()), OscType::Bool(true)],
                                )
                                .log_err("Could not send OSC message");
                        }

                        Ok(EventResult::Consumed)
                    }),
                    "::WhisperUnloadAndClose" => Box::new(move |_common, data, app, _state| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        app.whisper_sst = None;
                        app.tasks
                            .enqueue(TaskType::Overlay(OverlayTask::ToggleOverlay(
                                OverlaySelector::Name(WHISPER_NAME.into()),
                                ToggleMode::EnsureOff,
                            )));

                        Ok(EventResult::Consumed)
                    }),
                    _ => return,
                };

                let id = layout.add_event_listener(attribs.widget_id, *kind, callback);
                log::debug!("Registered {action} on {:?} as {id:?}", attribs.widget_id);
            }
        }
    });

    let params = NewGuiPanelParams {
        on_custom_attrib: Some(on_custom_attrib),
        ..Default::default()
    };

    let mut panel = GuiPanel::new_from_template(app, xml, state, params)?;
    panel.extra_attribs.insert(
        BackendAttrib::Icon,
        BackendAttribValue::Icon("@/icons/mic.svg".into()),
    );
    let id_label_transcription = panel.parser_state.get_widget_id("transcription")?;
    panel.state.id_label_progress = panel.parser_state.get_widget_id("label_progress")?;
    panel.state.id_rect_vu_meter = panel.parser_state.get_widget_id("rect_vu_meter")?;

    #[cfg(not(feature = "osc"))]
    {
        use wgui::event::{CallbackDataCommon, StyleSetRequest};
        let osc_button = panel
            .parser_state
            .fetch_component_as::<ComponentButton>("btn_osc_send")?;
        let common = CallbackDataCommon {
            state: &panel.layout.state,
            alterables: &mut panel.layout.alterables,
        };
        common.alterables.set_style(
            osc_button.get_rect(),
            StyleSetRequest::Display(wgui::taffy::Display::None),
        );
    }

    panel
        .timers
        .push(GuiTimer::new(Duration::from_millis(100), 0));

    let on_label_tick: EventCallback<AppState, WhisperState> =
        Box::new(move |common, data, app, state| {
            if let Some(whisper_stt) = app.whisper_sst.as_mut()
                && let Some(text) = whisper_stt.take_transcription()
            {
                reset_progress_state(common, state);

                let text: Rc<str> = text.into();
                state.last_transcription = Some(text.clone());
                let label = data.obj.get_as_mut::<WidgetLabel>().unwrap();
                label.set_text(common, Translation::from_raw_text_rc(text));
            }
            Ok(EventResult::Pass)
        });

    panel.layout.add_event_listener(
        id_label_transcription,
        wgui::event::EventListenerKind::InternalStateChange,
        on_label_tick,
    );

    panel.update_layout(app)?;

    #[allow(clippy::unreadable_literal)]
    let transform = Affine3A::from_cols_array_2d(&[
        [0.49993715, -0.00020921684, -0.008030709],
        [-0.0021463279, 0.47818363, -0.14607349],
        [0.007741399, 0.1460891, 0.478121],
        [-0.021562248, -0.40786624, -0.3346647],
    ]);

    Ok(OverlayWindowConfig {
        name: WHISPER_NAME.into(),
        default_state: OverlayWindowState {
            interactable: true,
            grabbable: true,
            transform,
            positioning: app.session.config.default_positioning.into(),
            alpha: app.session.config.default_opacity,
            ..OverlayWindowState::default()
        },
        category: OverlayCategory::BuiltInPanel,
        ..OverlayWindowConfig::from_backend(Box::new(panel))
    })
}
