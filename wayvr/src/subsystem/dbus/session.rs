use std::{
    collections::HashMap,
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{BufRead, BufReader},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::Context;
use dbus::blocking::Connection;

const ACTIVATION_VARS: &[&str] = &[
    "DBUS_SESSION_BUS_ADDRESS",
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_RUNTIME_DIR",
    "XAUTHORITY",
    "PATH",
    "GDK_BACKEND",
    "QT_QPA_PLATFORM",
];

const PORTAL_FRONTEND: Service = Service {
    name: "xdg-desktop-portal",
    bus_name: Some("org.freedesktop.portal.Desktop"),
    units: &["xdg-desktop-portal.service"],
    executables: &["xdg-desktop-portal"],
    flatpak: None,
};

const PORTAL_GTK: Service = Service {
    name: "xdg-desktop-portal-gtk",
    bus_name: Some("org.freedesktop.impl.portal.desktop.gtk"),
    units: &["xdg-desktop-portal-gtk.service"],
    executables: &["xdg-desktop-portal-gtk"],
    flatpak: None,
};

const PORTAL_KDE: Service = Service {
    name: "xdg-desktop-portal-kde",
    bus_name: Some("org.freedesktop.impl.portal.desktop.kde"),
    units: &[
        "xdg-desktop-portal-kde.service",
        "plasma-xdg-desktop-portal-kde.service",
    ],
    executables: &["xdg-desktop-portal-kde"],
    flatpak: None,
};

const POLKIT_GNOME: Service = Service {
    name: "polkit-gnome",
    bus_name: None,
    units: &[
        "polkit-gnome-authentication-agent-1.service",
        "polkit-gnome.service",
    ],
    executables: &["polkit-gnome-authentication-agent-1"],
    flatpak: None,
};

const POLKIT_KDE: Service = Service {
    name: "polkit-kde",
    bus_name: None,
    units: &["plasma-polkit-agent.service", "polkit-kde-agent-1.service"],
    executables: &["polkit-kde-authentication-agent-1"],
    flatpak: None,
};

const FCITX5: Service = Service {
    name: "fcitx5",
    bus_name: None,
    units: &[],
    executables: &["fcitx5"],
    flatpak: Some("org.fcitx.Fcitx5"),
};

struct Service {
    name: &'static str,
    bus_name: Option<&'static str>,
    units: &'static [&'static str],
    executables: &'static [&'static str],
    flatpak: Option<&'static str>,
}

pub struct XdgDesktopSession {
    bus_address: Arc<str>,
    private_bus: Option<Child>,
    worker: Option<JoinHandle<ServiceLauncher>>,
    cancelled: Arc<AtomicBool>,
    runtime_dir: Option<PathBuf>,
    portal_config: Option<PathBuf>,
    env: HashMap<String, OsString>,
    saved_systemd_env: Option<HashMap<String, Option<String>>>,
    saved_activation_env: Option<HashMap<String, Option<OsString>>>,
    systemd: bool,
}

struct ServiceLauncher {
    bus_address: Arc<str>,
    children: Vec<Child>,
    units: Vec<&'static str>,
    env: HashMap<String, OsString>,
    systemd: bool,
    cancelled: Arc<AtomicBool>,
}

impl XdgDesktopSession {
    pub fn new(wayland_display: &str, display: &str) -> anyhow::Result<Option<Self>> {
        let existing = existing_session_bus();
        if existing.is_some() && desktop_in_environment() {
            log::info!(
                "An existing desktop session is configured; leaving its D-Bus environment alone."
            );
            return Ok(None);
        }

        let (bus_address, private_bus, runtime_dir) = if let Some((_, address)) = existing {
            (address, None, None)
        } else {
            // runnning without a display manager → start our own dbus session
            let (address, process, runtime_dir) = start_private_bus()?;
            (address, Some(process), Some(runtime_dir))
        };

        let systemd = private_bus.is_none() && user_systemd_available();
        let mut session = Self {
            bus_address: bus_address.into(),
            private_bus,
            worker: None,
            cancelled: Arc::new(AtomicBool::new(false)),
            runtime_dir,
            portal_config: None,
            env: HashMap::new(),
            saved_systemd_env: None,
            saved_activation_env: None,
            systemd,
        };
        session.configure_environment(wayland_display, display);
        session.update_activation_environment()?;
        session.launch_services()?;
        Ok(Some(session))
    }

