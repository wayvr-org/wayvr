use crate::subsystem::audio::types::APCInner;
use pipewire::metadata::Metadata;
use pipewire::registry::GlobalObject;
use pipewire::spa::utils::dict::DictRef;

impl APCInner {
    pub fn on_metadata(&mut self, obj: &GlobalObject<&DictRef>) {
        if !obj.props.is_some_and(|props| {
            props
                .get("metadata.name")
                .is_some_and(|metadata_name| matches!(metadata_name, "default"))
        }) {
            return;
        }

        let Ok(metadata) = self.registry.bind::<Metadata, &DictRef>(obj) else {
            return;
        };

        let state = self.new_outer();
        let listener = metadata
            .add_listener_local()
            .property(move |_subject, key, _type, value| {
                let Some(key) = key else {
                    return 0;
                };
                let Some(value) = value else {
                    return 0;
                };
                match key {
                    "default.audio.sink" | "default.audio.source" => {
                        let name = serde_json::from_str::<serde_json::Value>(value)
                            .ok()
                            .and_then(|j| j["name"].as_str().map(str::to_owned));
                        if name.is_none() {
                            return 0;
                        }
                        state.run(|guard| {
                            match key {
                                "default.audio.sink" => guard.default_sink = name,
                                "default.audio.source" => guard.default_source = name,
                                _ => unreachable!(),
                            }
                            guard.publish_snapshot();
                        });
                    }
                    _ => {}
                }
                0
            })
            .register();

        self.metadata_listener = Some(listener);
        self.metadata = Some(metadata);
        self.metadata_id = Some(obj.id);
    }
}
