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

/// The two clocks of the contract at the moment an input was received.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputTime {
    /// Milliseconds since the device booted (monotonic clock). Debounce and
    /// intervals use this.
    pub uptime_ms: i64,
    /// Wall-clock Unix epoch ms, only while the device can vouch for its clock.
    pub unix_epoch_ms: Option<i64>,
}

impl InputTime {
    /// The current monotonic uptime with the given trusted wall-clock time.
    pub fn now(unix_epoch_ms: Option<i64>) -> Self {
        Self {
            uptime_ms: uptime_ms(),
            unix_epoch_ms,
        }
    }
}

/// Milliseconds since boot from `CLOCK_MONOTONIC`, shared by every process on
/// the device, so the node and `nodectl` publish on the same time base.
pub fn uptime_ms() -> i64 {
    let mut spec = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `clock_gettime` writes into the provided, properly aligned timespec.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut spec) };
    if rc != 0 {
        return 0;
    }
    // `tv_sec` and `tv_nsec` are 32-bit on some 32-bit targets (Raspberry Pi
    // OS 32-bit), so the casts are not redundant everywhere.
    #[allow(clippy::unnecessary_cast)]
    let millis = spec.tv_sec as i64 * 1_000 + spec.tv_nsec as i64 / 1_000_000;
    millis
}

/// One value produced by one pipeline, protocol-neutral.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub pipeline_id: PipelineId,
    pub series_id: String,
    pub sequence: u64,
    pub at: InputTime,
    pub value: ObservationValue,
}

#[derive(Serialize)]
struct Payload<'a> {
    series_id: &'a str,
    sequence: u64,
    uptime_ms: i64,
    unix_epoch_ms: Option<i64>,
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
            uptime_ms: self.at.uptime_ms,
            unix_epoch_ms: self.at.unix_epoch_ms,
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