    pub const fn bus_address(&self) -> &Arc<str> {
        &self.bus_address
    }

    fn configure_environment(&mut self, wayland_display: &str, display: &str) {
        self.env
            .insert("XDG_CURRENT_DESKTOP".into(), "WayVR".into());
        self.env.insert("DISPLAY".into(), display.into());
        self.env
            .insert("WAYLAND_DISPLAY".into(), wayland_display.into());
        self.env.insert("GDK_BACKEND".into(), "wayland,x11".into());
        self.env
            .insert("QT_QPA_PLATFORM".into(), "wayland;xcb".into());

        if let Some(runtime) = &self.runtime_dir {
            self.env
                .insert("XDG_RUNTIME_DIR".into(), runtime.as_os_str().to_os_string());
            let host_runtime = original_runtime_dir();
            let display_path = Path::new(wayland_display);
            if !display_path.is_absolute() {
                self.env.insert(
                    "WAYLAND_DISPLAY".into(),
                    host_runtime.join(display_path).into_os_string(),
                );
            }
        }

        if let Err(err) = self.create_portal_config() {
            log::warn!("Could not create WayVR portal configuration: {err:#}");
        }
    }

    fn create_portal_config(&mut self) -> anyhow::Result<()> {
        let config_home = xdg::BaseDirectories::new()
            .get_config_home()
            .context("HOME is not set.")?;
        let portal_dir = config_home.join("xdg-desktop-portal");
        fs::create_dir_all(&portal_dir)?;
        let config = portal_dir.join("wayvr-portals.conf");
        if !config.exists() {
            fs::write(
                &config,
                "[preferred]\ndefault=none\norg.freedesktop.impl.portal.AppChooser=gtk;kde;\norg.freedesktop.impl.portal.FileChooser=gtk;kde;\n",
            )?;
            log::debug!("Wrote portal config to {}", config.display());
            self.portal_config = Some(config);
        }
        Ok(())
    }

    fn update_activation_environment(&mut self) -> anyhow::Result<()> {
        log::debug!("Update activation env: {:?}", self.env);
        if self.systemd {
            self.saved_systemd_env = Some(read_systemd_environment()?);
        } else if self.private_bus.is_none() {
            self.saved_activation_env = Some(
                ACTIVATION_VARS
                    .iter()
                    .map(|name| ((*name).to_string(), env::var_os(name)))
                    .collect(),
            );
        }
        let mut command = Command::new("dbus-update-activation-environment");
        if self.systemd {
            command.arg("--systemd");
        }
        for name in ACTIVATION_VARS {
            if let Some(value) = self.env.get(*name) {
                command.arg(format!("{name}={}", value.to_string_lossy()));
            } else if env::var_os(name).is_some() {
                command.arg(name);
            }
        }
        self.apply_environment(&mut command);
        if command.status().is_ok_and(|status| status.success()) {
            return Ok(());
        }
        let mut variables = HashMap::new();
        for name in ACTIVATION_VARS {
            if let Some(value) = self.env.get(*name).cloned().or_else(|| env::var_os(name)) {
                variables.insert((*name).to_string(), value.to_string_lossy().into_owned());
            }
        }
        update_bus_environment(&self.bus_address, variables.clone())?;
        if self.systemd && !variables.is_empty() {
            let assignments = variables
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>();
            let status = Command::new("systemctl")
                .args(["--user", "set-environment"])
                .args(&assignments)
                .status()?;
            if !status.success() {
                anyhow::bail!("Could not update systemd user environment: {status}");
            }
        }
        Ok(())
    }

    fn apply_environment(&self, command: &mut Command) {
        command.envs(&self.env);
    }

    fn launch_services(&mut self) -> anyhow::Result<()> {
        let launcher = ServiceLauncher {
            bus_address: self.bus_address.clone(),
            children: Vec::new(),
            units: Vec::new(),
            env: self.env.clone(),
            systemd: self.systemd,
            cancelled: self.cancelled.clone(),
        };
        self.worker = Some(
            thread::Builder::new()
                .name("wayvr-dbus-services".to_string())
                .spawn(move || launcher.run())?,
        );
        Ok(())
    }
}

