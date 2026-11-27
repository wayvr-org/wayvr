use crate::subsystem::audio::types::APCInner;
use pipewire::registry::GlobalObject;
use pipewire::spa::utils::dict::{DictRef, ParsableValue};
use pipewire::types::ObjectType;

mod device;
mod metadata;
mod node;

impl APCInner {
    pub(super) fn on_global(&mut self, obj: &GlobalObject<&DictRef>) {
        let serial = obj
            .props
            .and_then(|props| props.get("object.serial"))
            .and_then(u64::parse_value)
            .unwrap_or_default(); // Could probably just unwrap, pretty sure object.serial is required
        self.max_serial = self.max_serial.max(serial);
        match obj.type_ {
            ObjectType::Node => self.on_node(obj, serial),
            ObjectType::Device => self.on_device(obj),
            ObjectType::Metadata => self.on_metadata(obj),
            _ => {}
        }
    }
}
