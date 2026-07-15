use evdev::{AttributeSetRef, Device, EventType, KeyCode, RelativeAxisCode};
use std::{
    collections::HashSet,
    io,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const IGNORE_PREFIX: &str = "WayVR";

const WATCHDOG_TIMEOUT: Duration = Duration::from_millis(5000);
const POLL_TIMEOUT_MS: i32 = 20;
const SYN_REPORT_CODE: u16 = 0;

#[derive(Debug, Clone)]
pub enum CapturedEvent {
    Key {
        code: u16,
        pressed: bool,
    },
    PointerButton {
        button: u32,
        pressed: bool,
    },
    PointerMotion {
        dx: f64,
        dy: f64,
    },
    PointerAxis {
        horizontal_v120: i32,
        vertical_v120: i32,
    },
    UngrabbedAll,
}

pub struct InputCapture {
    command_tx: SyncSender<Command>,
    event_rx: Receiver<CapturedEvent>,
    worker: Option<JoinHandle<()>>,
}

impl InputCapture {
    pub fn new() -> io::Result<Self> {
        let (command_tx, command_rx) = mpsc::sync_channel(8);
        let (event_tx, event_rx) = mpsc::sync_channel(64);

        let worker = thread::Builder::new()
            .name("wayvr-input-capture".into())
            .spawn(move || worker_main(command_rx, event_tx))?;

        Ok(Self {
            command_tx,
            event_rx,
            worker: Some(worker),
        })
    }

    /// Returns every currently queued event without blocking.
    pub fn drain_events(&self) -> Vec<CapturedEvent> {
        let _ = self.command_tx.try_send(Command::ResetWatchdog);
        self.event_rx.try_iter().collect()
    }

    /// Exclusively grabs every currently detected keyboard and mouse.
    /// Newly connected matching devices are grabbed automatically.
    pub fn set_grabbed(&self, grabbed: bool) -> anyhow::Result<()> {
        if let Err(e) = self.command_tx.send(Command::SetGrabbed { grabbed }) {
            anyhow::bail!("Worker thread unreachable: {e:?}");
        }
        Ok(())
    }
}

impl Drop for InputCapture {
    fn drop(&mut self) {
        let _ = self.command_tx.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

enum Command {
    ResetWatchdog,
    SetGrabbed { grabbed: bool },
    Shutdown,
}

struct OpenDevice {
    path: PathBuf,
    device: Device,
    is_keyboard: bool,
    is_mouse: bool,
    pending: Vec<PendingEvent>,
    pressed_keys: HashSet<u16>,
}

#[derive(Debug)]
enum PendingEvent {
    Key {
        code: u16,
        pressed: bool,
    },
    Button {
        button: u32,
        pressed: bool,
    },
    Motion {
        dx: i32,
        dy: i32,
    },
    Axis {
        horizontal: i32,
        vertical: i32,
        horizontal_hi_res: i32,
        vertical_hi_res: i32,
    },
}

fn worker_main(command_rx: Receiver<Command>, event_tx: SyncSender<CapturedEvent>) {
    let mut devices = Vec::<OpenDevice>::new();
    let mut grabbed = false;
    let mut emergency_ungrab = false;

    let mut want_rescan = true;
    let mut watchdog = Instant::now(); // in case wayvr main thread gets blocked

    'worker: loop {
        if Instant::now() >= watchdog && grabbed {
            log::warn!("Watchdog timed out, ungrabbing all input devices.");
            emergency_ungrab = true;
        } else if want_rescan {
            scan_for_devices(&mut devices, grabbed);
            want_rescan = false;
        }

        loop {
            match command_rx.try_recv() {
                Ok(Command::SetGrabbed { grabbed: requested }) => {
                    let result = if requested {
                        grab_existing_devices(&mut devices)
                    } else {
                        ungrab_existing_devices(&mut devices)
                    };

                    grabbed = requested && result.is_ok();
                    watchdog = Instant::now() + WATCHDOG_TIMEOUT;

                    for entry in &mut devices {
                        entry.pending.clear();
                        entry.pressed_keys.clear();
                        discard_available_events(&mut entry.device);
                    }
                }
                Ok(Command::ResetWatchdog) => watchdog = Instant::now() + WATCHDOG_TIMEOUT,
                Ok(Command::Shutdown) => break 'worker,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break 'worker,
            }
        }

        let mut poll_fds: Vec<libc::pollfd> = devices
            .iter()
            .map(|entry| libc::pollfd {
                fd: entry.device.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();

        // SAFETY: poll_fds is a valid contiguous array for the duration of the call.
        let poll_result = unsafe {
            libc::poll(
                poll_fds.as_mut_ptr(),
                poll_fds.len() as libc::nfds_t,
                POLL_TIMEOUT_MS,
            )
        };

        if poll_result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                thread::sleep(Duration::from_millis(20));
            }
            continue;
        }

        let mut dead = vec![false; devices.len()];

        for (index, poll_fd) in poll_fds.iter().enumerate() {
            if poll_fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                dead[index] = true;
                continue;
            }

            if poll_fd.revents & libc::POLLIN == 0 {
                continue;
            }

            match read_device_events(&mut devices[index], grabbed, &event_tx) {
                ReadResult::Continue => {}
                ReadResult::DeviceGone => dead[index] = true,
                ReadResult::EmergencyUngrab => {
                    emergency_ungrab = true;
                    break;
                }
                ReadResult::ReceiverGone => break 'worker,
            }
        }

        if emergency_ungrab {
            // dropping the descriptors forcibly releases EVIOCGRAB
            devices.clear();

            grabbed = false;
            emergency_ungrab = false;

            // rediscover the devices immediately, but leave them ungrabbed
            want_rescan = true;

            let e = event_tx.send(CapturedEvent::UngrabbedAll); // send this synchronously
            if e.is_err() {
                break 'worker;
            }

            continue;
        }

        for index in (0..dead.len()).rev() {
            if dead[index] {
                devices.swap_remove(index);
            }
        }
    }
}

fn scan_for_devices(devices: &mut Vec<OpenDevice>, grabbed: bool) {
    let known_paths: HashSet<PathBuf> = devices.iter().map(|entry| entry.path.clone()).collect();

    for (path, mut device) in evdev::enumerate() {
        if known_paths.contains(&path) {
            continue;
        }

        if device
            .name()
            .is_some_and(|name| name.starts_with(IGNORE_PREFIX))
        {
            continue;
        }

        let Some(keys) = device.supported_keys() else {
            continue;
        };

        let is_keyboard = looks_like_keyboard(keys);
        let is_mouse = looks_like_mouse(&device, keys);
        if !is_keyboard && !is_mouse {
            continue;
        }

        if device.set_nonblocking(true).is_err() {
            continue;
        }

        if grabbed && device.grab().is_err() {
            continue;
        }

        discard_available_events(&mut device);
        devices.push(OpenDevice {
            path,
            device,
            is_keyboard,
            is_mouse,
            pending: Vec::new(),
            pressed_keys: HashSet::with_capacity(8),
        });
    }
}

fn looks_like_keyboard(keys: &AttributeSetRef<KeyCode>) -> bool {
    let full_keyboard = (keys.contains(KeyCode::KEY_A) || keys.contains(KeyCode::KEY_Z))
        && [KeyCode::KEY_ESC, KeyCode::KEY_ENTER, KeyCode::KEY_SPACE]
            .into_iter()
            .filter(|key| keys.contains(*key))
            .count()
            >= 2;

    let numeric_keypad = keys.contains(KeyCode::KEY_KP0)
        && keys.contains(KeyCode::KEY_KP1)
        && keys.contains(KeyCode::KEY_KPENTER);

    full_keyboard || numeric_keypad
}

fn looks_like_mouse(device: &Device, keys: &AttributeSetRef<KeyCode>) -> bool {
    let Some(relative_axes) = device.supported_relative_axes() else {
        return false;
    };

    relative_axes.contains(RelativeAxisCode::REL_X)
        && relative_axes.contains(RelativeAxisCode::REL_Y)
        && mouse_buttons()
            .into_iter()
            .any(|button| keys.contains(button))
}

fn mouse_buttons() -> [KeyCode; 8] {
    [
        KeyCode::BTN_LEFT,
        KeyCode::BTN_RIGHT,
        KeyCode::BTN_MIDDLE,
        KeyCode::BTN_SIDE,
        KeyCode::BTN_EXTRA,
        KeyCode::BTN_FORWARD,
        KeyCode::BTN_BACK,
        KeyCode::BTN_TASK,
    ]
}

fn grab_existing_devices(devices: &mut [OpenDevice]) -> io::Result<()> {
    let mut grabbed_now = Vec::new();

    for index in 0..devices.len() {
        if devices[index].device.is_grabbed() {
            continue;
        }

        if let Err(error) = devices[index].device.grab() {
            let error = with_device_context("grab", &devices[index].path, error);
            for previous in grabbed_now {
                let dev: &mut OpenDevice = &mut devices[previous];
                let _ = dev.device.ungrab();
            }
            return Err(error);
        }

        grabbed_now.push(index);
    }

    Ok(())
}

fn ungrab_existing_devices(devices: &mut [OpenDevice]) -> io::Result<()> {
    let mut first_error = None;

    for entry in devices {
        if !entry.device.is_grabbed() {
            continue;
        }

        if let Err(error) = entry.device.ungrab() {
            if first_error.is_none() {
                first_error = Some(with_device_context("ungrab", &entry.path, error));
            }
        }
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn discard_available_events(device: &mut Device) {
    loop {
        match device.fetch_events() {
            Ok(events) => {
                if events.count() == 0 {
                    break;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
}

enum ReadResult {
    Continue,
    DeviceGone,
    EmergencyUngrab,
    ReceiverGone,
}

fn read_device_events(
    entry: &mut OpenDevice,
    emit: bool,
    event_tx: &SyncSender<CapturedEvent>,
) -> ReadResult {
    loop {
        let events = match entry.device.fetch_events() {
            Ok(events) => events.collect::<Vec<_>>(),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return ReadResult::Continue;
            }
            Err(_) => return ReadResult::DeviceGone,
        };

        if events.is_empty() {
            return ReadResult::Continue;
        }

        if !emit {
            entry.pending.clear();
            continue;
        }

        for event in events {
            if event.event_type() == EventType::KEY {
                match event.value() {
                    0 | 1 => {
                        let code = event.code();
                        let pressed = event.value() == 1;

                        if entry.is_mouse && is_mouse_button(code) {
                            entry.pending.push(PendingEvent::Button {
                                button: u32::from(code),
                                pressed,
                            });
                        } else if entry.is_keyboard {
                            entry.pending.push(PendingEvent::Key { code, pressed });

                            if !is_mouse_button(code) {
                                if pressed {
                                    entry.pressed_keys.insert(code);
                                    if emergency_chord_active(&entry.pressed_keys) {
                                        // do not forward delete or any pending events to wayvr
                                        entry.pending.clear();
                                        log::info!(
                                            "Ctrl+Alt+Del pressed, ungrabbing all input devices"
                                        );
                                        return ReadResult::EmergencyUngrab;
                                    }
                                } else {
                                    entry.pressed_keys.remove(&code);
                                }
                            }
                        }
                    }
                    2 => {
                        // kernel autorepeat. smithay/xkb performs repeat itself
                    }
                    _ => {}
                }
            } else if event.event_type() == EventType::RELATIVE && entry.is_mouse {
                push_relative_event(entry, event.code(), event.value());
            } else if event.event_type() == EventType::SYNCHRONIZATION
                && event.code() == SYN_REPORT_CODE
                && !flush_pending(entry, event_tx)
            {
                return ReadResult::ReceiverGone;
            }
        }
    }
}

fn push_relative_event(entry: &mut OpenDevice, code: u16, value: i32) {
    if code == RelativeAxisCode::REL_X.0 || code == RelativeAxisCode::REL_Y.0 {
        let (dx, dy) = if code == RelativeAxisCode::REL_X.0 {
            (value, 0)
        } else {
            (0, value)
        };

        if let Some(PendingEvent::Motion {
            dx: pending_dx,
            dy: pending_dy,
            ..
        }) = entry.pending.last_mut()
        {
            *pending_dx = pending_dx.saturating_add(dx);
            *pending_dy = pending_dy.saturating_add(dy);
        } else {
            entry.pending.push(PendingEvent::Motion { dx, dy });
        }
        return;
    }

    let axis_field = if code == RelativeAxisCode::REL_WHEEL.0 {
        AxisField::Vertical
    } else if code == RelativeAxisCode::REL_HWHEEL.0 {
        AxisField::Horizontal
    } else if code == RelativeAxisCode::REL_WHEEL_HI_RES.0 {
        AxisField::VerticalHiRes
    } else if code == RelativeAxisCode::REL_HWHEEL_HI_RES.0 {
        AxisField::HorizontalHiRes
    } else {
        return;
    };

    if !matches!(entry.pending.last(), Some(PendingEvent::Axis { .. })) {
        entry.pending.push(PendingEvent::Axis {
            horizontal: 0,
            vertical: 0,
            horizontal_hi_res: 0,
            vertical_hi_res: 0,
        });
    }

    let Some(PendingEvent::Axis {
        horizontal,
        vertical,
        horizontal_hi_res,
        vertical_hi_res,
        ..
    }) = entry.pending.last_mut()
    else {
        unreachable!();
    };

    let target = match axis_field {
        AxisField::Horizontal => horizontal,
        AxisField::Vertical => vertical,
        AxisField::HorizontalHiRes => horizontal_hi_res,
        AxisField::VerticalHiRes => vertical_hi_res,
    };
    *target = target.saturating_add(value);
}

enum AxisField {
    Horizontal,
    Vertical,
    HorizontalHiRes,
    VerticalHiRes,
}

fn flush_pending(entry: &mut OpenDevice, event_tx: &SyncSender<CapturedEvent>) -> bool {
    for pending in entry.pending.drain(..) {
        let event = match pending {
            PendingEvent::Key { code, pressed } => CapturedEvent::Key { code, pressed },
            PendingEvent::Button { button, pressed } => {
                CapturedEvent::PointerButton { button, pressed }
            }
            PendingEvent::Motion { dx, dy } => {
                if dx == 0 && dy == 0 {
                    continue;
                }
                CapturedEvent::PointerMotion {
                    dx: f64::from(dx),
                    dy: f64::from(dy),
                }
            }
            PendingEvent::Axis {
                horizontal,
                vertical,
                horizontal_hi_res,
                vertical_hi_res,
            } => {
                let horizontal_v120 = if horizontal_hi_res != 0 {
                    horizontal_hi_res
                } else {
                    horizontal.saturating_mul(120)
                };
                let vertical_v120_raw = if vertical_hi_res != 0 {
                    vertical_hi_res
                } else {
                    vertical.saturating_mul(120)
                };

                if horizontal_v120 == 0 && vertical_v120_raw == 0 {
                    continue;
                }

                CapturedEvent::PointerAxis {
                    horizontal_v120,
                    // REL_WHEEL is positive for wheel-up
                    vertical_v120: vertical_v120_raw.saturating_neg(),
                }
            }
        };

        if event_tx.send(event).is_err() {
            return false;
        }
    }

    true
}

fn is_mouse_button(code: u16) -> bool {
    mouse_buttons().into_iter().any(|button| button.0 == code)
}

fn with_device_context(action: &str, path: &Path, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("failed to {action} {}: {error}", path.display()),
    )
}

fn emergency_chord_active(pressed: &HashSet<u16>) -> bool {
    let ctrl =
        pressed.contains(&KeyCode::KEY_LEFTCTRL.0) || pressed.contains(&KeyCode::KEY_RIGHTCTRL.0);

    let alt =
        pressed.contains(&KeyCode::KEY_LEFTALT.0) || pressed.contains(&KeyCode::KEY_RIGHTALT.0);

    ctrl && alt && pressed.contains(&KeyCode::KEY_DELETE.0)
}
