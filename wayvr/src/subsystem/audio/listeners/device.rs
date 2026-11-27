use crate::subsystem::audio::pod;
use crate::subsystem::audio::pod::{ObjectExt, ValueArrayExt};
use crate::subsystem::audio::types::{APCActiveRoute, APCDevice, APCDeviceProperties, APCInner};
use pipewire::device::{Device, DeviceChangeMask, DeviceInfoRef};
use pipewire::registry::GlobalObject;
use pipewire::spa::param::ParamType;
use pipewire::spa::pod::{Object, Pod, Value};
use pipewire::spa::sys::{
    SPA_PARAM_PROFILE_classes, SPA_PARAM_PROFILE_description, SPA_PARAM_PROFILE_index,
    SPA_PARAM_PROFILE_name, SPA_PARAM_PROFILE_priority, SPA_PARAM_ROUTE_description,
    SPA_PARAM_ROUTE_device, SPA_PARAM_ROUTE_devices, SPA_PARAM_ROUTE_index, SPA_PARAM_ROUTE_info,
    SPA_PARAM_ROUTE_name, SPA_PARAM_ROUTE_profiles, SPA_PARAM_ROUTE_props, SPA_PROP_channelVolumes,
};
use pipewire::spa::utils::dict::DictRef;
use wlx_common::audio_devices::{AudioDeviceProfile, AudioDeviceRoute};

impl APCInner {
    pub fn on_device(&mut self, obj: &GlobalObject<&DictRef>) {
        if !obj.props.is_some_and(|props| {
            props
                .get("media.class")
                .is_some_and(|class| matches!(class, "Audio/Device"))
        }) {
            return;
        }

        let Ok(device) = self.registry.bind::<Device, &DictRef>(obj) else {
            return;
        };
        let device_id = obj.id;

        let info_state = self.new_outer();
        let param_state = self.new_outer();
        let listener = device
            .add_listener_local()
            .info(move |info| info_state.run(|s| s.on_device_info(device_id, info)))
            .param(move |_seq, param_type, _index, _next, param| {
                param_state.run(|s| s.on_device_param(device_id, param_type, param));
            })
            .register();
        self.devices.retain(|d| d.id != obj.id);
        self.devices.push(APCDevice {
            id: obj.id,
            device,
            _listener: listener,
            active: vec![],
            routes: vec![],
            profiles: vec![],
            active_profile: None,
            properties: None,
        });
    }

    fn on_device_info(&mut self, device_id: u32, info: &DeviceInfoRef) {
        let Some(device) = self.devices.iter_mut().find(|d| d.id == device_id) else {
            return;
        };

        if info.change_mask().contains(DeviceChangeMask::PARAMS) {
            for param in [
                ParamType::EnumRoute,
                ParamType::Route,
                ParamType::EnumProfile,
                ParamType::Profile,
            ] {
                device.device.enum_params(0, Some(param), 0, u32::MAX);
            }
        }
        let Some(props) = info.props() else {
            return;
        };

        let Some(description) = props.get("device.description") else {
            return;
        };
        let Some(name) = props.get("device.name") else {
            return;
        };
        let nick = props.get("device.nick").map(ToString::to_string);

        device.properties = Some(APCDeviceProperties {
            description: description.to_owned(),
            name: name.to_owned(),
            nick,
        });
        self.publish_snapshot();
    }

