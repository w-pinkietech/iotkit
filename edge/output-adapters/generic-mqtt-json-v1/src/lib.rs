//! IoTKit generic MQTT JSON Output Adapter v1.

use iotkit_output_adapter_api::{
    AdapterError, Descriptor, IdentityPolicy, IdentityScope, Mode, MqttPublication, Observation,
    ObservationKind, OutputAdapter, ProfilePolicy, ProfileRequest, ProfileSetup, RouteProposal,
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

const ACCEPTS: &[ObservationKind] = &[
    ObservationKind::Numeric,
    ObservationKind::Boolean,
    ObservationKind::CumulativeValue,
    ObservationKind::Alarm,
];
const MODES: &[Mode] = &[Mode {
    key: "observation",
    display_name: "IoTKit共通Observation",
    accepts: ACCEPTS,
}];
static DESCRIPTOR: Descriptor = Descriptor {
    id: "iotkit.mqtt-json.v1",
    display_name: "IoTKit MQTT JSON v1",
    config_schema_version: 1,
    modes: MODES,
};
static SETUP: ProfileSetup = ProfileSetup {
    fields: &[],
    requires_external_confirmation: false,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema_version: u32,
    topic: String,
}

#[derive(Serialize)]
struct Payload<'a> {
    schema_version: u32,
    observation_id: &'a str,
    series_id: &'a str,
    sequence: u64,
    observed_at: i64,
    kind: ObservationKind,
    value: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    reading: Option<f64>,
}

pub struct GenericMqttJsonAdapter;
pub struct GenericMqttJsonPolicy;

impl OutputAdapter for GenericMqttJsonAdapter {
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
            || config.topic.is_empty()
            || config.topic.contains(['\0', '+', '#'])
            || !ACCEPTS.contains(&kind)
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
        let payload = serde_json::value::to_raw_value(&Payload {
            schema_version: 1,
            observation_id: observation.observation_id(),
            series_id: observation.series_id(),
            sequence: observation.sequence(),
            observed_at: observation.observed_at(),
            kind: observation.kind(),
            value: observation.json_value(),
            reading: observation.reading(),
        })
        .map_err(|_| AdapterError::TransformFailed)?;
        MqttPublication::new(config.topic, 1, false, payload)
    }
}

impl ProfilePolicy for GenericMqttJsonPolicy {
    fn setup(&self) -> &'static ProfileSetup {
        &SETUP
    }

    fn identity_policy(&self) -> IdentityPolicy {
        IdentityPolicy {
            scope: IdentityScope::RuleMode,
            prefix: "sig-",
        }
    }

    fn propose(&self, request: &ProfileRequest<'_>) -> Result<Vec<RouteProposal>, AdapterError> {
        if request.mode != "observation" {
            return Err(AdapterError::InvalidConfiguration);
        }
        let topic = format!(
            "iotkit/v1/sources/{}/signals/{}/observations",
            request.edge_id, request.external_id
        );
        let config = serde_json::value::to_raw_value(&serde_json::json!({
            "schema_version": 1,
            "topic": topic
        }))
        .map_err(|_| AdapterError::InvalidConfiguration)?;
        Ok(vec![RouteProposal {
            config,
            requires_external_confirmation: false,
        }])
    }
}