impl ServiceLauncher {
    fn apply_environment(&self, command: &mut Command) {
        command.envs(&self.env);
    }

    fn run(mut self) -> Self {
        self.restart_services();
        self
    }

    fn restart_services(&mut self) {
        let connection = match Connection::new_address(&self.bus_address) {
            Ok(connection) => connection,
            Err(err) => {
                log::warn!("Cannot connect to WayVR D-Bus session: {err}");
                return;
            }
        };

        let paths_to_scan = paths_to_scan();

        let backend = [PORTAL_GTK, PORTAL_KDE]
            .iter()
            .find(|service| self.restart_service(&paths_to_scan, service, &connection))
            .map(|service| service.name);
        if let Some(name) = backend {
            log::info!("Using {name} as the preferred portal backend");
        } else {
            log::warn!(
                "No GTK or KDE desktop portal backend could be started. Install either for proper desktop support."
            );
        }

        if !self.restart_service(&paths_to_scan, &PORTAL_FRONTEND, &connection) {
            log::warn!("Could not start xdg-desktop-portal");
        } else if let Ok((xml,)) = connection
            .with_proxy(
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                Duration::from_millis(1500),
            )
            .method_call::<(String,), _, _, _>(
                "org.freedesktop.DBus.Introspectable",
                "Introspect",
                (),
            )
            && !xml.contains("org.freedesktop.portal.FileChooser")
        {
            log::warn!("xdg-desktop-portal started without a FileChooser interface");
        }

        if self.cancelled.load(Ordering::Relaxed) {
            return;
        }
        if !polkit_available() {
            log::debug!("polkit authority is unavailable; skipping authentication agent");
            return;
        }
        if !self.restart_service(&paths_to_scan, &POLKIT_GNOME, &connection)
            && !self.restart_service(&paths_to_scan, &POLKIT_KDE, &connection)
        {
            log::warn!("Could not start a GNOME or KDE polkit authentication agent");
        }

        let _ = self.restart_service(&paths_to_scan, &FCITX5, &connection);
    }

