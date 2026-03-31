use std::rc::Rc;

use chrono::DateTime;
use chrono::Local;
use chrono_tz::Tz;
use wgui::{
    drawing,
    event::{self, EventCallback},
    i18n::Translation,
    layout::Layout,
    parser::{CustomAttribsInfoOwned, ParserState, parse_color_hex},
    widget::{EventResult, label::WidgetLabel},
};

use crate::{
    gui::panel::{log_invalid_attrib, log_missing_attrib},
    state::AppState,
};

#[allow(clippy::too_many_lines)]
pub(super) fn setup_custom_label<S: 'static>(
    layout: &mut Layout,
    parser_state: &ParserState,
    attribs: &CustomAttribsInfoOwned,
    app: &AppState,
) {
    const TAG: &str = "label";

    let Some(source) = attribs.get_value("_source") else {
        log_missing_attrib(parser_state, TAG, "_source");
        return;
    };

    let callback: EventCallback<AppState, S> = match source {
        "battery" => {
            let Some(device) = attribs.get_value("_device") else {
                log_missing_attrib(parser_state, TAG, "_device");
                return;
            };
            let Ok(device) = device.parse::<usize>() else {
                let msg = format!("expected integer, found \"{device}\"");
                log_invalid_attrib(parser_state, TAG, "_device", &msg);
                return;
            };

            let state = BatteryLabelState {
                low_color: attribs
                    .get_value("_low_color")
                    .and_then(parse_color_hex)
                    .unwrap_or(BAT_LOW),
                normal_color: attribs
                    .get_value("_normal_color")
                    .and_then(parse_color_hex)
                    .unwrap_or(BAT_NORMAL),
                charging_color: attribs
                    .get_value("_charging_color")
                    .and_then(parse_color_hex)
                    .unwrap_or(BAT_CHARGING),
                low_threshold: attribs
                    .get_value("_low_threshold")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(BAT_LOW_THRESHOLD),
                device,
            };
            Box::new(move |common, data, app, _| {
                battery_on_tick(&state, common, data, app);
                Ok(EventResult::Pass)
            })
        }
        "clock" => {
            let Some(display) = attribs.get_value("_display") else {
                log_missing_attrib(parser_state, TAG, "_display");
                return;
            };

            let tz_str = attribs
                .get_value("_timezone")
                .and_then(|tz| {
                    tz.parse::<usize>()
                        .inspect_err(|_| {
                            let msg = format!("expected integer, found \"{tz}\"");
                            log_invalid_attrib(parser_state, TAG, "_timezone", &msg);
                        })
                        .ok()
                })
                .and_then(|tz_idx| {
                    app.session.config.timezones.get(tz_idx).or_else(|| {
                        let msg = format!("timezone index \"{tz_idx}\" is out of range");
                        log_invalid_attrib(parser_state, TAG, "_timezone", &msg);
                        None
                    })
                });

            let format = match display {
                "name" => {
                    let maybe_pretty_tz = tz_str.and_then(|tz_name| {
                        tz_name.split('/').next_back().map(|x| x.replace('_', " "))
                    });

                    let pretty_tz = maybe_pretty_tz.as_ref().map_or("Local", |x| x.as_str());

                    let mut globals = layout.state.globals.get();
                    layout
                        .state
                        .widgets
                        .cast_as::<WidgetLabel>(attribs.widget_id)
                        .unwrap()
                        .set_text_simple(&mut globals, Translation::from_raw_text(pretty_tz));

                    // does not need to be dynamic
                    return;
                }
                "date" => "%x",
                "dow" => "%A",
                "dow_short" => "%a",
                "time" => {
                    if app.session.config.clock_12h {
                        "%I:%M %p"
                    } else {
                        "%H:%M"
                    }
                }
                unk => unk,
            };

            let state = ClockLabelState {
                timezone: tz_str.and_then(|tz| {
                    tz.parse()
                        .inspect_err(|e| log::warn!("Invalid timezone: {e:?}"))
                        .ok()
                }),
                format: format.into(),
                locale: {
                    let i18n = app.wgui_globals.i18n();
                    let locale = i18n.get_locale();
                    match pure_rust_locales::Locale::try_from(locale.to_string().as_str()) {
                        Ok(loc) => loc,
                        Err(_) => pure_rust_locales::Locale::en_US,
                    }
                },
            };

            Box::new(move |common, data, _, _| {
                clock_on_tick(&state, common, data);
                Ok(EventResult::Pass)
            })
        }
        "timer" => {
            let format = attribs.get_value("_format").unwrap_or("%h:%m");

            let state = TimerLabelState {
                start: Local::now(),
                format: format.into(),
            };

            Box::new(move |common, data, _, _| {
                timer_on_tick(&state, common, data);
                Ok(EventResult::Pass)
            })
        }
        "ipd" => Box::new(|common, data, app, _| {
            ipd_on_tick(common, data, app);
            Ok(EventResult::Pass)
        }),
        unk => {
            log_invalid_attrib(parser_state, TAG, "_source", unk);
            return;
        }
    };

    layout.add_event_listener(
        attribs.widget_id,
        wgui::event::EventListenerKind::InternalStateChange,
        callback,
    );
}

