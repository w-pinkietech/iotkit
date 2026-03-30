use crate::envelope::*;
use crate::error::DecodeError;
use crate::topic::EventType;
use iotkit_core_types::*;
use std::time::{Duration, UNIX_EPOCH};

/// Check common envelope version.
fn check_version(v: u32) -> Result<(), DecodeError> {
    if v != 1 {
        return Err(DecodeError::UnknownVersion(v));
    }
    Ok(())
}

/// Check timestamp is non-negative.
fn check_ts(ts: i64) -> Result<(), DecodeError> {
    if ts < 0 {
        return Err(DecodeError::InvalidTimestamp(ts));
    }
    Ok(())
}

fn identity_from_payload(p: IdentityPayload) -> SensorIdentity {
    SensorIdentity {
        manufacturer: p.manufacturer,
        ic_part_number: p.ic_part_number,
        sensor_type: SensorType::from_db_str(&p.sensor_type),
        connection: ConnectionInfo {
            kind: ConnectionKind::from_str(&p.connection.kind),
            parameters: p.connection.parameters,
        },
    }
}

/// Pre-check: parse as Value, check version and timestamp before typed deserialization.
/// This ensures DecodeError::UnknownVersion and DecodeError::InvalidTimestamp take priority
/// over serde field-missing errors (spec section 1.4).
fn precheck(payload: &[u8]) -> Result<serde_json::Value, DecodeError> {
    let val: serde_json::Value = serde_json::from_slice(payload)?;
    if let Some(v_val) = val.get("v") {
        match v_val.as_u64() {
            Some(v) => check_version(v as u32)?,
            None => {
                return Err(DecodeError::InvalidPayload(
                    "v must be an integer".to_string(),
                ))
            }
        }
    }
    if let Some(ts) = val.get("ts").and_then(|v| v.as_i64()) {
        check_ts(ts)?;
    }
    Ok(val)
}

/// Decode a non-status, non-inventory event payload.
pub fn decode_event(
    event_type: EventType,
    payload: &[u8],
) -> Result<(AdapterId, AdapterEvent), DecodeError> {
    let _val = precheck(payload)?;

    match event_type {
        EventType::Telemetry => {
            let env: TelemetryEnvelope = serde_json::from_slice(payload)?;
            // ts already checked in precheck, but check ingested_at
            if env.ingested_at < 0 {
                return Err(DecodeError::InvalidTimestamp(env.ingested_at));
            }
            if env.labels.len() != env.values.len() {
                return Err(DecodeError::InvalidPayload(format!(
                    "labels length {} does not match values length {}",
                    env.labels.len(),
                    env.values.len()
                )));
            }
            let ingested_at = UNIX_EPOCH + Duration::from_millis(env.ingested_at as u64);
            Ok((
                AdapterId::new(env.adapter_id),
                AdapterEvent::SensorData {
                    device_key: DeviceKey::new(env.device_key),
                    reading: SensorReading::new(
                        SensorType::from_db_str(&env.sensor_type),
                        env.values,
                        env.labels,
                    ),
                    rssi: env.rssi,
                    battery_pct: env.battery_pct,
                    ingested_at,
                },
            ))
        }
        EventType::Discovery => {
            let env: DiscoveryEnvelope = serde_json::from_slice(payload)?;
            Ok((
                AdapterId::new(env.adapter_id),
                AdapterEvent::DeviceDiscovered {
                    device_key: DeviceKey::new(env.device_key),
                    identity: identity_from_payload(env.identity),
                },
            ))
        }
        EventType::Loss => {
            let env: LossEnvelope = serde_json::from_slice(payload)?;
            Ok((
                AdapterId::new(env.adapter_id),
                AdapterEvent::DeviceLost {
                    device_key: DeviceKey::new(env.device_key),
                    reason: env.reason,
                },
            ))
        }
        EventType::Error => {
            let env: ErrorEnvelope = serde_json::from_slice(payload)?;
            Ok((
                AdapterId::new(env.adapter_id),
                AdapterEvent::AdapterError {
                    device_key: env.device_key.map(DeviceKey::new),
                    error: env.error,
                },
            ))
        }
        EventType::Status | EventType::Inventory => {
            Err(DecodeError::InvalidPayload(format!(
                "decode_event cannot decode {:?}; use decode_status() or decode_inventory()",
                event_type
            )))
        }
    }
}

/// Decode a status payload. Returns `(adapter_id, online, ts, session_id)`.
///
/// `ts = 0` is accepted (LWT sentinel). Other negative values are rejected.
pub fn decode_status(payload: &[u8]) -> Result<(AdapterId, bool, i64, String), DecodeError> {
    let _val = precheck(payload)?;
    let env: StatusEnvelope = serde_json::from_slice(payload)?;
    // ts=0 is valid for LWT; only reject strictly negative
    if env.ts < 0 {
        return Err(DecodeError::InvalidTimestamp(env.ts));
    }
    Ok((
        AdapterId::new(env.adapter_id),
        env.online,
        env.ts,
        env.session_id,
    ))
}

/// Decode an inventory payload. Returns `(adapter_id, DeviceDiscovered event, session_id, first_seen_at)`.
pub fn decode_inventory(
    payload: &[u8],
) -> Result<(AdapterId, AdapterEvent, String, i64), DecodeError> {
    let _val = precheck(payload)?;
    let env: InventoryEnvelope = serde_json::from_slice(payload)?;
    if env.first_seen_at < 0 {
        return Err(DecodeError::InvalidTimestamp(env.first_seen_at));
    }
    Ok((
        AdapterId::new(env.adapter_id),
        AdapterEvent::DeviceDiscovered {
            device_key: DeviceKey::new(env.device_key),
            identity: identity_from_payload(env.identity),
        },
        env.session_id,
        env.first_seen_at,
    ))
}
