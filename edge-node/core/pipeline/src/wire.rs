//! Observation and its MQTT Output Adapter v1 wire form. The bytes produced
//! here are fixed by the fixtures under `testdata/observation/v1`.

use iotkit_core_types::{EdgeNodeId, PipelineId};
use serde::Serialize;

use crate::definition::PipelineKind;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ObservationValue {
    Measurement(f64),
    State(bool),
    AccumulatedCount(i64),
}

impl ObservationValue {
    pub fn kind(self) -> PipelineKind {
        match self {
            Self::Measurement(_) => PipelineKind::Measurement,
            Self::State(_) => PipelineKind::State,
            Self::AccumulatedCount(_) => PipelineKind::AccumulatedCount,
        }
    }

    /// Serializes as the contract's `value`: a JSON number for measurement
    /// (without a fraction when the value is integral), a boolean for state,
    /// and an integer for accumulated-count.
    pub fn to_json(self) -> serde_json::Value {
        match self {
            Self::Measurement(value) => {
                if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
                    serde_json::Value::from(value as i64)
                } else {
                    serde_json::Value::from(value)
                }
            }
            Self::State(value) => serde_json::Value::Bool(value),
            Self::AccumulatedCount(value) => serde_json::Value::from(value),
        }
    }
}

/// One value produced by one pipeline, protocol-neutral.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub pipeline_id: PipelineId,
    pub series_id: String,
    pub sequence: u64,
    pub timestamp: i64,
    pub value: ObservationValue,
}

#[derive(Serialize)]
struct Payload<'a> {
    series_id: &'a str,
    sequence: u64,
    timestamp: i64,
    value: serde_json::Value,
}

impl Observation {
    pub fn topic(&self, edge_node_id: &EdgeNodeId) -> String {
        observation_topic(edge_node_id, &self.pipeline_id, self.value.kind())
    }

    /// Canonical payload bytes: fixed key order, no whitespace.
    pub fn payload(&self) -> Vec<u8> {
        serde_json::to_vec(&Payload {
            series_id: &self.series_id,
            sequence: self.sequence,
            timestamp: self.timestamp,
            value: self.value.to_json(),
        })
        .expect("observation payload serializes")
    }
}

pub fn observation_topic(
    edge_node_id: &EdgeNodeId,
    pipeline_id: &PipelineId,
    kind: PipelineKind,
) -> String {
    format!(
        "iotkit/v1/edge-node/{edge_node_id}/observation/{pipeline_id}/{}",
        kind.key()
    )
}

#[cfg(test)]
#[path = "../tests/unit/wire_tests.rs"]
mod tests;
