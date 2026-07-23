//! Public API for trusted compile-time IoTKit Output Adapters.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use thiserror::Error;

pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub const MAX_UNIX_MILLIS: i64 = 253_402_300_799_999;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    Numeric,
    Boolean,
    CumulativeValue,
    Alarm,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObservationValue {
    Numeric(f64),
    Boolean(bool),
    CumulativeValue(u64),
    Alarm { active: bool, reading: Option<f64> },
}

impl ObservationValue {
    #[must_use]
    pub fn kind(&self) -> ObservationKind {
        match self {
            Self::Numeric(_) => ObservationKind::Numeric,
            Self::Boolean(_) => ObservationKind::Boolean,
            Self::CumulativeValue(_) => ObservationKind::CumulativeValue,
            Self::Alarm { .. } => ObservationKind::Alarm,
        }
    }

    fn is_finite(&self) -> bool {
        match self {
            Self::Numeric(value) => value.is_finite(),
            Self::Alarm {
                reading: Some(value),
                ..
            } => value.is_finite(),
            Self::Boolean(_) | Self::CumulativeValue(_) | Self::Alarm { reading: None, .. } => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    observation_id: String,
    series_id: String,
    sequence: u64,
    observed_at: i64,
    value: ObservationValue,
}

impl Observation {
    pub fn new(
        observation_id: impl Into<String>,
        series_id: impl Into<String>,
        sequence: u64,
        observed_at: i64,
        value: ObservationValue,
    ) -> Result<Self, AdapterError> {
        let observation_id = observation_id.into();
        let series_id = series_id.into();
        if !is_canonical_uuid(&observation_id)
            || !is_canonical_uuid(&series_id)
            || sequence == 0
            || sequence > MAX_SAFE_INTEGER
            || !(0..=MAX_UNIX_MILLIS).contains(&observed_at)
            || !value.is_finite()
            || matches!(value, ObservationValue::CumulativeValue(number) if number > MAX_SAFE_INTEGER)
        {
            return Err(AdapterError::InvalidObservation);
        }
        Ok(Self {
            observation_id,
            series_id,
            sequence,
            observed_at,
            value,
        })
    }

    #[must_use]
    pub fn observation_id(&self) -> &str {
        &self.observation_id
    }

    #[must_use]
    pub fn series_id(&self) -> &str {
        &self.series_id
    }

    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub fn observed_at(&self) -> i64 {
        self.observed_at
    }

    #[must_use]
    pub fn kind(&self) -> ObservationKind {
        self.value.kind()
    }

    #[must_use]
    pub fn value(&self) -> &ObservationValue {
        &self.value
    }

    #[must_use]
    pub fn json_value(&self) -> serde_json::Value {
        match self.value {
            ObservationValue::Numeric(value) => serde_json::Value::Number(
                serde_json::Number::from_f64(value).expect("validated finite number"),
            ),
            ObservationValue::Boolean(value) | ObservationValue::Alarm { active: value, .. } => {
                serde_json::Value::Bool(value)
            }
            ObservationValue::CumulativeValue(value) => serde_json::Value::Number(value.into()),
        }
    }

    #[must_use]
    pub fn reading(&self) -> Option<f64> {
        match self.value {
            ObservationValue::Alarm { reading, .. } => reading,
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MqttPublication {
    topic: String,
    qos: u8,
    retain: bool,
    payload: Box<RawValue>,
}

impl MqttPublication {
    pub fn new(
        topic: impl Into<String>,
        qos: u8,
        retain: bool,
        payload: Box<RawValue>,
    ) -> Result<Self, AdapterError> {
        let topic = topic.into();
        if topic.is_empty()
            || topic.contains('\0')
            || topic.contains('+')
            || topic.contains('#')
            || qos != 1
        {
            return Err(AdapterError::InvalidPublication);
        }
        Ok(Self {
            topic,
            qos,
            retain,
            payload,
        })
    }

    #[must_use]
    pub fn topic(&self) -> &str {
        &self.topic
    }

    #[must_use]
    pub fn qos(&self) -> u8 {
        self.qos
    }

    #[must_use]
    pub fn retain(&self) -> bool {
        self.retain
    }

    #[must_use]
    pub fn payload(&self) -> &RawValue {
        &self.payload
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub key: &'static str,
    pub display_name: &'static str,
    pub accepts: &'static [ObservationKind],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Descriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub config_schema_version: u32,
    pub modes: &'static [Mode],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupFieldKind {
    Text,
    Choice(&'static [SetupChoice]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupChoice {
    pub value: &'static str,
    pub display_name: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupField {
    pub key: &'static str,
    pub display_name: &'static str,
    pub kind: SetupFieldKind,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileSetup {
    pub fields: &'static [SetupField],
    pub requires_external_confirmation: bool,
}

#[derive(Debug)]
pub struct ProfileRequest<'a> {
    pub edge_id: &'a str,
    pub signal_id: &'a str,
    pub observation_kind: ObservationKind,
    pub mode: &'a str,
    pub values: &'a serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct RouteProposal {
    pub config: Box<RawValue>,
    pub requires_external_confirmation: bool,
}

pub trait OutputAdapter: Send + Sync {
    fn descriptor(&self) -> &'static Descriptor;

    fn validate_config(&self, config: &RawValue, kind: ObservationKind)
    -> Result<(), AdapterError>;

    fn transform(
        &self,
        config: &RawValue,
        observation: &Observation,
    ) -> Result<MqttPublication, AdapterError>;
}

pub trait ProfilePolicy: Send + Sync {
    fn setup(&self) -> &'static ProfileSetup;

    fn propose(&self, request: &ProfileRequest<'_>) -> Result<Vec<RouteProposal>, AdapterError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AdapterError {
    #[error("invalid adapter descriptor")]
    InvalidDescriptor,
    #[error("invalid adapter configuration")]
    InvalidConfiguration,
    #[error("invalid observation")]
    InvalidObservation,
    #[error("unsupported observation")]
    UnsupportedObservation,
    #[error("invalid MQTT publication")]
    InvalidPublication,
    #[error("adapter transformation failed")]
    TransformFailed,
}

#[must_use]
pub fn is_canonical_uuid(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    value.bytes().enumerate().all(|(index, byte)| {
        if matches!(index, 8 | 13 | 18 | 23) {
            byte == b'-'
        } else {
            byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
        }
    })
}
