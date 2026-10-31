use anyhow::Context;
use dbus::{
    Message,
    blocking::{Connection, stdintf::org_freedesktop_dbus::RequestNameReply},
    channel::{MatchingReceiver, Sender},
    message::MatchRule,
};
use dbus_crossroads::Crossroads;
use serde::Deserialize;
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use wgui::i18n::Translation;
use wlx_common::overlays::ToastTopic;

use crate::{
    overlays::toast::{BakedToast, Toast, ToastId},
    state::AppSession,
    subsystem::dbus::DbusConnector,
};

pub enum NotificationEvent {
    Show(BakedToast),
    Close(ToastId),
}

pub struct NotificationManager {
    rx_toast: mpsc::Receiver<NotificationEvent>,
    tx_toast: mpsc::SyncSender<NotificationEvent>,
    tx_closed: Option<mpsc::Sender<(ToastId, u32)>>,
    running: Arc<AtomicBool>,
    dbus_thread: Option<JoinHandle<()>>,
}

impl NotificationManager {
    pub fn new() -> Self {
        let (tx_toast, rx_toast) = mpsc::sync_channel(10);
        Self {
            rx_toast,
            tx_toast,
            tx_closed: None,
            running: Arc::new(AtomicBool::new(true)),
            dbus_thread: None,
        }
    }

    pub fn drain_pending(&self, session: &AppSession) -> Vec<NotificationEvent> {
        if session.config.notifications_enabled {
            self.rx_toast.try_iter().collect()
        } else {
            self.rx_toast
                .try_iter()
                .filter_map(|event| match event {
                    NotificationEvent::Show(toast) => {
                        self.notification_closed(toast.id(), 4);
                        None
                    }
                    NotificationEvent::Close(id) => Some(NotificationEvent::Close(id)),
                })
                .collect()
        }
    }

    pub fn notification_closed(&self, id: ToastId, reason: u32) {
        if let Some(sender) = &self.tx_closed {
            let _ = sender.send((id, reason));
        }
    }

    pub fn run_dbus(&mut self, dbus: &mut DbusConnector) {
        match Connection::new_session() {
            Ok(connection) => {
                match connection.request_name("org.freedesktop.Notifications", false, false, true) {
                    Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {
                        let sender = self.tx_toast.clone();
                        let running = self.running.clone();
                        let (tx_closed, rx_closed) = mpsc::channel();
                        match thread::Builder::new()
                            .name("wayvr-notifications".into())
                            .spawn(move || {
                                run_notification_server(connection, sender, rx_closed, running);
                            }) {
                            Ok(handle) => {
                                self.dbus_thread = Some(handle);
                                self.tx_closed = Some(tx_closed);
                                log::info!(
                                    "Serving D-Bus notifications as org.freedesktop.Notifications."
                                );
                                return;
                            }
                            Err(err) => log::warn!("Failed to start notification server: {err:?}"),
                        }
                    }
                    Ok(reply) => log::info!(
                        "Could not own D-Bus notifications ({reply:?}); trying BecomeMonitor."
                    ),
                    Err(err) => log::warn!("Could not own D-Bus notifications: {err:?}"),
                }
            }
            Err(err) => log::warn!("Could not connect to D-Bus for notifications: {err:?}"),
        }

        let rule = MatchRule::new_method_call()
            .with_member("Notify")
            .with_interface("org.freedesktop.Notifications")
            .with_path("/org/freedesktop/Notifications");

        let sender = self.tx_toast.clone();
        if dbus
            .become_monitor(
                rule.clone(),
                Box::new(move |msg, _| {
                    if let Ok((_, toast)) = parse_dbus(&msg) {
                        let _ = sender
                            .try_send(NotificationEvent::Show(toast))
                            .inspect_err(|e| log::error!("Failed to send notification: {e:?}"));
                    }
                    true
                }),
            )
            .context("Could not register BecomeMonitor")
            .inspect_err(|e| log::warn!("{e:?}"))
            .is_ok()
        {
            log::info!("Listening to D-Bus notifications via BecomeMonitor.");
            return;
        }

        let sender = self.tx_toast.clone();
        let _ = dbus
            .add_match(
                rule.with_eavesdrop(),
                Box::new(move |(), _, msg| {
                    if let Ok((_, toast)) = parse_dbus(msg) {
                        let _ = sender
                            .try_send(NotificationEvent::Show(toast))
                            .inspect_err(|e| log::error!("Failed to send notification: {e:?}"));
                    }
                    true
                }),
            )
            .context("Failed to register D-Bus notifications. Desktop notifications won't work.")
            .inspect_err(|e| log::warn!("{e:?}"));
    }

