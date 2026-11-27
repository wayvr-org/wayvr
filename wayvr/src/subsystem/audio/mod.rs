mod connection;
pub mod listeners;
pub mod pod;
pub mod snapshot;
pub mod types;
pub mod update;

use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    thread::JoinHandle,
};

use crate::subsystem::audio::connection::AudioServiceLaunch;
use pipewire as pw;
use wlx_common::audio_devices::{AudioSnapshot, AudioTask};

pub const MAX_VOLUME: f32 = 1.5 * 1.5 * 1.5;

#[derive(Debug, Clone)]
pub enum AudioEvent {
    DefaultSinkVolume {
        description: String,
        volume: f32,
        mute: bool,
    },
}

enum Command {
    Task(AudioTask),
    Stop,
}

type SharedSnapshot = Arc<Mutex<Option<Arc<AudioSnapshot>>>>;

pub struct AudioService {
    commands: pw::channel::Sender<Command>,
    snapshot: SharedSnapshot,
    events: Receiver<AudioEvent>,
    thread: Option<JoinHandle<()>>,
}

impl AudioService {
    pub fn new() -> anyhow::Result<Self> {
        let (commands_tx, commands_rx) = pw::channel::channel();
        let (events_tx, events_rx) = mpsc::channel();
        let snapshot = SharedSnapshot::default();

        let launch = AudioServiceLaunch {
            commands: commands_rx,
            snapshot: snapshot.clone(),
            events: events_tx,
        };
        let thread = std::thread::Builder::new()
            .name("wayvr-audio-service".into())
            .spawn(move || launch.launch())?;

        Ok(Self {
            commands: commands_tx,
            snapshot,
            events: events_rx,
            thread: Some(thread),
        })
    }

    pub fn snapshot(&self) -> Option<Arc<AudioSnapshot>> {
        self.snapshot.lock().unwrap().clone()
    }

    pub fn task(&self, task: AudioTask) {
        if self.commands.send(Command::Task(task)).is_err() {
            log::warn!("Audio service is not running");
        }
    }

    pub fn poll_events(&self) -> impl Iterator<Item = AudioEvent> + '_ {
        self.events.try_iter()
    }
}

impl Drop for AudioService {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
