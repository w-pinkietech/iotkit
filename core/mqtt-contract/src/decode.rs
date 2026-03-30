use crate::envelope::*;
use crate::error::DecodeError;
use crate::topic::EventType;
use iotkit_core_types::*;
use std::time::{Duration, UNIX_EPOCH};

const SUPPORTED_VERSION: u32 = 1;

fn check_version(payload: &[u8]) -> Result<(), DecodeError> {
    let vc: VersionCheck = serde_json::from_slice(payload)?;
    if vc.v != SUPPORTED_VERSION {
        return Err(DecodeError::UnknownVersion(vc.v));
    }
    Ok(())
}

fn ms_to_system_time(ms: i64) -> Result<std::time::SystemTime, DecodeError> {
    if ms < 0 {
        return Err(DecodeError::InvalidTimestamp(ms));
    }
    Ok(UNIX_EPOCH + Duration::from_millis(ms as u64))
}

/// Decode MQTT payload back to (AdapterId, AdapterEvent).
pub fn decode_event(
    event_type: EventType,
    payload: &[u8],
) -> Result<(AdapterId, AdapterEvent), DecodeError> {
    check_version(payload)?;

    match event_type {
        EventType::Telemetry => {
            let env: TelemetryEnvelope = serde_json::from_slice(payload)?;
            let event = AdapterEvent::SensorData {
                device_key: DeviceKey::new(env.device_key),
                reading: SensorReading::new(
                    SensorType::from_db_str(&env.sensor_type),
                    env.values,
                    env.labels,
                ),
                rssi: env.rssi,
                battery_pct: env.battery_pct,
                ingested_at: ms_to_system_time(env.ingested_at)?,
            };
            Ok((AdapterId::new(env.adapter_id), event))
        }
        EventType::Discovery => {
            let env: DiscoveryEnvelope = serde_json::from_slice(payload)?;
            let event = AdapterEvent::DeviceDiscovered {
                device_key: DeviceKey::new(env.device_key),
                identity: SensorIdentity {
                    manufacturer: env.identity.manufacturer,
                    ic_part_number: env.identity.ic_part_number,
                    sensor_type: SensorType::from_db_str(&env.identity.sensor_type),
                    connection: ConnectionInfo {
                        kind: ConnectionKind::from_str(&env.identity.connection.kind),
                        parameters: env.identity.connection.parameters,
                    },
                },
            };
            Ok((AdapterId::new(env.adapter_id), event))
        }
        EventType::Loss => {
            let env: LossEnvelope = serde_json::from_slice(payload)?;
            let event = AdapterEvent::DeviceLost {
                device_key: DeviceKey::new(env.device_key),
                reason: env.reason,
            };
            Ok((AdapterId::new(env.adapter_id), event))
        }
        EventType::Error => {
            let env: ErrorEnvelope = serde_json::from_slice(payload)?;
            let event = AdapterEvent::AdapterError {
                device_key: env.device_key.map(DeviceKey::new),
                error: env.error,
            };
            Ok((AdapterId::new(env.adapter_id), event))
        }
        EventType::Status => {
            Err(DecodeError::InvalidPayload("use decode_status for status messages".into()))
        }
        EventType::Inventory => {
            Err(DecodeError::InvalidPayload("use decode_inventory for inventory messages".into()))
        }
    }
}

/// Decode a status message.
pub fn decode_status(payload: &[u8]) -> Result<(AdapterId, bool), DecodeError> {
    check_version(payload)?;
    let env: StatusEnvelope = serde_json::from_slice(payload)?;
    Ok((AdapterId::new(env.adapter_id), env.online))
}