    pub fn run_udp(&mut self) {
        let sender = self.tx_toast.clone();
        let running = self.running.clone();
        let _ = std::thread::spawn(move || {
            let addr = "127.0.0.1:42069"; // XsNotification compat
            let socket = match std::net::UdpSocket::bind(addr) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("Failed to bind notification socket @ {addr}: {e:?}");
                    return;
                }
            };
            if let Err(err) = socket.set_read_timeout(Some(Duration::from_millis(200))) {
                log::error!("Failed to set read timeout: {err:?}");
            }

            let mut buf = [0u8; 1024 * 16]; // vrcx embeds icons as b64

            while running.load(Ordering::Relaxed) {
                if let Ok((num_bytes, _)) = socket.recv_from(&mut buf) {
                    let json_str = match std::str::from_utf8(&buf[..num_bytes]) {
                        Ok(s) => s,
                        Err(e) => {
                            log::error!("Failed to receive notification message: {e:?}");
                            continue;
                        }
                    };
                    let msg = match serde_json::from_str::<XsoMessage>(json_str) {
                        Ok(m) => m,
                        Err(e) => {
                            log::error!("Failed to parse notification message: {e:?}");
                            continue;
                        }
                    };

                    if msg.messageType != 1 {
                        continue;
                    }

                    let toast = Toast::new(
                        ToastTopic::XSNotification,
                        Some(Translation::from_raw_text_string(msg.title)),
                        Translation::from_raw_text_string(msg.content.unwrap_or(String::new())),
                    )
                    .with_timeout(msg.timeout.unwrap_or(5.))
                    .with_sound(msg.volume.unwrap_or(-1.) >= 0.); // XSOverlay still plays at 0,

                    match sender.try_send(NotificationEvent::Show(toast.build_raw())) {
                        Ok(()) => {}
                        Err(e) => {
                            log::error!("Failed to send notification: {e:?}");
                        }
                    }
                }
            }
            log::info!("Notification listener stopped.");
        });
    }
}

impl Drop for NotificationManager {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.dbus_thread.take() {
            let _ = handle.join();
        }
    }
}

type NotifyArgs = (
    String,
    u32,
    String,
    String,
    String,
    Vec<String>,
    dbus::arg::PropMap,
    i32,
);

