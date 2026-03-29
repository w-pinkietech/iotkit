use crate::envelope::*;
use crate::error::EncodeError;
use crate::topic::EventType;
use iotkit_core_types::{AdapterId, AdapterEvent};
use std::time::{SystemTime, UNIX_EPOCH};

const ENVELOPE_VERSION: u32 = 1;

/// Current time in milliseconds since Unix epoch.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn system_time_to_ms(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Encode an AdapterEvent into (EventType, JSON bytes).
pub fn encode_event(
    adapter_id: &AdapterId,
    event: &AdapterEvent,
) -> Result<(EventType, Vec<u8>), EncodeError> {
    let aid = adapter_id.as_str().to_string();
    let ts = now_ms();

    match event {
        AdapterEvent::SensorData {
            device_key,
            reading,
            rssi,
            battery_pct,
            ingested_at,
        } => {
            let env = TelemetryEnvelope {
                v: ENVELOPE_VERSION,
                adapter_id: aid,
                ts,
                device_key: device_key.as_str().to_string(),
                sensor_type: reading.sensor_type.as_db_str().to_string(),
                ingested_at: system_time_to_ms(*ingested_at),
                values: reading.values.clone(),
                labels: reading.labels.clone(),
                rssi: *rssi,
                battery_pct: *battery_pct,
            };
            Ok((EventType::Telemetry, serde_json::to_vec(&env)?))
        }
        AdapterEvent::DeviceDiscovered {
            device_key,
            identity,
        } => {
            let env = DiscoveryEnvelope {
                v: ENVELOPE_VERSION,
                adapter_id: aid,
                ts,
                device_key: device_key.as_str().to_string(),
                identity: IdentityPayload {
                    manufacturer: identity.manufacturer.clone(),
                    ic_part_number: identity.ic_part_number.clone(),
                    sensor_type: identity.sensor_type.as_db_str().to_string(),
                    connection: ConnectionPayload {
                        kind: identity.connection.kind.as_str().to_string(),
                        parameters: identity.connection.parameters.clone(),
                    },
                },
            };
            Ok((EventType::Discovery, serde_json::to_vec(&env)?))
        }
        AdapterEvent::DeviceLost { device_key, reason } => {
            let env = LossEnvelope {
                v: ENVELOPE_VERSION,
                adapter_id: aid,
                ts,
                device_key: device_key.as_str().to_string(),
                reason: reason.clone(),
            };
            Ok((EventType::Loss, serde_json::to_vec(&env)?))
        }
        AdapterEvent::AdapterError { device_key, error } => {
            let env = ErrorEnvelope {
                v: ENVELOPE_VERSION,
                adapter_id: aid,
                ts,
                device_key: device_key.as_ref().map(|k| k.as_str().to_string()),
                error: error.clone(),
            };
            Ok((EventType::Error, serde_json::to_vec(&env)?))
        }
        AdapterEvent::DeviceConfig { .. } => {
            Err(EncodeError::UnsupportedEvent("DeviceConfig not supported in v1 MQTT contract".into()))
        }
    }
}

/// Encode a status message.
/// `ts` should be `0` for LWT (set at connect time, actual time unknown)
/// and `now_ms()` for graceful online/offline messages.
pub fn encode_status(adapter_id: &AdapterId, online: bool, ts: i64) -> Vec<u8> {
    let env = StatusEnvelope {
        v: ENVELOPE_VERSION,
        adapter_id: adapter_id.as_str().to_string(),
        ts,
        online,
    };
    serde_json::to_vec(&env).expect("status envelope serialization cannot fail")
}
