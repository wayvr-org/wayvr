use anyhow::anyhow;
use pipewire::spa::pod::deserialize::PodDeserializer;
use pipewire::spa::pod::serialize::PodSerializer;
use pipewire::spa::pod::{Object, Pod, Property, Value, ValueArray};
use pipewire::spa::sys::{
    SPA_PARAM_PROFILE_index, SPA_PARAM_PROFILE_save, SPA_PARAM_Profile, SPA_PARAM_Props,
    SPA_PARAM_ROUTE_device, SPA_PARAM_ROUTE_index, SPA_PARAM_ROUTE_props, SPA_PARAM_ROUTE_save,
    SPA_PARAM_Route, SPA_PROP_channelVolumes, SPA_PROP_mute, SPA_TYPE_OBJECT_ParamProfile,
    SPA_TYPE_OBJECT_ParamRoute, SPA_TYPE_OBJECT_Props,
};

pub(in crate::subsystem::audio) fn serialize_object(obj: Object) -> anyhow::Result<Vec<u8>> {
    Ok(
        PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &Value::Object(obj))?
            .0
            .into_inner(),
    )
}

pub(in crate::subsystem::audio) fn deserialize_object(bytes: &[u8]) -> anyhow::Result<Value> {
    Ok(PodDeserializer::deserialize_from(bytes)
        .map_err(|_| anyhow!("Failed to deserialize pod"))?
        .1)
}

pub(in crate::subsystem::audio) fn pod_from_bytes(bytes: &[u8]) -> anyhow::Result<&Pod> {
    Pod::from_bytes(bytes).ok_or_else(|| anyhow::anyhow!("invalid pod"))
}

pub fn audio_props(mute: Option<bool>, channel_volumes: Option<Vec<f32>>) -> Object {
    let mut properties = Vec::new();

    if let Some(mute) = mute {
        properties.push(Property::new(SPA_PROP_mute, Value::Bool(mute)));
    }
    if let Some(channel_volumes) = channel_volumes {
        properties.push(Property::new(
            SPA_PROP_channelVolumes,
            Value::ValueArray(ValueArray::Float(channel_volumes)),
        ));
    }

    Object {
        type_: SPA_TYPE_OBJECT_Props,
        id: SPA_PARAM_Props,
        properties,
    }
}

pub fn route_object(route_index: i32, route_device: i32, props: Object) -> Object {
    Object {
        type_: SPA_TYPE_OBJECT_ParamRoute,
        id: SPA_PARAM_Route,
        properties: vec![
            Property::new(SPA_PARAM_ROUTE_index, Value::Int(route_index)),
            Property::new(SPA_PARAM_ROUTE_device, Value::Int(route_device)),
            Property::new(SPA_PARAM_ROUTE_props, Value::Object(props)),
            Property::new(SPA_PARAM_ROUTE_save, Value::Bool(true)),
        ],
    }
}

pub fn profile_object(index: i32) -> Object {
    Object {
        type_: SPA_TYPE_OBJECT_ParamProfile,
        id: SPA_PARAM_Profile,
        properties: vec![
            Property::new(SPA_PARAM_PROFILE_index, Value::Int(index)),
            Property::new(SPA_PARAM_PROFILE_save, Value::Bool(true)),
        ],
    }
}

pub(super) trait ObjectExt {
    fn value(&self, key: u32) -> Option<&Value>;

    fn int(&self, key: u32) -> Option<&i32> {
        match self.value(key) {
            Some(Value::Int(i)) => Some(i),
            _ => None,
        }
    }
    fn string(&self, key: u32) -> Option<&String> {
        match self.value(key) {
            Some(Value::String(s)) => Some(s),
            _ => None,
        }
    }
    fn bool(&self, key: u32) -> Option<&bool> {
        match self.value(key) {
            Some(Value::Bool(b)) => Some(b),
            _ => None,
        }
    }
    fn array(&self, key: u32) -> Option<&ValueArray> {
        match self.value(key) {
            Some(Value::ValueArray(a)) => Some(a),
            _ => None,
        }
    }
    fn object(&self, key: u32) -> Option<&Object> {
        match self.value(key) {
            Some(Value::Object(obj)) => Some(obj),
            _ => None,
        }
    }
    fn struct_(&self, key: u32) -> Option<&Vec<Value>> {
        match self.value(key) {
            Some(Value::Struct(str)) => Some(str),
            _ => None,
        }
    }
}
impl ObjectExt for Object {
    fn value(&self, key: u32) -> Option<&Value> {
        Some(&self.properties.iter().find(|p| p.key == key)?.value)
    }
}

pub(super) trait ValueArrayExt {
    fn as_int(&self) -> Option<&Vec<i32>>;
    fn as_float(&self) -> Option<&Vec<f32>>;
}
impl ValueArrayExt for ValueArray {
    fn as_int(&self) -> Option<&Vec<i32>> {
        match self {
            ValueArray::Int(a) => Some(a),
            _ => None,
        }
    }
    fn as_float(&self) -> Option<&Vec<f32>> {
        match self {
            ValueArray::Float(a) => Some(a),
            _ => None,
        }
    }
}