    fn on_device_param(&mut self, device_id: u32, param_type: ParamType, param: Option<&Pod>) {
        if !matches!(
            param_type,
            ParamType::EnumRoute | ParamType::Route | ParamType::EnumProfile | ParamType::Profile
        ) {
            return;
        }
        let Some(param) = param else {
            return;
        };
        let Ok(Value::Object(obj)) = pod::deserialize_object(param.as_bytes()) else {
            return;
        };

        let Some(device) = self.devices.iter_mut().find(|d| d.id == device_id) else {
            return;
        };

        match param_type {
            ParamType::EnumRoute => {
                if let Some(route) = parse_device_route(&obj) {
                    device.routes.retain(|r| r.index != route.index);
                    device.routes.push(route);
                    self.publish_snapshot();
                }
            }
            ParamType::Route => {
                if let Some(route) = parse_active_route(&obj) {
                    device.active.retain(|r| r.device != route.device);
                    device.active.push(route);
                    self.publish_snapshot();
                }
            }
            ParamType::EnumProfile => {
                if let Some(profile) = parse_device_profile(&obj) {
                    device.profiles.retain(|p| p.index != profile.index);
                    device.profiles.push(profile);
                    self.publish_snapshot();
                }
            }
            ParamType::Profile => {
                if let Some(&index) = obj.int(SPA_PARAM_PROFILE_index) {
                    device.active_profile = Some(index);
                    self.publish_snapshot();
                }
            }
            _ => unreachable!(),
        }
    }
}

fn parse_device_route(obj: &Object) -> Option<AudioDeviceRoute> {
    let index = *obj.int(SPA_PARAM_ROUTE_index)?;
    let name = obj.string(SPA_PARAM_ROUTE_name)?.clone();
    let description = obj.string(SPA_PARAM_ROUTE_description)?.clone();
    let device_ids = obj.array(SPA_PARAM_ROUTE_devices)?.as_int()?.clone();
    let profile_ids = obj.array(SPA_PARAM_ROUTE_profiles)?.as_int()?.clone();
    let product_name = obj.struct_(SPA_PARAM_ROUTE_info).and_then(|info| {
        let mut iter = info.iter().skip(1);
        while let Some(Value::String(key)) = iter.next() {
            if key == "device.product.name" {
                let Some(Value::String(value)) = iter.next() else {
                    continue;
                };
                return Some(value.clone());
            }
            // Skip unwanted values
            iter.next();
        }
        None
    });
    Some(AudioDeviceRoute {
        index,
        name,
        description,
        device_ids,
        profile_ids,
        product_name,
    })
}

fn parse_active_route(obj: &Object) -> Option<APCActiveRoute> {
    let index = *obj.int(SPA_PARAM_ROUTE_index)?;
    let device_id = *obj.int(SPA_PARAM_ROUTE_device)?;
    let channel_volumes = obj
        .object(SPA_PARAM_ROUTE_props)?
        .array(SPA_PROP_channelVolumes)?
        .as_float()?
        .clone();
    Some(APCActiveRoute {
        device: device_id,
        index,
        channel_volumes,
    })
}

fn parse_device_profile(obj: &Object) -> Option<AudioDeviceProfile> {
    let index = *obj.int(SPA_PARAM_PROFILE_index)?;
    let name = obj.string(SPA_PARAM_PROFILE_name)?.clone();
    let description = obj.string(SPA_PARAM_PROFILE_description)?.clone();
    let priority = *obj.int(SPA_PARAM_PROFILE_priority)?;
    let classes: Vec<(String, i32)> = obj
        .struct_(SPA_PARAM_PROFILE_classes)?
        .iter()
        .skip(1)
        .filter_map(|c| match c {
            Value::Struct(s) => Some(s),
            _ => None,
        })
        .filter_map(|c| {
            let Value::String(class) = c.first()? else {
                return None;
            };
            let Value::Int(count) = c.get(1)? else {
                return None;
            };
            Some((class.clone(), *count))
        })
        .collect();

    let class_count = |class: &str| {
        classes
            .iter()
            .find_map(|(c, i)| if c == class { Some(*i) } else { None })
            .unwrap_or(0)
    };

    let sources = class_count("Audio/Source");
    let sinks = class_count("Audio/Sink");

    Some(AudioDeviceProfile {
        index,
        name,
        description,
        priority,
        source_count: sources as usize,
        sink_count: sinks as usize,
    })
}
