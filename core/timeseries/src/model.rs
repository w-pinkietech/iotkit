use std::time::SystemTime;

use iotkit_core_types::SensorType;

/// A single reading row from the sensor_readings table.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadingRow {
    pub adapter_id: String,
    pub device_key: String,
    /// Unix milliseconds since epoch (1970-01-01T00:00:00Z).
    pub ingested_at: i64,
    pub sensor_type: SensorType,
    pub values: Vec<f64>,
    pub rssi: Option<i16>,
    pub battery_pct: Option<u8>,
}

/// Time range for queries.
#[derive(Debug, Clone)]
pub struct TimeRange {
    /// Inclusive start.
    pub start: SystemTime,
    /// Exclusive end.
    pub end: SystemTime,
}
