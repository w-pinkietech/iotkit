use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub device_key: String,
    pub sensor_type: String,
    pub ingested_at: i64,
    pub values: Vec<f64>,
    pub labels: Vec<String>,
    pub rssi: Option<i16>,
    pub battery_pct: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityPayload {
    pub manufacturer: String,
    pub ic_part_number: String,
    pub sensor_type: String,
    pub connection: ConnectionPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionPayload {
    pub kind: String,
    pub parameters: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub device_key: String,
    pub identity: IdentityPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LossEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub device_key: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub device_key: Option<String>,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub online: bool,
}

/// Used only for version check during decode.
#[derive(Deserialize)]
pub(crate) struct VersionCheck {
    pub v: u32,
}
