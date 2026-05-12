use std::{
    cmp,
    collections::{BinaryHeap, VecDeque},
    sync::atomic::{self, AtomicUsize},
    time::Instant,
};

use serde::Deserialize;

use crate::{
    backend::input,
    state::AppState,
    windowing::{OverlaySelector, window::OverlayWindowConfig},
};

static TASK_AUTO_INCREMENT: AtomicUsize = AtomicUsize::new(0);

struct AppTask {
    pub not_before: Instant,
    pub id: usize,
    pub task: TaskType,
}

impl PartialEq<Self> for AppTask {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == cmp::Ordering::Equal
    }
}
impl PartialOrd<Self> for AppTask {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Eq for AppTask {}
impl Ord for AppTask {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.not_before
            .cmp(&other.not_before)
            .then(self.id.cmp(&other.id))
            .reverse()
    }
}

pub enum InputTask {
    Haptics {
        device: usize,
        haptics: input::Haptics,
    },
}

#[cfg(feature = "openvr")]
#[allow(dead_code)]
pub enum OpenVrTask {
    ColorGain(ColorChannel, f32),
}

#[cfg(feature = "openxr")]
pub enum OpenXrTask {
    EnvironmentChanged,
}

pub enum PlayspaceTask {
    Recenter,
    Reset,
    FixFloor,
}

#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
#[allow(dead_code)]
pub enum ModifyPanelCommand {
    SetText(String),
    SetColor(String),
    SetImage(String),
    SetVisible(bool),
    GetValue,
    SetValue(String),
    SetStickyState(bool),
}

#[derive(Debug, Clone)]
pub struct ModifyPanelTask {
    pub overlay: String,
    pub element: String,
    pub command: ModifyPanelCommand,
}

pub enum ToggleMode {
    EnsureOn,
    EnsureOff,
    Toggle,
}

pub type ModifyOverlayTask = dyn FnOnce(&mut AppState, &mut OverlayWindowConfig) + Send;
pub type CreateOverlayTask = dyn FnOnce(&mut AppState) -> Option<OverlayWindowConfig> + Send;
pub enum OverlayTask {
    AddSet,
    ToggleSet(usize),
    SwitchSet(Option<usize>),
    ToggleOverlay(OverlaySelector, ToggleMode),
    ResetOverlay(OverlaySelector),
    DeleteActiveSet,
    ToggleEditMode,
    ToggleDashboard,
    ShowHide,
    CleanupMirrors,
    SettingsChanged,
    KeyboardChanged,
    Modify(OverlaySelector, Box<ModifyOverlayTask>),
    Create(OverlaySelector, Box<CreateOverlayTask>),
    ModifyPanel(ModifyPanelTask),
    Drop(OverlaySelector),
}

#[allow(dead_code)]
pub enum TaskType {
    Input(InputTask),
    Overlay(OverlayTask),
    Playspace(PlayspaceTask),
    #[cfg(feature = "openvr")]
    OpenVR(OpenVrTask),
    #[cfg(feature = "openxr")]
    OpenXR(OpenXrTask),
}

#[derive(Deserialize, Clone, Copy)]
pub enum ColorChannel {
    R,
    G,
    B,
    All,
}

pub struct TaskContainer {
    tasks: BinaryHeap<AppTask>,
}

impl TaskContainer {
    pub const fn new() -> Self {
        Self {
            tasks: BinaryHeap::new(),
        }
    }

    pub fn transfer_from(&mut self, other: &mut Self) {
        self.tasks.append(&mut other.tasks);
    }

    pub fn enqueue(&mut self, task: TaskType) {
        self.tasks.push(AppTask {
            not_before: Instant::now(),
            id: TASK_AUTO_INCREMENT.fetch_add(1, atomic::Ordering::Relaxed),
            task,
        });
    }

    /// Enqueue a task to be executed at a specific time.
    /// If the time is in the past, the task will be executed immediately.
    /// Multiple tasks enqueued for the same instant will be executed in order of submission.
    pub fn enqueue_at(&mut self, task: TaskType, not_before: Instant) {
        self.tasks.push(AppTask {
            not_before,
            id: TASK_AUTO_INCREMENT.fetch_add(1, atomic::Ordering::Relaxed),
            task,
        });
    }

    pub fn retrieve_due(&mut self, dest_buf: &mut VecDeque<TaskType>) {
        let now = Instant::now();

        while let Some(task) = self.tasks.peek() {
            if task.not_before > now {
                break;
            }

            // Safe unwrap because we peeked.
            dest_buf.push_back(self.tasks.pop().unwrap().task);
        }
    }
}
