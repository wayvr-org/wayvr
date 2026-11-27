use crate::subsystem::audio::pod;
use crate::subsystem::audio::pod::profile_object;
use crate::subsystem::audio::types::{APCInner, PendingDefaultSink};
use anyhow::anyhow;
use pipewire::spa::param::ParamType;
use std::time::{Duration, Instant};
use wlx_common::audio_devices::{AudioNodeType, AudioTask};

impl APCInner {
    pub(super) fn handle_task(&mut self, task: AudioTask) {
        match task {
            AudioTask::SetDefaultNode(node_type, index) => {
                if node_type == AudioNodeType::Sink {
                    self.pending_default_sink = None;
                }
                self.set_default(node_type, index).ok();
            }
            AudioTask::SetVolume(index, v) => {
                self.update_node_props(index, None, Some(v * v * v)).ok();
            }
            AudioTask::SetMute(index, muted) => {
                self.update_node_props(index, Some(muted), None).ok();
            }
            AudioTask::AddDefaultSinkVolume(volume_delta) => {
                self.update_default_sink_volume(volume_delta).ok();
            }
            AudioTask::SetCardProfile {
                card: device_id,
                profile,
                make_default_sink,
            } => self.set_card_profile(device_id, profile, make_default_sink),
        }
    }

    fn set_card_profile(&mut self, device_id: u32, profile: i32, make_default_sink: bool) {
        let Some(device) = self.devices.iter().find(|d| d.id == device_id) else {
            return;
        };

        if device.active_profile.is_some_and(|p| p == profile) {
            let Some(node) = self.nodes.iter().find(|n| {
                n.node_type == AudioNodeType::Sink
                    && n.properties
                        .as_ref()
                        .and_then(|p| p.device_id)
                        .is_some_and(|d| d == device_id)
            }) else {
                return;
            };
            let node_id = node.id;
            if make_default_sink {
                self.set_default(AudioNodeType::Sink, node_id).ok();
            }
            return;
        }

        if make_default_sink {
            self.pending_default_sink = Some(PendingDefaultSink {
                device_id,
                after_serial: self.max_serial,
                deadline: Instant::now() + Duration::from_secs(5),
            });
        }
        self.set_profile(device_id, profile).ok();
    }

    fn update_default_sink_volume(&self, delta_volume_cbrt: f32) -> anyhow::Result<()> {
        let default_sink_name = self
            .default_sink
            .clone()
            .ok_or_else(|| anyhow!("No default sink"))?;
        let default_sink = self
            .nodes
            .iter()
            .find(|n| {
                n.properties
                    .as_ref()
                    .is_some_and(|p| p.name == default_sink_name)
            })
            .ok_or_else(|| anyhow!("Could not find node for default sink"))?;
        let index = default_sink.id;
        let current_volume = default_sink
            .volume()
            .ok_or_else(|| anyhow!("Could not resolve current volume"))?;
        let v = current_volume.cbrt() + delta_volume_cbrt;
        let volume = v * v * v;
        self.update_node_props(index, None, Some(volume))
    }

    fn update_node_props(
        &self,
        node_index: u32,
        mute: Option<bool>,
        volume: Option<f32>,
    ) -> anyhow::Result<()> {
        // Nothing to do
        if mute.is_none() && volume.is_none() {
            return Ok(());
        }
        let volume = volume.map(|v| v.clamp(0., super::MAX_VOLUME));

        let node = self
            .nodes
            .iter()
            .find(|n| n.id == node_index)
            .ok_or_else(|| anyhow!("Could not find node to update"))?;

        let node_props = node
            .properties
            .as_ref()
            .ok_or_else(|| anyhow!("Node properties not loaded"))?;
        let device_id = node_props.device_id;
        let route_device = node_props.route_device;
        if let (Some(device_id), Some(route_device)) = (device_id, route_device) {
            let device = self
                .devices
                .iter()
                .find(|d| d.id == device_id)
                .ok_or_else(|| anyhow!("Could not find device associated with node"))?;
            let route = device
                .active
                .iter()
                .find(|r| r.device == route_device)
                .ok_or_else(|| anyhow!("Device does not have route requested by node"))?;

            let bytes = pod::serialize_object(pod::route_object(
                route.index,
                route_device,
                pod::audio_props(mute, volume.map(|v| vec![v; route.channel_volumes.len()])),
            ))?;
            let pod = pod::pod_from_bytes(bytes.as_ref())?;
            device.device.set_param(ParamType::Route, 0, pod);
        } else {
            let bytes = pod::serialize_object(pod::audio_props(
                mute,
                volume.map(|v| vec![v; node.last_volumes.as_ref().map_or(0, Vec::len)]),
            ))?;
            let pod = pod::pod_from_bytes(bytes.as_ref())?;
            node.node.set_param(ParamType::Props, 0, pod);
        }
        Ok(())
    }

    fn set_profile(&self, device_id: u32, profile: i32) -> anyhow::Result<()> {
        let device = self
            .devices
            .iter()
            .find(|d| d.id == device_id)
            .ok_or_else(|| anyhow!("Could not find device to update"))?;
        let bytes = pod::serialize_object(profile_object(profile))?;
        let pod = pod::pod_from_bytes(bytes.as_ref())?;
        device.device.set_param(ParamType::Profile, 0, pod);
        Ok(())
    }

    pub(super) fn set_default(
        &self,
        node_type: AudioNodeType,
        node_index: u32,
    ) -> anyhow::Result<()> {
        let node = self
            .nodes
            .iter()
            .find(|n| n.id == node_index)
            .ok_or_else(|| anyhow!("Could not find node to update"))?;
        let node_name = node
            .properties
            .as_ref()
            .ok_or_else(|| anyhow!("Node properties not loaded"))?
            .name
            .clone();

        let metadata = self
            .metadata
            .as_ref()
            .ok_or_else(|| anyhow!("Metadata was not loaded, is wireplumber running?"))?;

        let key = match node_type {
            AudioNodeType::Source => "default.configured.audio.source",
            AudioNodeType::Sink => "default.configured.audio.sink",
        };

        let value = serde_json::json!({ "name": node_name }).to_string();
        metadata.set_property(0, key, Some("Spa:String:JSON"), Some(&value));

        Ok(())
    }
}
