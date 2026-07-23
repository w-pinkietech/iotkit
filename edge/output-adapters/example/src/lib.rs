//! Vendor-neutral compile-tested Output Adapter example.

use iotkit_output_adapter_api::{
    AdapterError, Descriptor, Mode, MqttPublication, Observation, ObservationKind, OutputAdapter,
};
use serde::Deserialize;
use serde_json::value::RawValue;

const MODES: &[Mode] = &[Mode {
    key: "numeric",
    display_name: "Numeric observation",
    accepts: &[ObservationKind::Numeric],
}];
static DESCRIPTOR: Descriptor = Descriptor {
    id: "example.numeric.v1",
    display_name: "Example numeric MQTT v1",
    config_schema_version: 1,
    modes: MODES,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema_version: u32,
    topic: String,
}

pub struct ExampleNumericAdapter;

impl OutputAdapter for ExampleNumericAdapter {
    fn descriptor(&self) -> &'static Descriptor {
        &DESCRIPTOR
    }

    fn validate_config(
        &self,
        config: &RawValue,
        kind: ObservationKind,
    ) -> Result<(), AdapterError> {
        let config: Config =
            serde_json::from_str(config.get()).map_err(|_| AdapterError::InvalidConfiguration)?;
        if config.schema_version != 1
            || kind != ObservationKind::Numeric
            || config.topic.is_empty()
            || config.topic.contains(['\0', '+', '#'])
        {
            return Err(AdapterError::InvalidConfiguration);
        }
        Ok(())
    }

    fn transform(
        &self,
        config: &RawValue,
        observation: &Observation,
    ) -> Result<MqttPublication, AdapterError> {
        self.validate_config(config, observation.kind())?;
        let config: Config =
            serde_json::from_str(config.get()).map_err(|_| AdapterError::InvalidConfiguration)?;
        let payload = serde_json::value::to_raw_value(&serde_json::json!({
            "value": observation.json_value()
        }))
        .map_err(|_| AdapterError::TransformFailed)?;
        MqttPublication::new(config.topic, 1, false, payload)
    }
}
