//! Pinikiet MQTT Output Adapter v1.

use iotkit_output_adapter_api::{
    AdapterError, Descriptor, IdentityPolicy, IdentityScope, MAX_SAFE_INTEGER, Mode,
    MqttPublication, Observation, ObservationKind, ObservationValue, OutputAdapter, ProfilePolicy,
    ProfileRequest, ProfileSetup, RouteProposal, SetupField, SetupFieldKind,
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

const MODES: &[Mode] = &[
    Mode {
        key: "production",
        display_name: "累積値",
        accepts: &[ObservationKind::CumulativeValue],
    },
    Mode {
        key: "onoff",
        display_name: "ON/OFF",
        accepts: &[ObservationKind::Boolean],
    },
    Mode {
        key: "gantt_chart",
        display_name: "稼働状態",
        accepts: &[ObservationKind::Boolean],
    },
    Mode {
        key: "alarm",
        display_name: "アラーム",
        accepts: &[ObservationKind::Alarm],
    },
];
static DESCRIPTOR: Descriptor = Descriptor {
    id: "pinikiet.mqtt.v1",
    display_name: "Pinikiet MQTT v1",
    config_schema_version: 1,
    modes: MODES,
};
static SETUP: ProfileSetup = ProfileSetup {
    fields: &[SetupField {
        key: "reason",
        display_name: "Alarm reason",
        kind: SetupFieldKind::Text,
        required: false,
    }],
    requires_external_confirmation: true,
};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PinikietKind {
    Production,
    Onoff,
    GanttChart,
    Alarm,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema_version: u32,
    source_id: String,
    sensor_id: String,
    kind: PinikietKind,
    #[serde(default)]
    reason: String,
}

#[derive(Serialize)]
struct Payload<'a> {
    schema_version: u32,
    observation_id: &'a str,
    series_id: &'a str,
    sequence: u64,
    observed_at: i64,
    kind: PinikietKind,
    value: serde_json::Value,
    #[serde(skip_serializing_if = "String::is_empty")]
    reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reading: Option<f64>,
}

#[derive(Serialize)]
struct SourceStatus {
    schema_version: u32,
    reported_at: i64,
    state: &'static str,
}

pub struct PinikietMqttAdapter;
pub struct PinikietProfilePolicy;

pub fn source_status(source_id: &str, reported_at: i64) -> Result<MqttPublication, AdapterError> {
    if !valid_id(source_id) || !(0..=253_402_300_799_999).contains(&reported_at) {
        return Err(AdapterError::InvalidConfiguration);
    }
    let payload = serde_json::value::to_raw_value(&SourceStatus {
        schema_version: 1,
        reported_at,
        state: "online",
    })
    .map_err(|_| AdapterError::TransformFailed)?;
    MqttPublication::new(
        format!("pinikiet/v1/sources/{source_id}/status"),
        1,
        true,
        payload,
    )
}

impl OutputAdapter for PinikietMqttAdapter {
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
            || !valid_id(&config.source_id)
            || !valid_id(&config.sensor_id)
            || config.reason.len() > 512
            || (config.kind != PinikietKind::Alarm && !config.reason.is_empty())
        {
            return Err(AdapterError::InvalidConfiguration);
        }
        let compatible = matches!(
            (kind, config.kind),
            (ObservationKind::CumulativeValue, PinikietKind::Production)
                | (ObservationKind::Boolean, PinikietKind::Onoff)
                | (ObservationKind::Boolean, PinikietKind::GanttChart)
                | (ObservationKind::Alarm, PinikietKind::Alarm)
        );
        if !compatible {
            return Err(AdapterError::UnsupportedObservation);
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
        if let ObservationValue::CumulativeValue(value) = observation.value()
            && *value > MAX_SAFE_INTEGER
        {
            return Err(AdapterError::InvalidObservation);
        }
        let payload = serde_json::value::to_raw_value(&Payload {
            schema_version: 1,
            observation_id: observation.observation_id(),
            series_id: observation.series_id(),
            sequence: observation.sequence(),
            observed_at: observation.observed_at(),
            kind: config.kind,
            value: observation.json_value(),
            reason: if config.kind == PinikietKind::Alarm {
                config.reason
            } else {
                String::new()
            },
            reading: if config.kind == PinikietKind::Alarm {
                observation.reading()
            } else {
                None
            },
        })
        .map_err(|_| AdapterError::TransformFailed)?;
        MqttPublication::new(
            format!(
                "pinikiet/v1/sources/{}/sensors/{}/observations",
                config.source_id, config.sensor_id
            ),
            1,
            false,
            payload,
        )
    }
}

impl ProfilePolicy for PinikietProfilePolicy {
    fn setup(&self) -> &'static ProfileSetup {
        &SETUP
    }

    fn identity_policy(&self) -> IdentityPolicy {
        IdentityPolicy {
            scope: IdentityScope::Signal,
            prefix: "sen-",
        }
    }

    fn propose(&self, request: &ProfileRequest<'_>) -> Result<Vec<RouteProposal>, AdapterError> {
        let reason = request
            .values
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let compatible = matches!(
            (request.observation_kind, request.mode),
            (ObservationKind::CumulativeValue, "production")
                | (ObservationKind::Boolean, "onoff" | "gantt_chart")
                | (ObservationKind::Alarm, "alarm")
        );
        if !compatible || reason.len() > 512 || (request.mode != "alarm" && !reason.is_empty()) {
            return Err(AdapterError::InvalidConfiguration);
        }
        let config = serde_json::value::to_raw_value(&serde_json::json!({
            "schema_version": 1,
            "source_id": request.edge_id,
            "sensor_id": request.external_id,
            "kind": request.mode,
            "reason": reason
        }))
        .map_err(|_| AdapterError::InvalidConfiguration)?;
        Ok(vec![RouteProposal {
            config,
            requires_external_confirmation: true,
        }])
    }
}

fn valid_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}
