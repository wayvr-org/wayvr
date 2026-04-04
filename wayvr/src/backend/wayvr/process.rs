use std::{collections::HashMap, io::Read, sync::Arc};

use wayvr_ipc::{packet_client, packet_server};

use crate::gen_id;

#[derive(Debug)]
#[allow(dead_code)]
pub struct WayVRProcess {
    pub auth_key: String,
    pub child: std::process::Child,
    pub app_name: String,
    pub exec_path: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub working_dir: Option<String>,
    pub resolution: [u32; 2],
    pub pos_mode: packet_client::PositionMode,
    pub icon: Option<Arc<str>>,

    pub userdata: HashMap<String, String>,
}

#[derive(Debug)]
pub struct ExternalProcess {
    pub pid: u32,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Process {
    Managed(WayVRProcess),     // Process spawned by WayVR
    External(ExternalProcess), // External process not directly controlled by us
}

#[derive(Clone, Copy)]
pub enum KillSignal {
    Term,
    Kill,
}

impl Process {
    pub fn is_running(&mut self) -> bool {
        match self {
            Self::Managed(p) => p.is_running(),
            Self::External(p) => p.is_running(),
        }
    }

    pub fn kill(&mut self, signal: KillSignal) {
        let signal = match signal {
            KillSignal::Term => libc::SIGTERM,
            KillSignal::Kill => libc::SIGKILL,
        };

        match self {
            Self::Managed(p) => p.kill(signal),
            Self::External(p) => p.kill(signal),
        }
    }

    #[allow(dead_code)]
    pub fn get_name(&self) -> String {
        match self {
            Self::Managed(p) => p
                .get_name()
                .or_else(|| p.exec_path.split('/').next_back().map(String::from))
                .unwrap_or_else(|| String::from("unknown")),
            Self::External(p) => p.get_name().unwrap_or_else(|| String::from("unknown")),
        }
    }

    pub fn to_packet(&self, handle: ProcessHandle) -> packet_server::WvrProcess {
        match self {
            Self::Managed(p) => packet_server::WvrProcess {
                name: p.get_name().unwrap_or_else(|| String::from("unknown")),
                userdata: p.userdata.clone(),
                handle: handle.as_packet(),
            },
            Self::External(p) => packet_server::WvrProcess {
                name: p.get_name().unwrap_or_else(|| String::from("unknown")),
                userdata: HashMap::default(),
                handle: handle.as_packet(),
            },
        }
    }
}

impl Drop for WayVRProcess {
    fn drop(&mut self) {
        log::info!(
            "Sending SIGTERM (graceful exit) to process {} ({})",
            self.child.id(),
            self.exec_path.as_str()
        );
        self.kill(libc::SIGTERM);
    }
}

fn get_process_env_value(pid: i32, key: &str) -> anyhow::Result<Option<String>> {
    let path = format!("/proc/{pid}/environ");
    let mut env_data = String::new();
    std::fs::File::open(path)?.read_to_string(&mut env_data)?;
    let lines: Vec<&str> = env_data.split('\0').filter(|s| !s.is_empty()).collect();

    for line in lines {
        if let Some(cell) = line.split_once('=')
            && cell.0 == key
        {
            return Ok(Some(String::from(cell.1)));
        }
    }

    Ok(None)
}

impl WayVRProcess {
    fn is_running(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(Some(_exit_status)) => false,
            Ok(None) => true,
            Err(e) => {
                // this shouldn't happen
                log::error!("Child::try_wait failed: {e}");
                false
            }
        }
    }

    fn kill(&mut self, signal: i32) {
        unsafe {
            // Gracefully stop process
            libc::kill(self.child.id() as i32, signal);
        }
    }

    pub fn get_name(&self) -> Option<String> {
        get_exec_name_from_pid(self.child.id())
    }
}

fn get_exec_name_from_pid(pid: u32) -> Option<String> {
    let path = format!("/proc/{pid}/exe");
    match std::fs::read_link(&path) {
        Ok(buf) => {
            if let Some(process_name) = buf.file_name().and_then(|s| s.to_str()) {
                return Some(String::from(process_name));
            }
            None
        }
        Err(_) => None,
    }
}

impl ExternalProcess {
    fn is_running(&self) -> bool {
        if self.pid == 0 {
            false
        } else {
            std::fs::metadata(format!("/proc/{}", self.pid)).is_ok()
        }
    }

    fn kill(&mut self, signal: i32) {
        if self.pid != 0 {
            unsafe {
                // send SIGINT (^C)
                libc::kill(self.pid as i32, signal);
            }
        }
        self.pid = 0;
    }

    pub fn get_name(&self) -> Option<String> {
        get_exec_name_from_pid(self.pid)
    }
}

gen_id!(ProcessVec, Process, ProcessCell, ProcessHandle);

pub fn find_by_pid(processes: &ProcessVec, pid: u32) -> Option<ProcessHandle> {
    log::debug!("Finding process with PID {pid}");

    for (idx, cell) in processes.vec.iter().enumerate() {
        let Some(cell) = cell else {
            continue;
        };
        match &cell.obj {
            Process::Managed(wayvr_process) => {
                if wayvr_process.child.id() == pid {
                    return Some(ProcessVec::get_handle(cell, idx));
                }
            }
            Process::External(external_process) => {
                if external_process.pid == pid {
                    return Some(ProcessVec::get_handle(cell, idx));
                }
            }
        }
    }

    log::debug!("Finding by PID failed, trying WAYVR_DISPLAY_AUTH...");

    if let Ok(Some(value)) = get_process_env_value(pid as i32, "WAYVR_DISPLAY_AUTH") {
        for (idx, cell) in processes.vec.iter().enumerate() {
            let Some(cell) = cell else {
                continue;
            };
            if let Process::Managed(wayvr_process) = &cell.obj
                && wayvr_process.auth_key == value
            {
                return Some(ProcessVec::get_handle(cell, idx));
            }
        }
    }

    log::debug!("Process find with PID {pid} failed");
    None
}

impl ProcessHandle {
    pub const fn from_packet(handle: packet_server::WvrProcessHandle) -> Self {
        Self {
            generation: handle.generation,
            idx: handle.idx,
        }
    }

    pub const fn as_packet(&self) -> packet_server::WvrProcessHandle {
        packet_server::WvrProcessHandle {
            idx: self.idx,
            generation: self.generation,
        }
    }
}
