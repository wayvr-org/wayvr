use anyhow::Context;
use dbus::message::MatchRule;
use serde::Deserialize;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use wgui::i18n::Translation;
use wlx_common::overlays::ToastTopic;

use crate::{
    overlays::toast::{BakedToast, Toast},
    state::AppSession,
    subsystem::dbus::DbusConnector,
};

pub struct NotificationManager {
    rx_toast: mpsc::Receiver<BakedToast>,
    tx_toast: mpsc::SyncSender<BakedToast>,
    running: Arc<AtomicBool>,
}

impl NotificationManager {
    pub fn new() -> Self {
        let (tx_toast, rx_toast) = mpsc::sync_channel(10);
        Self {
            rx_toast,
            tx_toast,
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn drain_pending(&self, session: &AppSession) -> Vec<BakedToast> {
        if session.config.notifications_enabled {
            self.rx_toast.try_iter().collect()
        } else {
            // consume without submitting
            for _ in self.rx_toast.try_iter() {}

            Vec::new()
        }
    }

    pub fn run_dbus(&mut self, dbus: &mut DbusConnector) {
        let rule = MatchRule::new_method_call()
            .with_member("Notify")
            .with_interface("org.freedesktop.Notifications")
            .with_path("/org/freedesktop/Notifications");

        let sender = self.tx_toast.clone();
        if dbus
            .become_monitor(
                rule.clone(),
                Box::new(move |msg, _| {
                    if let Ok(toast) = parse_dbus(&msg) {
                        let _ = sender
                            .try_send(toast)
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
                    if let Ok(toast) = parse_dbus(msg) {
                        let _ = sender
                            .try_send(toast)
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
            let addr = "127.0.0.1:42069";
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

                    match sender.try_send(
                        toast.build_raw(), /* we don't use translation keys here */
                    ) {
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
    }
}

fn parse_dbus(msg: &dbus::Message) -> anyhow::Result<BakedToast> {
    let mut args = msg.iter_init();
    let app_name: String = args.read()?;
    let _replaces_id: u32 = args.read()?;
    let _app_icon: String = args.read()?;
    let summary: String = args.read()?;
    let body: String = args.read()?;

    let title = if summary.is_empty() {
        app_name
    } else {
        summary
    };

    let toast = Toast::new(
        ToastTopic::DesktopNotification,
        Some(Translation::from_raw_text_string(title)),
        Translation::from_raw_text_string(body),
    )
    .with_timeout(5.0)
    .with_opacity(1.0);

    Ok(toast.build_raw())
    // leave the audio part to the desktop env
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
