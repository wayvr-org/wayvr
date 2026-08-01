use std::{rc::Rc, time::Duration};

use glam::Affine3A;
use wgui::{
    components::button::ComponentButton,
    event::EventCallback,
    i18n::Translation,
    log::LogErr,
    parser::Fetchable,
    widget::{EventResult, label::WidgetLabel},
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
        whisper_stt::WhisperStt,
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
        return false;
    }
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
                    "::WhisperTranscribeStart" => Box::new(move |_common, data, app, _state| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        let whisper = match app.whisper_sst.as_mut() {
                            Some(x) => x,
                            None => {
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
                                                "WHISPER.INIT_ERROR".into(),
                                                e.to_string(),
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
                                        "WHISPER.MODEL_NOT_DOWNLOADED".into(),
                                        "WHISPER.DOWNLOAD_GUIDANCE".into(),
                                    )
                                    .with_timeout(5.)
                                    .with_sound(true)
                                    .submit(app);
                                    return Ok(EventResult::Consumed);
                                }

                                app.whisper_sst.as_mut().unwrap()
                            }
                        };

                        let _ = whisper
                            .ptt_start()
                            .log_err("Could not start Whisper transcription");

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
        BackendAttribValue::Icon("icons/mic.svg".into()),
    );

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

    let label = panel.parser_state.get_widget_id("transcription")?;

    panel
        .timers
        .push(GuiTimer::new(Duration::from_millis(100), 0));

    let on_label_tick: EventCallback<AppState, WhisperState> =
        Box::new(move |common, data, app, state| {
            if let Some(whisper_stt) = app.whisper_sst.as_mut() {
                if let Some(text) = whisper_stt.take_transcription() {
                    let text: Rc<str> = text.into();
                    state.last_transcription = Some(text.clone());
                    let label = data.obj.get_as_mut::<WidgetLabel>().unwrap();
                    label.set_text(common, Translation::from_raw_text_rc(text));
                }
            }
            Ok(EventResult::Pass)
        });

    panel.layout.add_event_listener(
        label,
        wgui::event::EventListenerKind::InternalStateChange,
        on_label_tick,
    );

    panel.update_layout(app)?;

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
