//! The status topic of the MQTT Output Adapter v1 contract: heartbeat,
//! graceful `offline`, the Will, and the `faults` list. The bytes produced
//! here are fixed by the `status-*.json` fixtures under `testdata/observation/v1`.

use iotkit_core_types::EdgeNodeId;
use serde::Serialize;

use crate::wire::InputTime;

pub fn status_topic(edge_node_id: &EdgeNodeId) -> String {
    format!("iotkit/v1/edge-node/{edge_node_id}/status")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusValue {
    /// Connected and persisting input.
    Online,
    /// Connected and still draining the outbox, but new input is discarded.
    Degraded,
    /// Published by IoTKit itself at graceful shutdown.
    Offline,
}

impl StatusValue {
    pub fn key(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Degraded => "degraded",
            Self::Offline => "offline",
        }
    }
}

/// Why an Input Adapter could not open its hardware interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceOpenReason {
    NotFound,
    PermissionDenied,
    Busy,
    IoError,
}

impl InterfaceOpenReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::NotFound => "not-found",
            Self::PermissionDenied => "permission-denied",
            Self::Busy => "busy",
            Self::IoError => "io-error",
        }
    }

    pub fn from_io_kind(kind: std::io::ErrorKind) -> Self {
        use std::io::ErrorKind;
        match kind {
            ErrorKind::NotFound => Self::NotFound,
            ErrorKind::PermissionDenied => Self::PermissionDenied,
            ErrorKind::ResourceBusy | ErrorKind::AddrInUse | ErrorKind::WouldBlock => Self::Busy,
            _ => Self::IoError,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultKind {
    /// The input transaction fails and new input is being discarded.
    StorageWriteFailed { count: u64 },
    /// An Input Adapter cannot open its serial, I2C, or GPIO interface.
    InterfaceOpenFailed {
        adapter: String,
        reason: InterfaceOpenReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    pub kind: FaultKind,
    /// The two clocks at the moment the fault started.
    pub since: InputTime,
    /// Short human-readable text; never used for machine decisions.
    pub detail: Option<String>,
}

/// A heartbeat or the graceful `offline`. The Will has its own fixed bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub at: InputTime,
    pub value: StatusValue,
    pub faults: Vec<Fault>,
}

#[derive(Serialize)]
struct FaultPayload<'a> {
    kind: &'static str,
    since_uptime_ms: i64,
    since_unix_epoch_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    adapter: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<&'a str>,
}

impl Fault {
    fn payload(&self) -> FaultPayload<'_> {
        let (kind, count, adapter, reason) = match &self.kind {
            FaultKind::StorageWriteFailed { count } => {
                ("storage-write-failed", Some(*count), None, None)
            }
            FaultKind::InterfaceOpenFailed { adapter, reason } => (
                "interface-open-failed",
                None,
                Some(adapter.as_str()),
                Some(reason.key()),
            ),
        };
        FaultPayload {
            kind,
            since_uptime_ms: self.since.uptime_ms,
            since_unix_epoch_ms: self.since.unix_epoch_ms,
            count,
            adapter,
            reason,
            detail: self.detail.as_deref(),
        }
    }
}

#[derive(Serialize)]
struct StatusPayload<'a> {
    uptime_ms: i64,
    unix_epoch_ms: Option<i64>,
    value: &'static str,
    faults: Vec<FaultPayload<'a>>,
}

impl Status {
    /// Canonical payload bytes: fixed key order, no whitespace.
    pub fn payload(&self) -> Vec<u8> {
        serde_json::to_vec(&StatusPayload {
            uptime_ms: self.at.uptime_ms,
            unix_epoch_ms: self.at.unix_epoch_ms,
            value: self.value.key(),
            faults: self.faults.iter().map(Fault::payload).collect(),
        })
        .expect("status payload serializes")
    }
}

/// The Will registered at connect. IoTKit does not observe the disconnect,
/// so both times are null and there is no `faults` key.
pub const WILL_PAYLOAD: &[u8] = br#"{"uptime_ms":null,"unix_epoch_ms":null,"value":"offline"}"#;

#[cfg(test)]
#[path = "../tests/unit/status_tests.rs"]
mod tests;