fn run_notification_server(
    connection: Connection,
    sender: mpsc::SyncSender<NotificationEvent>,
    rx_closed: mpsc::Receiver<(ToastId, u32)>,
    running: Arc<AtomicBool>,
) {
    let state = Arc::new(Mutex::new(HashSet::<ToastId>::new()));
    let mut crossroads = Crossroads::new();
    let iface = crossroads.register::<(), _, _>("org.freedesktop.Notifications", |b| {
        let notify_sender = sender.clone();
        let notify_state = state.clone();
        b.method(
            "Notify",
            (
                "app_name",
                "replaces_id",
                "app_icon",
                "summary",
                "body",
                "actions",
                "hints",
                "expire_timeout",
            ),
            ("id",),
            move |_, (), (app_name, replaces_id, _, summary, body, _, _, expire_timeout): NotifyArgs| {
                let mut toast = bake_notification(app_name, summary, body, expire_timeout, true);
                let mut state = notify_state.lock().unwrap();
                if let Some(id) = ToastId::from_u32(replaces_id)
                    && state.contains(&id)
                {
                    toast = toast.with_id(id);
                }
                let id = toast.id();
                notify_sender
                    .try_send(NotificationEvent::Show(toast))
                    .map_err(|err| dbus_crossroads::MethodErr::failed(&err.to_string()))?;
                state.insert(id);
                Ok((id.as_u32(),))
            },
        );
        let close_state = state.clone();
        let close_sender = sender.clone();
        b.method(
            "CloseNotification",
            ("id",),
            (),
            move |ctx, (), (raw_id,): (u32,)| {
                let id = ToastId::from_u32(raw_id)
                    .ok_or_else(|| dbus_crossroads::MethodErr::failed("Unknown notification"))?;
                let mut state = close_state.lock().unwrap();
                if !state.contains(&id) {
                    return Err(dbus_crossroads::MethodErr::failed("Unknown notification"));
                }
                close_sender
                    .try_send(NotificationEvent::Close(id))
                    .map_err(|err| dbus_crossroads::MethodErr::failed(&err.to_string()))?;
                state.remove(&id);
                if let Some(signal) = notification_closed_message(raw_id, 3) {
                    ctx.push_msg(signal);
                }
                Ok(())
            },
        );
        b.method("GetCapabilities", (), ("capabilities",), |_, (), ()| {
            Ok((vec!["body".to_string()],))
        });
        b.method(
            "GetServerInformation",
            (),
            ("name", "vendor", "version", "spec_version"),
            |_, (), ()| {
                Ok((
                    "WayVR".to_string(),
                    "WayVR".to_string(),
                    env!("CARGO_PKG_VERSION").to_string(),
                    "1.2".to_string(),
                ))
            },
        );
        b.signal::<(u32, u32), _>("NotificationClosed", ("id", "reason"));
    });
    crossroads.insert("/org/freedesktop/Notifications", &[iface], ());
    connection.start_receive(
        MatchRule::new_method_call().with_path("/org/freedesktop/Notifications"),
        Box::new(move |msg, connection| {
            let _ = crossroads.handle_message(msg, connection);
            true
        }),
    );

    while running.load(Ordering::Relaxed) {
        for (id, reason) in rx_closed.try_iter() {
            if state.lock().unwrap().remove(&id) {
                send_notification_closed(&connection, id.as_u32(), reason);
            }
        }
        if let Err(err) = connection.process(Duration::from_millis(100)) {
            log::warn!("D-Bus notification server stopped: {err:?}");
            break;
        }
    }
}

fn notification_closed_message(id: u32, reason: u32) -> Option<Message> {
    Message::new_signal(
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
        "NotificationClosed",
    )
    .ok()
    .map(|signal| signal.append2(id, reason))
}

fn send_notification_closed(connection: &Connection, id: u32, reason: u32) {
    if let Some(signal) = notification_closed_message(id, reason) {
        let _ = connection.send(signal);
    }
}

fn parse_dbus(msg: &dbus::Message) -> anyhow::Result<(u32, BakedToast)> {
    let mut args = msg.iter_init();
    let app_name: String = args.read()?;
    let replaces_id: u32 = args.read()?;
    let _app_icon: String = args.read()?;
    let summary: String = args.read()?;
    let body: String = args.read()?;
    let _: Vec<String> = args.read()?;
    let _: dbus::arg::PropMap = args.read()?;
    let expire_timeout: i32 = args.read()?;

    Ok((
        replaces_id,
        bake_notification(app_name, summary, body, expire_timeout, false),
    ))
}

fn bake_notification(
    app_name: String,
    summary: String,
    body: String,
    expire_timeout: i32,
    sound: bool,
) -> BakedToast {
    let title = if summary.is_empty() {
        app_name
    } else {
        summary
    };

    Toast::new(
        ToastTopic::DesktopNotification,
        Some(Translation::from_raw_text_string(title)),
        Translation::from_raw_text_string(body),
    )
    .with_timeout(match expire_timeout {
        0 => f32::INFINITY,
        n if n > 0 => n as f32 / 1000.0,
        _ => 5.0,
    })
    .with_opacity(1.0)
    .with_sound(sound)
    .build_raw()
}

#[allow(dead_code)]
#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct XsoMessage {
    messageType: i32,
    index: Option<i32>,
    volume: Option<f32>,
    audioPath: Option<String>,
    timeout: Option<f32>,
    title: String,
    content: Option<String>,
    icon: Option<String>,
    height: Option<f32>,
    opacity: Option<f32>,
    useBase64Icon: Option<bool>,
    sourceApp: Option<String>,
    alwaysShow: Option<bool>,
}