    fn restart_service(
        &mut self,
        paths_to_scan: &[PathBuf],
        service: &Service,
        connection: &Connection,
    ) -> bool {
        if self.cancelled.load(Ordering::Relaxed) {
            return false;
        }

        // if we're on systemd and there's a known systemd unit, try that first
        if self.systemd {
            for unit in service.units {
                let was_active = systemctl(&["is-active", "--quiet", unit]);
                if systemctl(&["--no-block", "restart", unit]) {
                    if !was_active {
                        self.units.push(*unit);
                    }
                    if service.bus_name.map_or_else(
                        || wait_for_unit(unit, &self.cancelled),
                        |name| wait_for_owner(connection, name, &self.cancelled),
                    ) {
                        log::debug!("{unit}: Restarted successfully");
                        return true;
                    }
                    log::warn!(
                        "{unit} started but {} did not become available.",
                        service.name
                    );
                }
            }
        }

        // wouldn't be able to restore a non-systemd service after shutdown, so just skip it
        if service
            .bus_name
            .is_some_and(|name| name_has_owner(connection, name))
        {
            log::debug!(
                "Cannot start {:?}: Already owned",
                service.bus_name.unwrap()
            );
            return true;
        }

        // launch executable directly
        for executable in service.executables {
            if self.cancelled.load(Ordering::Relaxed) {
                return false;
            }
            let Some(path) = find_executable(paths_to_scan, executable) else {
                log::trace!("{executable}: Not found");
                continue;
            };
            let mut command = Command::new(path);
            self.apply_environment(&mut command);
            command.stdin(Stdio::null()).stdout(Stdio::null());
            match command.spawn() {
                Ok(mut child) => {
                    if let Some(name) = service.bus_name {
                        if wait_for_owner(connection, name, &self.cancelled) {
                            log::debug!("{executable}: Started successfully, PID {}", child.id());
                            self.children.push(child);
                            return true;
                        }
                    } else {
                        thread::sleep(Duration::from_millis(250));
                        if matches!(child.try_wait(), Ok(None)) {
                            log::debug!("{executable}: Started successfully, PID {}", child.id());
                            self.children.push(child);
                            return true;
                        }
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                }
                Err(err) => log::warn!("Could not launch {}: {err}", service.name),
            }
        }

        if let Some(flatpak) = service.flatpak.as_ref() {
            if self.cancelled.load(Ordering::Relaxed) {
                return false;
            }

            let mut command = Command::new("/usr/bin/flatpak");
            self.apply_environment(&mut command);
            command.arg("run");
            command.arg(flatpak);
            command.stdin(Stdio::null()).stdout(Stdio::null());
            match command.spawn() {
                Ok(mut child) => {
                    if let Some(name) = service.bus_name {
                        if wait_for_owner(connection, name, &self.cancelled) {
                            log::debug!("{flatpak}: Started successfully, PID {}", child.id());
                            self.children.push(child);
                            return true;
                        }
                    } else {
                        thread::sleep(Duration::from_millis(250));
                        if matches!(child.try_wait(), Ok(None)) {
                            log::debug!("{flatpak}: Started successfully, PID {}", child.id());
                            self.children.push(child);
                            return true;
                        }
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                }
                Err(err) => log::warn!("Could not launch {}: {err}", service.name),
            }
        }
        false
    }
}

impl Drop for ServiceLauncher {
    fn drop(&mut self) {
        for unit in self.units.iter().rev() {
            let _ = systemctl(&["--no-block", "restart", unit]);
        }
        for child in self.children.iter_mut().rev() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for XdgDesktopSession {
    fn drop(&mut self) {
        if let Some(saved) = &self.saved_systemd_env {
            restore_systemd_environment(saved, &self.bus_address);
        } else if let Some(saved) = &self.saved_activation_env {
            let variables = saved
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .as_ref()
                        .map(|value| (name.clone(), value.to_string_lossy().into_owned()))
                })
                .collect();
            let _ = update_bus_environment(&self.bus_address, variables);
        }
        self.cancelled.store(true, Ordering::Relaxed);
        // this dropping the worker restarts systemd units in their og env
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(bus) = &mut self.private_bus {
            let _ = bus.kill();
            let _ = bus.wait();
        }
        if let Some(config) = &self.portal_config {
            let _ = fs::remove_file(config);
        }
        if let Some(runtime) = &self.runtime_dir {
            let _ = fs::remove_dir_all(runtime);
        }
    }
}

fn desktop_in_environment() -> bool {
    const DESKTOPS: &[&str] = &[
        "gnome", "kde", "plasma", "xfce", "mate", "cinnamon", "lxqt", "budgie", "cosmic", "niri",
        "sway", "hyprland", "deepin", "unity", "pantheon",
    ];
    env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| {
        desktop
            .split(':')
            .any(|name| DESKTOPS.contains(&name.to_ascii_lowercase().as_str()))
    })
}

fn name_has_owner(connection: &Connection, name: &str) -> bool {
    let proxy = connection.with_proxy(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        Duration::from_millis(700),
    );
    let result: Result<(bool,), dbus::Error> =
        proxy.method_call("org.freedesktop.DBus", "NameHasOwner", (name,));
    result.is_ok_and(|(owned,)| owned)
}

fn wait_for_owner(connection: &Connection, name: &str, cancelled: &AtomicBool) -> bool {
    log::info!("Waiting for {name} to be disowned...");
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return false;
        }
        if name_has_owner(connection, name) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(80));
    }
}

fn wait_for_unit(unit: &str, cancelled: &AtomicBool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return false;
        }
        if systemctl(&["is-active", "--quiet", unit]) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(80));
    }
}

fn existing_session_bus() -> Option<(Connection, String)> {
    let mut addresses = Vec::new();
    if let Some(address) = env::var_os("DBUS_SESSION_BUS_ADDRESS") {
        addresses.push(address.to_string_lossy().into_owned());
    }
    let runtime_bus = original_runtime_dir().join("bus");
    if runtime_bus.exists() {
        addresses.push(format!("unix:path={}", runtime_bus.display()));
    }
    for address in addresses {
        if let Ok(connection) = Connection::new_address(&address) {
            return Some((connection, address));
        }
    }
    None
}

