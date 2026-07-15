use evdev::{AttributeSetRef, Device as EvdevDevice, KeyCode, RelativeAxisCode};
use input::{
    AccelProfile, Device as LibinputDevice, DeviceCapability, Libinput, LibinputInterface,
    event::{
        Event, EventTrait,
        device::DeviceEvent,
        keyboard::{KeyState, KeyboardEvent, KeyboardEventTrait},
        pointer::{Axis, ButtonState, PointerEvent, PointerScrollEvent},
    },
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    io,
    os::{
        fd::{AsRawFd, OwnedFd, RawFd},
        unix::fs::OpenOptionsExt,
    },
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const IGNORE_PREFIX: &str = "WayVR";
const WATCHDOG_TIMEOUT: Duration = Duration::from_millis(5000);
const POLL_TIMEOUT_MS: i32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCombo {
    AltF4,
    AltTab,
    /// Always consumed by InputCapture internally
    CtrlAltDel,
}

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
        dx_raw: f64,
        dy_raw: f64,
    },
    PointerAxis {
        horizontal_v120: i32,
        vertical_v120: i32,
    },
    UngrabbedAll,
    KeyCombo {
        combo: KeyCombo,
        pressed: bool,
    },
}

#[derive(Debug, Clone, Copy)]
struct PointerAccelConfig {
    accel: bool,
    speed: f32,
}

impl Default for PointerAccelConfig {
    fn default() -> Self {
        Self {
            accel: true,
            speed: 0.0,
        }
    }
}

pub struct InputCapture {
    command_tx: SyncSender<Command>,
    event_rx: Receiver<CapturedEvent>,
    worker: Option<JoinHandle<()>>,
}

impl InputCapture {
    pub fn new() -> io::Result<Self> {
        let (command_tx, command_rx) = mpsc::sync_channel(8);
        let (event_tx, event_rx) = mpsc::channel();
        let (init_tx, init_rx) = mpsc::sync_channel(1);

        let worker = thread::Builder::new()
            .name("wayvr-input-capture".into())
            .spawn(move || worker_main(command_rx, event_tx, init_tx))?;

        match init_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                command_tx,
                event_rx,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(error) => {
                let _ = worker.join();
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("input worker exited during initialization: {error}"),
                ))
            }
        }
    }

    /// Returns every currently queued event without blocking.
    pub fn drain_events(&self) -> Vec<CapturedEvent> {
        let _ = self.command_tx.try_send(Command::ResetWatchdog);
        self.event_rx.try_iter().collect()
    }

    /// Exclusively grabs every currently detected keyboard and mouse.
    /// Newly connected matching devices are grabbed automatically.
    pub fn set_grabbed(&self, grabbed: bool) -> anyhow::Result<()> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);

        self.command_tx
            .send(Command::SetGrabbed {
                grabbed,
                response_tx,
            })
            .map_err(|error| anyhow::anyhow!("worker thread unreachable: {error}"))?;

        response_rx
            .recv()
            .map_err(|error| anyhow::anyhow!("worker thread unreachable: {error}"))??;

        Ok(())
    }

    /// Set acceleration profile for current and future mice
    pub fn set_pointer_accel(&self, accel: bool, speed: f32) -> anyhow::Result<()> {
        if !speed.is_finite() || !(-1.0..=1.0).contains(&speed) {
            anyhow::bail!("pointer acceleration speed must be within -1.0..=1.0");
        }

        self.command_tx
            .send(Command::SetPointerAccel { accel, speed })
            .map_err(|error| anyhow::anyhow!("worker thread unreachable: {error}"))?;

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
    SetGrabbed {
        grabbed: bool,
        response_tx: SyncSender<io::Result<()>>,
    },
    SetPointerAccel {
        accel: bool,
        speed: f32,
    },
    Shutdown,
}

struct RestrictedDevice {
    path: PathBuf,
    device: EvdevDevice,
}

#[derive(Default)]
struct RestrictedState {
    desired_grabbed: bool,
    devices: HashMap<RawFd, RestrictedDevice>,
}

