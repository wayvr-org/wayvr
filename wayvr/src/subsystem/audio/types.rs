use crate::subsystem::audio::{AudioEvent, SharedSnapshot};
use pipewire::device::{Device, DeviceListener};
use pipewire::metadata::{Metadata, MetadataListener};
use pipewire::node::{Node, NodeListener};
use pipewire::registry::{Listener, RegistryRc};
use std::cell::RefCell;
use std::rc;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Instant;
use wlx_common::audio_devices::{
    AudioDeviceProfile, AudioDeviceRoute, AudioNodeType, AudioSnapshot,
};

#[derive(Clone)]
pub(super) struct Output {
    pub(super) snapshot: SharedSnapshot,
    pub(super) events: Sender<AudioEvent>,
}
impl Output {
    pub(super) fn publish_empty(&self) {
        let mut snapshot = self.snapshot.lock().unwrap();
        if let Some(layout_revision) = snapshot.as_ref().map(|s| s.layout_revision) {
            snapshot.replace(Arc::new(AudioSnapshot {
                nodes: vec![],
                devices: vec![],
                default_sink: None,
                default_source: None,
                layout_revision: layout_revision + 1,
            }));
        }
    }
}

pub struct APCInner {
    pub(super) weak: rc::Weak<RefCell<APCInner>>,
    pub(super) output: Output,
    pub(super) registry: RegistryRc,
    pub(super) global_listener: Option<Listener>,
    pub(super) pending_default_sink: Option<PendingDefaultSink>,
    pub(super) max_serial: u64,
    pub(super) metadata_id: Option<u32>,
    pub(super) metadata_listener: Option<MetadataListener>,
    pub(super) metadata: Option<Metadata>,
    pub(super) devices: Vec<APCDevice>,
    pub(super) nodes: Vec<APCNode>,
    pub(super) default_source: Option<String>,
    pub(super) default_sink: Option<String>,
}
impl APCInner {
    pub(super) fn new_outer(&self) -> AudioPipewireState {
        AudioPipewireState(self.weak.clone())
    }
}

pub struct PendingDefaultSink {
    pub device_id: u32,
    pub after_serial: u64,
    pub deadline: Instant,
}

pub struct APCDevice {
    pub id: u32,
    pub _listener: DeviceListener,
    pub device: Device,
    pub active: Vec<APCActiveRoute>,
    pub routes: Vec<AudioDeviceRoute>,
    pub profiles: Vec<AudioDeviceProfile>,
    pub active_profile: Option<i32>,
    pub properties: Option<APCDeviceProperties>,
}

#[derive(Debug)]
pub struct APCDeviceProperties {
    pub description: String,
    pub name: String,
    pub nick: Option<String>,
}

#[derive(Debug)]
pub struct APCActiveRoute {
    pub index: i32,
    pub device: i32,
    pub channel_volumes: Vec<f32>,
}

pub struct APCNode {
    pub(super) id: u32,
    pub(super) serial: u64,
    pub(super) _listener: NodeListener,
    pub(super) node: Node,
    pub(super) node_type: AudioNodeType,
    pub(super) last_mute: Option<bool>,
    pub(super) last_volumes: Option<Vec<f32>>,
    pub(super) properties: Option<APCNodeProperties>,
}

impl APCNode {
    pub fn volume(&self) -> Option<f32> {
        self.last_volumes
            .as_ref()
            .map(|v| v.iter().sum::<f32>() / v.len() as f32)
    }
}

#[derive(Debug)]
pub struct APCNodeProperties {
    pub name: String,
    pub description: String,
    pub nick: Option<String>,
    pub device_id: Option<u32>,
    pub route_device: Option<i32>,
}

pub struct AudioPipewireOwner(pub(super) Rc<RefCell<APCInner>>);

#[derive(Clone)]
pub struct AudioPipewireState(rc::Weak<RefCell<APCInner>>);
impl AudioPipewireState {
    pub(super) fn run(&self, fun: impl FnOnce(&mut APCInner)) {
        if let Some(inner) = self.0.upgrade() {
            fun(&mut inner.borrow_mut());
        }
    }
}