fn original_runtime_dir() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from)
}

fn start_private_bus() -> anyhow::Result<(String, Child, PathBuf)> {
    let runtime = original_runtime_dir().join(format!("wayvr-dbus-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&runtime).with_context(|| format!("Creating {}", runtime.display()))?;
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))?;
    let socket = runtime.join("bus");
    let mut command = Command::new("dbus-daemon");
    command.args(["--session", "--nofork", "--print-address=1"]);
    command.arg(format!("--address=unix:path={}", socket.display()));
    command.stdin(Stdio::null()).stdout(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            let _ = fs::remove_dir_all(&runtime);
            return Err(err).context("Starting private D-Bus session");
        }
    };
    let mut address = String::new();
    let result = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("No D-Bus address pipe"))
        .and_then(|stdout| {
            BufReader::new(stdout)
                .read_line(&mut address)
                .map_err(Into::into)
        });
    if result.is_err() || address.trim().is_empty() {
        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_dir_all(&runtime);
        anyhow::bail!("Private D-Bus failed");
    }
    Ok((address.trim().to_string(), child, runtime))
}

fn user_systemd_available() -> bool {
    systemctl(&["show-environment"])
}

fn systemctl(args: &[&str]) -> bool {
    Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn read_systemd_environment() -> anyhow::Result<HashMap<String, Option<String>>> {
    let output = Command::new("systemctl")
        .args(["--user", "show-environment"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("Could not read systemd activation environment");
    }
    let text = String::from_utf8(output.stdout)?;
    let mut saved = ACTIVATION_VARS
        .iter()
        .map(|name| ((*name).to_string(), None))
        .collect::<HashMap<_, _>>();
    for line in text.lines() {
        if let Some((name, value)) = line.split_once('=')
            && let Some(slot) = saved.get_mut(name)
        {
            *slot = Some(value.to_string());
        }
    }
    Ok(saved)
}

fn restore_systemd_environment(saved: &HashMap<String, Option<String>>, address: &str) {
    let mut update = Command::new("dbus-update-activation-environment");
    update
        .arg("--systemd")
        .env("DBUS_SESSION_BUS_ADDRESS", address);
    for (name, value) in saved {
        if let Some(value) = value {
            update.arg(format!("{name}={value}"));
        } else {
            let _ = systemctl(&["unset-environment", name]);
        }
    }
    let _ = update.stdout(Stdio::null()).stderr(Stdio::null()).status();
}

fn update_bus_environment(address: &str, variables: HashMap<String, String>) -> anyhow::Result<()> {
    let connection = Connection::new_address(address)?;
    connection
        .with_proxy(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            Duration::from_secs(2),
        )
        .method_call::<(), _, _, _>(
            "org.freedesktop.DBus",
            "UpdateActivationEnvironment",
            (variables,),
        )?;
    Ok(())
}

fn polkit_available() -> bool {
    Connection::new_system().is_ok()
}

fn paths_to_scan() -> Vec<PathBuf> {
    let mut roots = Vec::with_capacity(64);

    let path_var = env::var_os("PATH").unwrap_or_default();
    roots.extend(env::split_paths(&path_var));
    roots.extend([
        PathBuf::from("/usr/libexec"),
        PathBuf::from("/usr/local/libexec"),
        PathBuf::from("/usr/lib"),
        PathBuf::from("/usr/lib64"),
        PathBuf::from("/usr/local/lib"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/lib/polkit-gnome"),
        PathBuf::from("/usr/libexec/polkit-gnome"),
        PathBuf::from("/usr/libexec/kf6"),
        PathBuf::from("/usr/lib/kf6"),
    ]);
    if let Ok(entries) = fs::read_dir("/usr/lib") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && entry.file_name().to_string_lossy().contains("-linux-gnu") {
                roots.push(path.join("libexec"));
            }
        }
    }
    roots
}

fn find_executable(paths_to_scan: &[PathBuf], executable: &str) -> Option<PathBuf> {
    paths_to_scan
        .iter()
        .map(|dir| dir.join(OsStr::new(executable)))
        .find(|path| {
            path.metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}