impl RestrictedState {
    fn set_grabbed(&mut self, grabbed: bool) -> io::Result<()> {
        if grabbed {
            let fds = self.devices.keys().copied().collect::<Vec<_>>();
            let mut grabbed_now = Vec::new();

            for fd in fds {
                let newly_grabbed = {
                    let Some(entry) = self.devices.get_mut(&fd) else {
                        continue;
                    };
                    if entry.device.is_grabbed() {
                        Ok(false)
                    } else {
                        entry
                            .device
                            .grab()
                            .map(|()| true)
                            .map_err(|error| with_device_context("grab", &entry.path, error))
                    }
                };

                match newly_grabbed {
                    Ok(true) => grabbed_now.push(fd),
                    Ok(false) => {}
                    Err(error) => {
                        for previous_fd in grabbed_now {
                            if let Some(previous) = self.devices.get_mut(&previous_fd) {
                                let _ = previous.device.ungrab();
                            }
                        }
                        self.desired_grabbed = false;
                        return Err(error);
                    }
                }
            }

            self.desired_grabbed = true;
            return Ok(());
        }

        // devices opened during a later resume shall not be grabbed
        self.desired_grabbed = false;
        let mut first_error = None;

        for entry in self.devices.values_mut() {
            if !entry.device.is_grabbed() {
                continue;
            }

            if let Err(error) = entry.device.ungrab()
                && first_error.is_none()
            {
                first_error = Some(with_device_context("ungrab", &entry.path, error));
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[derive(Clone)]
struct RestrictedInterface {
    state: Rc<RefCell<RestrictedState>>,
}

impl LibinputInterface for RestrictedInterface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        let access = flags & libc::O_ACCMODE;
        let file = OpenOptions::new()
            .custom_flags(flags)
            .read(access == libc::O_RDONLY || access == libc::O_RDWR)
            .write(access == libc::O_WRONLY || access == libc::O_RDWR)
            .open(path)
            .map_err(io_errno)?;

        // duplicate fd used for capability inspection and EVIOCGRAB
        let control_file = file.try_clone().map_err(io_errno)?;
        let control_fd: OwnedFd = control_file.into();
        let mut device = EvdevDevice::from_fd(control_fd).map_err(io_errno)?;

        if !should_capture_device(&device) {
            return Err(libc::ENODEV);
        }

        let raw_fd = file.as_raw_fd();
        let mut state = self.state.borrow_mut();

        if state.desired_grabbed {
            device.grab().map_err(io_errno)?;
        }

        state.devices.insert(
            raw_fd,
            RestrictedDevice {
                path: path.to_owned(),
                device,
            },
        );

        Ok(file.into())
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        self.state.borrow_mut().devices.remove(&fd.as_raw_fd());
        drop(fd);
    }
}

#[derive(Default)]
struct RuntimeDeviceState {
    pressed_keys: HashSet<u16>,
    active_combos: HashSet<KeyCombo>,
    horizontal_v120_remainder: f64,
    vertical_v120_remainder: f64,
}

enum ProcessResult {
    Continue,
    EmergencyUngrab,
    ReceiverGone,
}

fn worker_main(
    command_rx: Receiver<Command>,
    event_tx: Sender<CapturedEvent>,
    init_tx: SyncSender<io::Result<()>>,
) {
    let restricted_state = Rc::new(RefCell::new(RestrictedState::default()));
    let interface = RestrictedInterface {
        state: restricted_state.clone(),
    };

    let mut libinput = Libinput::new_with_udev(interface);
    if libinput.udev_assign_seat("seat0").is_err() {
        let _ = init_tx.send(Err(io::Error::new(
            io::ErrorKind::Other,
            format!("failed to assign libinput seat"),
        )));
        return;
    }

    if init_tx.send(Ok(())).is_err() {
        return;
    }

    let mut grabbed = false;
    let mut watchdog = Instant::now() + WATCHDOG_TIMEOUT;
    let mut emergency_ungrab = false;
    let mut pointer_accel = PointerAccelConfig::default();
    let mut runtime_devices = HashMap::<LibinputDevice, RuntimeDeviceState>::new();
    let mut pointer_devices = HashSet::<LibinputDevice>::new();

    'worker: loop {
        loop {
            match command_rx.try_recv() {
                Ok(Command::SetGrabbed {
                    grabbed: requested,
                    response_tx,
                }) => {
                    let result = if requested {
                        match restricted_state.borrow_mut().set_grabbed(true) {
                            Ok(()) => {
                                grabbed = true;
                                Ok(())
                            }
                            Err(error) => {
                                grabbed = false;
                                Err(error)
                            }
                        }
                    } else {
                        let ungrab_result = restricted_state.borrow_mut().set_grabbed(false);
                        grabbed = false;

                        match ungrab_result {
                            Ok(()) => Ok(()),
                            Err(error) => {
                                log::warn!(
                                    "Could not cleanly ungrab every input device: {error}. Reopening libinput devices."
                                );
                                force_reopen_ungrabbed(
                                    &mut libinput,
                                    &restricted_state,
                                    &mut runtime_devices,
                                    &mut pointer_devices,
                                    pointer_accel,
                                    &event_tx,
                                )
                            }
                        }
                    };

                    watchdog = Instant::now() + WATCHDOG_TIMEOUT;
                    clear_transient_state(&mut runtime_devices);
                    drain_pending_libinput_events(
                        &mut libinput,
                        &mut runtime_devices,
                        &mut pointer_devices,
                        pointer_accel,
                        &event_tx,
                    );

                    let _ = response_tx.send(result);
                }
                Ok(Command::SetPointerAccel { accel, speed }) => {
                    pointer_accel = PointerAccelConfig { accel, speed };
                    for mut device in pointer_devices.iter().cloned() {
                        apply_pointer_accel(&mut device, pointer_accel);
                    }
                }
                Ok(Command::ResetWatchdog) => {
                    watchdog = Instant::now() + WATCHDOG_TIMEOUT;
                }
                Ok(Command::Shutdown) => break 'worker,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break 'worker,
            }
        }

        if grabbed && Instant::now() >= watchdog {
            log::warn!("Watchdog timed out, ungrabbing all input devices.");
            emergency_ungrab = true;
        }

        if emergency_ungrab {
            force_emergency_ungrab(
                &mut libinput,
                &restricted_state,
                &mut runtime_devices,
                &mut pointer_devices,
                pointer_accel,
                &event_tx,
            );
            grabbed = false;
            emergency_ungrab = false;

            if event_tx.send(CapturedEvent::UngrabbedAll).is_err() {
                break 'worker;
            }
            continue;
        }

        let mut poll_fd = libc::pollfd {
            fd: libinput.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };

        // SAFETY: poll_fd is valid for the duration of this call
        let poll_result = unsafe { libc::poll(&mut poll_fd, 1, POLL_TIMEOUT_MS) };

        if poll_result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                log::warn!("libinput poll failed: {error}");
                thread::sleep(Duration::from_millis(20));
            }
            continue;
        }

