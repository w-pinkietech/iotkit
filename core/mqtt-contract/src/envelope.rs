use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize)]
pub(crate) struct TelemetryEnvelope {
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

#[derive(Serialize, Deserialize)]
pub(crate) struct DiscoveryEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub device_key: String,
    pub identity: IdentityPayload,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct InventoryEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub session_id: String,
    pub device_key: String,
    pub first_seen_at: i64,
    pub identity: IdentityPayload,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct LossEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub device_key: String,
    pub reason: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct ErrorEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub device_key: Option<String>,
    pub error: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct StatusEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub online: bool,
    pub session_id: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct IdentityPayload {
    pub manufacturer: String,
    pub ic_part_number: String,
    pub sensor_type: String,
    pub connection: ConnectionPayload,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct ConnectionPayload {
    pub kind: String,
    pub parameters: BTreeMap<String, String>,
}

/// Used only for version check during decode.
#[derive(Deserialize)]
pub(crate) struct VersionCheck {
    pub v: u32,
}