const BAT_LOW: drawing::Color = drawing::Color::new(0.69, 0.38, 0.38, 1.);
const BAT_NORMAL: drawing::Color = drawing::Color::new(0.55, 0.84, 0.79, 1.);
const BAT_CHARGING: drawing::Color = drawing::Color::new(0.38, 0.50, 0.62, 1.);
const BAT_LOW_THRESHOLD: u32 = 30;

struct BatteryLabelState {
    device: usize,
    low_color: drawing::Color,
    normal_color: drawing::Color,
    charging_color: drawing::Color,
    low_threshold: u32,
}

fn battery_on_tick(
    state: &BatteryLabelState,
    common: &mut event::CallbackDataCommon,
    data: &mut event::CallbackData,
    app: &AppState,
) {
    let device = app.input_state.devices.get(state.device);
    let label = data.obj.get_as_mut::<WidgetLabel>().unwrap();

    if let Some(device) = device
        && let Some(soc) = device.soc
    {
        let soc = (soc * 100.).min(99.) as u32;
        let num_devices = app.input_state.devices.len();
        let suffix = if num_devices > 3 { "" } else { "%" };
        let text = format!("{soc}{suffix}");
        let color = if device.charging {
            state.charging_color
        } else if soc < state.low_threshold {
            state.low_color
        } else {
            state.normal_color
        };
        label.set_color(common, color, false);
        label.set_text(common, Translation::from_raw_text(&text));
        return;
    }
    label.set_text(common, Translation::default());
}

struct ClockLabelState {
    timezone: Option<Tz>,
    format: Rc<str>,
    locale: chrono::Locale,
}

fn clock_on_tick(
    state: &ClockLabelState,
    common: &mut event::CallbackDataCommon,
    data: &mut event::CallbackData,
) {
    let date_time = state.timezone.as_ref().map_or_else(
        || {
            format!(
                "{}",
                Local::now().format_localized(&state.format, state.locale)
            )
        },
        |tz| {
            format!(
                "{}",
                Local::now()
                    .with_timezone(tz)
                    .format_localized(&state.format, state.locale)
            )
        },
    );

    let label = data.obj.get_as_mut::<WidgetLabel>().unwrap();
    label.set_text(common, Translation::from_raw_text(&date_time));
}

struct TimerLabelState {
    start: DateTime<Local>,
    format: Rc<str>,
}

fn timer_on_tick(
    state: &TimerLabelState,
    common: &mut event::CallbackDataCommon,
    data: &mut event::CallbackData,
) {
    let duration = Local::now()
        .signed_duration_since(state.start)
        .num_seconds();

    let time = &state
        .format
        .replace("%s", &format!("{:02}", (duration % 60)))
        .replace("%m", &format!("{:02}", ((duration / 60) % 60)))
        .replace("%h", &format!("{:02}", ((duration / 60) / 60)));

    let label = data.obj.get_as_mut::<WidgetLabel>().unwrap();
    label.set_text(common, Translation::from_raw_text(time));
}

fn ipd_on_tick(
    common: &mut event::CallbackDataCommon,
    data: &mut event::CallbackData,
    app: &AppState,
) {
    let text = app.input_state.ipd.to_string();
    let label = data.obj.get_as_mut::<WidgetLabel>().unwrap();
    label.set_text(common, Translation::from_raw_text(&text));
}