        if poll_result == 0 {
            continue;
        }

        if poll_fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            log::error!("libinput poll fd became invalid; input worker is exiting");
            break 'worker;
        }

        if poll_fd.revents & libc::POLLIN == 0 {
            continue;
        }

        if let Err(error) = libinput.dispatch() {
            if error.kind() != io::ErrorKind::WouldBlock {
                log::warn!("libinput dispatch failed: {error}");
            }
            continue;
        }

        for event in &mut libinput {
            match process_libinput_event(
                event,
                grabbed,
                &event_tx,
                &mut runtime_devices,
                &mut pointer_devices,
                pointer_accel,
            ) {
                ProcessResult::Continue => {}
                ProcessResult::EmergencyUngrab => {
                    emergency_ungrab = true;
                    break;
                }
                ProcessResult::ReceiverGone => break 'worker,
            }
        }

        if emergency_ungrab {
            force_emergency_ungrab(
                &mut libinput,
                &restricted_state,
                &mut runtime_devices,
                &mut pointer_devices,
                pointer_accel,
                &event_tx,
            );
            grabbed = false;
            emergency_ungrab = false;

            if event_tx.send(CapturedEvent::UngrabbedAll).is_err() {
                break 'worker;
            }
        }
    }
}

fn process_libinput_event(
    event: Event,
    emit: bool,
    event_tx: &Sender<CapturedEvent>,
    runtime_devices: &mut HashMap<LibinputDevice, RuntimeDeviceState>,
    pointer_devices: &mut HashSet<LibinputDevice>,
    pointer_accel: PointerAccelConfig,
) -> ProcessResult {
    match event {
        Event::Device(event) => {
            handle_device_event(event, runtime_devices, pointer_devices, pointer_accel);
        }
        Event::Keyboard(KeyboardEvent::Key(event)) if emit => {
            let code_u32 = event.key();
            let Ok(code) = u16::try_from(code_u32) else {
                log::warn!("Ignoring out-of-range Linux key code {code_u32}");
                return ProcessResult::Continue;
            };

            let pressed = event.key_state() == KeyState::Pressed;
            let device = event.device();
            let state = runtime_devices.entry(device).or_default();

            if pressed {
                state.pressed_keys.insert(code);
            } else {
                state.pressed_keys.remove(&code);
            }

            if pressed && key_combo_is_pressed(KeyCombo::CtrlAltDel, &state.pressed_keys) {
                log::info!("Ctrl+Alt+Del pressed, ungrabbing all input devices");
                return ProcessResult::EmergencyUngrab;
            }

            // emit press/release whenever a combo's complete state changes.
            for combo in [KeyCombo::AltF4, KeyCombo::AltTab] {
                let combo_pressed = key_combo_is_pressed(combo, &state.pressed_keys);
                let was_pressed = state.active_combos.contains(&combo);

                if combo_pressed == was_pressed {
                    continue;
                }

                if combo_pressed {
                    state.active_combos.insert(combo);
                } else {
                    state.active_combos.remove(&combo);
                }

                if event_tx
                    .send(CapturedEvent::KeyCombo {
                        combo,
                        pressed: combo_pressed,
                    })
                    .is_err()
                {
                    return ProcessResult::ReceiverGone;
                }
            }

            if event_tx.send(CapturedEvent::Key { code, pressed }).is_err() {
                return ProcessResult::ReceiverGone;
            }
        }
        Event::Pointer(PointerEvent::Motion(event)) if emit => {
            let dx = event.dx();
            let dy = event.dy();
            let dx_raw = event.dx_unaccelerated();
            let dy_raw = event.dy_unaccelerated();
            if (dx != 0.0 || dy != 0.0)
                && event_tx
                    .send(CapturedEvent::PointerMotion {
                        dx,
                        dy,
                        dx_raw,
                        dy_raw,
                    })
                    .is_err()
            {
                return ProcessResult::ReceiverGone;
            }
        }
        Event::Pointer(PointerEvent::Button(event)) if emit => {
            let pressed = event.button_state() == ButtonState::Pressed;
            if event_tx
                .send(CapturedEvent::PointerButton {
                    button: event.button(),
                    pressed,
                })
                .is_err()
            {
                return ProcessResult::ReceiverGone;
            }
        }
        Event::Pointer(PointerEvent::ScrollWheel(event)) if emit => {
            let horizontal = if event.has_axis(Axis::Horizontal) {
                event.scroll_value_v120(Axis::Horizontal)
            } else {
                0.0
            };
            let vertical = if event.has_axis(Axis::Vertical) {
                event.scroll_value_v120(Axis::Vertical)
            } else {
                0.0
            };

            let state = runtime_devices.entry(event.device()).or_default();
            let horizontal_v120 = accumulate_v120(&mut state.horizontal_v120_remainder, horizontal);
            let vertical_v120 = accumulate_v120(&mut state.vertical_v120_remainder, vertical);

            // libinput already provides correct values unlike REL_WHEEL
            if (horizontal_v120 != 0 || vertical_v120 != 0)
                && event_tx
                    .send(CapturedEvent::PointerAxis {
                        horizontal_v120,
                        vertical_v120,
                    })
                    .is_err()
            {
                return ProcessResult::ReceiverGone;
            }
        }
        _ => {
            // intentionally ignore absolute pointer, finger-scroll,
            // continuous-scroll, touch, tablet and gesture events
        }
    }

    ProcessResult::Continue
}

