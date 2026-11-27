use crate::subsystem::audio::types::{APCInner, APCNode};
use anyhow::anyhow;
use std::sync::Arc;
use wlx_common::audio_devices::{AudioDevice, AudioNode, AudioSnapshot};

impl TryFrom<&APCNode> for AudioNode {
    type Error = anyhow::Error;

    fn try_from(n: &APCNode) -> Result<Self, Self::Error> {
        let props = n
            .properties
            .as_ref()
            .ok_or_else(|| anyhow!("No node properties"))?;
        let volume = n.volume().unwrap_or(0.).cbrt();
        Ok(AudioNode {
            index: n.id,
            node_type: n.node_type,
            name: props.name.clone(),
            nick: props.nick.clone(),
            description: props.description.clone(),
            mute: n.last_mute.unwrap_or(false),
            volume,
            device_id: props.device_id,
        })
    }
}

impl APCInner {
    fn build_snapshot(&self) -> AudioSnapshot {
        let nodes = self
            .nodes
            .iter()
            .filter_map(|n| n.try_into().ok())
            .collect();

        let devices = self
            .devices
            .iter()
            .filter_map(|d| {
                let properties = d.properties.as_ref()?;
                Some(AudioDevice {
                    index: d.id,
                    description: properties.description.clone(),
                    name: properties.name.clone(),
                    nick: properties.nick.clone(),
                    profiles: d.profiles.clone(),
                    active_profile_index: d.active_profile,
                    routes: d.routes.clone(),
                })
            })
            .collect();
        AudioSnapshot {
            nodes,
            devices,
            default_sink: self.default_sink.clone(),
            default_source: self.default_source.clone(),
            layout_revision: 0,
        }
    }

    pub(super) fn publish_snapshot(&self) {
        let mut new_snapshot = self.build_snapshot();
        let mut snapshot = self.output.snapshot.lock().unwrap();
        if let Some(existing) = &*snapshot {
            new_snapshot.layout_revision = existing.layout_revision;
            if !new_snapshot.same_layout(existing) {
                new_snapshot.layout_revision += 1;
            }
        }
        snapshot.replace(Arc::new(new_snapshot));
    }
}
