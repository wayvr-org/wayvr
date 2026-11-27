use crate::subsystem::audio::pod::{ObjectExt, ValueArrayExt};
use crate::subsystem::audio::types::{APCInner, APCNode, APCNodeProperties};
use crate::subsystem::audio::{AudioEvent, pod};
use pipewire::node::{Node, NodeInfoRef};
use pipewire::registry::GlobalObject;
use pipewire::spa::param::ParamType;
use pipewire::spa::pod::{Pod, Value};
use pipewire::spa::sys::{SPA_PROP_channelVolumes, SPA_PROP_mute};
use pipewire::spa::utils::dict::DictRef;
use std::time::Instant;
use wlx_common::audio_devices::AudioNodeType;

impl APCInner {
    pub fn on_node(&mut self, obj: &GlobalObject<&DictRef>, serial: u64) {
        let Some(props) = obj.props else {
            return;
        };
        let node_type = match props.get("media.class") {
            Some("Audio/Sink") => AudioNodeType::Sink,
            Some("Audio/Source") => AudioNodeType::Source,
            _ => return,
        };

        let Ok(node) = self.registry.bind::<Node, &DictRef>(obj) else {
            return;
        };
        let node_id = obj.id;

        let info_state = self.new_outer();
        let param_state = self.new_outer();
        let listener = node
            .add_listener_local()
            .info(move |info| info_state.run(|s| s.on_node_info(node_id, info)))
            .param(move |_seq, param_type, _index, _next, param| {
                param_state.run(|s| s.on_node_param(node_id, param_type, param));
            })
            .register();
        node.subscribe_params(&[ParamType::Props]);
        self.nodes.retain(|n| n.id != obj.id);
        self.nodes.push(APCNode {
            id: obj.id,
            serial,
            node,
            node_type,
            _listener: listener,
            last_mute: None,
            last_volumes: None,
            properties: None,
        });
    }

    fn on_node_info(&mut self, node_id: u32, info: &NodeInfoRef) {
        let Some(props) = info.props() else {
            return;
        };
        let Some(node) = self.nodes.iter_mut().find(|n| n.id == node_id) else {
            return;
        };

        let Some(name) = props.get("node.name") else {
            return;
        };
        let Some(description) = props.get("node.description") else {
            return;
        };
        let nick = props.get("node.nick").map(ToString::to_string);
        let device_id = props.get("device.id").and_then(|s| s.parse().ok());
        let route_device = props
            .get("card.profile.device")
            .and_then(|s| s.parse().ok());
        node.properties = Some(APCNodeProperties {
            name: name.to_string(),
            description: description.to_string(),
            nick,
            device_id,
            route_device,
        });
        if node.node_type == AudioNodeType::Sink
            && let Some(pending) = &self.pending_default_sink
            && let Some(device_id) = device_id
        {
            if pending.deadline < Instant::now() {
                self.pending_default_sink = None;
                return;
            }
            if pending.device_id != device_id {
                return;
            }
            if node.serial <= pending.after_serial {
                return;
            }
            self.pending_default_sink = None;
            self.set_default(AudioNodeType::Sink, node_id).ok();
        }
    }

    fn on_node_param(&mut self, node_id: u32, param_type: ParamType, param: Option<&Pod>) {
        if param_type != ParamType::Props {
            return;
        }
        let Some(param) = param else {
            return;
        };

        let Some(n) = self.nodes.iter_mut().find(|n| n.id == node_id) else {
            return;
        };

        let Ok(Value::Object(obj)) = pod::deserialize_object(param.as_bytes()) else {
            return;
        };

        let new_mute = obj.bool(SPA_PROP_mute).copied();
        let new_volumes = obj
            .array(SPA_PROP_channelVolumes)
            .and_then(|v| v.as_float())
            .cloned();
        let is_first_info = n.last_mute.is_none() || n.last_volumes.is_none();
        if n.last_mute.is_some()
            && n.last_mute == new_mute
            && n.last_volumes.is_some()
            && n.last_volumes == new_volumes
        {
            return;
        }

        n.last_mute = new_mute.or(n.last_mute);
        n.last_volumes = new_volumes.or_else(|| n.last_volumes.take());
        let name = n.properties.as_ref().map_or_default(|p| p.name.clone());
        let description = n
            .properties
            .as_ref()
            .map_or_default(|p| p.nick.clone().unwrap_or_else(|| p.description.clone()));
        let volume = n.volume().unwrap_or_default().cbrt();
        let mute = n.last_mute.unwrap_or_default();
        if !is_first_info && self.default_sink.as_ref().is_some_and(|s| *s == name) {
            self.output
                .events
                .send(AudioEvent::DefaultSinkVolume {
                    description,
                    volume,
                    mute,
                })
                .ok();
        }
        self.publish_snapshot();
    }
}