fn handle_device_event(
    event: DeviceEvent,
    runtime_devices: &mut HashMap<LibinputDevice, RuntimeDeviceState>,
    pointer_devices: &mut HashSet<LibinputDevice>,
    pointer_accel: PointerAccelConfig,
) {
    match event {
        DeviceEvent::Added(event) => {
            let mut device = event.device();
            runtime_devices.entry(device.clone()).or_default();

            if device.has_capability(DeviceCapability::Pointer) {
                apply_pointer_accel(&mut device, pointer_accel);
                pointer_devices.insert(device);
            }
        }
        DeviceEvent::Removed(event) => {
            let device = event.device();
            runtime_devices.remove(&device);
            pointer_devices.remove(&device);
        }
        _ => {}
    }
}

fn apply_pointer_accel(device: &mut LibinputDevice, config: PointerAccelConfig) {
    if !device.config_accel_is_available() {
        return;
    }

    let profile = if config.accel {
        AccelProfile::Adaptive
    } else {
        AccelProfile::Flat
    };
    if device.config_accel_profiles().contains(&profile) {
        let _ = device.config_accel_set_profile(profile);
    }
    let _ = device.config_accel_set_speed(config.speed as _);
}

fn drain_pending_libinput_events(
    libinput: &mut Libinput,
    runtime_devices: &mut HashMap<LibinputDevice, RuntimeDeviceState>,
    pointer_devices: &mut HashSet<LibinputDevice>,
    pointer_accel: PointerAccelConfig,
    event_tx: &Sender<CapturedEvent>,
) {
    let _ = libinput.dispatch();
    for event in libinput {
        let _ = process_libinput_event(
            event,
            false,
            event_tx,
            runtime_devices,
            pointer_devices,
            pointer_accel,
        );
    }
}

fn force_reopen_ungrabbed(
    libinput: &mut Libinput,
    restricted_state: &Rc<RefCell<RestrictedState>>,
    runtime_devices: &mut HashMap<LibinputDevice, RuntimeDeviceState>,
    pointer_devices: &mut HashSet<LibinputDevice>,
    pointer_accel: PointerAccelConfig,
    event_tx: &Sender<CapturedEvent>,
) -> io::Result<()> {
    restricted_state.borrow_mut().desired_grabbed = false;

    // closing the libinput-owned descriptors forcibly releases EVIOCGRAB
    libinput.suspend();
    restricted_state.borrow_mut().devices.clear();
    runtime_devices.clear();
    pointer_devices.clear();

    libinput
        .resume()
        .map_err(|()| io::Error::new(io::ErrorKind::Other, "failed to resume libinput context"))?;

    drain_pending_libinput_events(
        libinput,
        runtime_devices,
        pointer_devices,
        pointer_accel,
        event_tx,
    );
    Ok(())
}

fn force_emergency_ungrab(
    libinput: &mut Libinput,
    restricted_state: &Rc<RefCell<RestrictedState>>,
    runtime_devices: &mut HashMap<LibinputDevice, RuntimeDeviceState>,
    pointer_devices: &mut HashSet<LibinputDevice>,
    pointer_accel: PointerAccelConfig,
    event_tx: &Sender<CapturedEvent>,
) {
    if let Err(error) = force_reopen_ungrabbed(
        libinput,
        restricted_state,
        runtime_devices,
        pointer_devices,
        pointer_accel,
        event_tx,
    ) {
        // ungrab has succeded, just not the resume
        log::error!("Could not resume libinput after emergency ungrab: {error}");
    }
}

fn clear_transient_state(runtime_devices: &mut HashMap<LibinputDevice, RuntimeDeviceState>) {
    for state in runtime_devices.values_mut() {
        state.pressed_keys.clear();
        state.active_combos.clear();
        state.horizontal_v120_remainder = 0.0;
        state.vertical_v120_remainder = 0.0;
    }
}

fn accumulate_v120(remainder: &mut f64, value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }

    let total = *remainder + value;
    let integer = total.trunc();
    let clamped = integer.clamp(i32::MIN as f64, i32::MAX as f64);
    let result = clamped as i32;

    *remainder = if integer == clamped {
        total - integer
    } else {
        0.0
    };

    result
}

fn should_capture_device(device: &EvdevDevice) -> bool {
    if device
        .name()
        .is_some_and(|name| name.starts_with(IGNORE_PREFIX))
    {
        return false;
    }

    let Some(keys) = device.supported_keys() else {
        return false;
    };

    looks_like_keyboard(keys) || looks_like_mouse(device, keys)
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

fn looks_like_mouse(device: &EvdevDevice, keys: &AttributeSetRef<KeyCode>) -> bool {
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

fn key_combo_is_pressed(combo: KeyCombo, pressed: &HashSet<u16>) -> bool {
    let alt =
        pressed.contains(&KeyCode::KEY_LEFTALT.0) || pressed.contains(&KeyCode::KEY_RIGHTALT.0);

    let ctrl =
        pressed.contains(&KeyCode::KEY_LEFTCTRL.0) || pressed.contains(&KeyCode::KEY_RIGHTCTRL.0);

    match combo {
        KeyCombo::AltF4 => alt && pressed.contains(&KeyCode::KEY_F4.0),
        KeyCombo::AltTab => alt && pressed.contains(&KeyCode::KEY_TAB.0),
        KeyCombo::CtrlAltDel => ctrl && alt && pressed.contains(&KeyCode::KEY_DELETE.0),
    }
}

fn io_errno(error: io::Error) -> i32 {
    error.raw_os_error().unwrap_or(libc::EIO)
}

fn with_device_context(action: &str, path: &Path, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("failed to {action} {}: {error}", path.display()),
    )
}
