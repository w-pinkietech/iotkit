mod decode;
mod encode;
mod envelope;
mod error;
mod topic;

pub use decode::{decode_event, decode_status};
pub use encode::{encode_event, encode_status, now_ms};
pub use error::{DecodeError, EncodeError};
pub use topic::{encode_topic_segment, inventory_topic, topic, EventType};


#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_types::*;
    use std::collections::BTreeMap;
    use std::time::{Duration, UNIX_EPOCH};

    fn sample_adapter_id() -> AdapterId {
        AdapterId::new("rpi-local:default")
    }

    #[test]
    fn roundtrip_telemetry() {
        let aid = sample_adapter_id();
        let event = AdapterEvent::SensorData {
            device_key: DeviceKey::new("i2c:0x60:mcp9600"),
            reading: SensorReading::new(
                SensorType::Temperature,
                vec![25.3],
                vec!["temperature_c".into()],
            ),
            rssi: Some(-70),
            battery_pct: Some(85),
            ingested_at: UNIX_EPOCH + Duration::from_millis(1711700000000),
        };
        let (et, bytes) = encode_event(&aid, &event).unwrap();
        assert_eq!(et, EventType::Telemetry);

        let (decoded_aid, decoded_event) = decode_event(EventType::Telemetry, &bytes).unwrap();
        assert_eq!(decoded_aid.as_str(), aid.as_str());

        if let AdapterEvent::SensorData { device_key, reading, rssi, battery_pct, ingested_at } = decoded_event {
            assert_eq!(device_key.as_str(), "i2c:0x60:mcp9600");
            assert_eq!(reading.sensor_type, SensorType::Temperature);
            assert_eq!(reading.values, vec![25.3]);
            assert_eq!(reading.labels, vec!["temperature_c"]);
            assert_eq!(rssi, Some(-70));
            assert_eq!(battery_pct, Some(85));
            assert_eq!(ingested_at, UNIX_EPOCH + Duration::from_millis(1711700000000));
        } else {
            panic!("expected SensorData");
        }
    }

    #[test]
    fn roundtrip_discovery() {
        let aid = sample_adapter_id();
        let mut params = BTreeMap::new();
        params.insert("address".into(), "0x60".into());
        let event = AdapterEvent::DeviceDiscovered {
            device_key: DeviceKey::new("i2c:0x60:mcp9600"),
            identity: SensorIdentity {
                manufacturer: "Microchip".into(),
                ic_part_number: "MCP9600".into(),
                sensor_type: SensorType::Temperature,
                connection: ConnectionInfo {
                    kind: ConnectionKind::I2c,
                    parameters: params,
                },
            },
        };
        let (et, bytes) = encode_event(&aid, &event).unwrap();
        assert_eq!(et, EventType::Discovery);

        let (_, decoded) = decode_event(EventType::Discovery, &bytes).unwrap();
        if let AdapterEvent::DeviceDiscovered { identity, .. } = decoded {
            assert_eq!(identity.manufacturer, "Microchip");
            assert_eq!(identity.connection.kind, ConnectionKind::I2c);
        } else {
            panic!("expected DeviceDiscovered");
        }
    }

    #[test]
    fn roundtrip_loss() {
        let aid = sample_adapter_id();
        let event = AdapterEvent::DeviceLost {
            device_key: DeviceKey::new("i2c:0x60:mcp9600"),
            reason: "5 consecutive read failures".into(),
        };
        let (et, bytes) = encode_event(&aid, &event).unwrap();
        assert_eq!(et, EventType::Loss);

        let (_, decoded) = decode_event(EventType::Loss, &bytes).unwrap();
        if let AdapterEvent::DeviceLost { reason, .. } = decoded {
            assert_eq!(reason, "5 consecutive read failures");
        } else {
            panic!("expected DeviceLost");
        }
    }

    #[test]
    fn roundtrip_error() {
        let aid = sample_adapter_id();
        let event = AdapterEvent::AdapterError {
            device_key: None,
            error: "bus error".into(),
        };
        let (et, bytes) = encode_event(&aid, &event).unwrap();
        assert_eq!(et, EventType::Error);

        let (_, decoded) = decode_event(EventType::Error, &bytes).unwrap();
        if let AdapterEvent::AdapterError { device_key, error } = decoded {
            assert!(device_key.is_none());
            assert_eq!(error, "bus error");
        } else {
            panic!("expected AdapterError");
        }
    }

    #[test]
    fn roundtrip_status() {
        let aid = sample_adapter_id();
        let bytes = encode_status(&aid, true, now_ms());
        let (decoded_aid, online) = decode_status(&bytes).unwrap();
        assert_eq!(decoded_aid.as_str(), aid.as_str());
        assert!(online);
    }

    #[test]
    fn status_lwt_uses_zero_ts() {
        let aid = sample_adapter_id();
        let bytes = encode_status(&aid, false, 0);
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["ts"], 0);
        assert_eq!(json["online"], false);
    }

    #[test]
    fn encode_device_config_returns_unsupported() {
        let aid = sample_adapter_id();
        let event = AdapterEvent::DeviceConfig {
            device_key: DeviceKey::new("test"),
            config: DeviceConfigData {
                firmware_version: None,
                uplink_interval_secs: None,
                properties: BTreeMap::new(),
            },
        };
        let result = encode_event(&aid, &event);
        assert!(result.is_err());
    }

    #[test]
    fn decode_negative_timestamp_returns_error() {
        let json = br#"{"v":1,"adapter_id":"test","ts":0,"device_key":"k","sensor_type":"temperature","ingested_at":-1,"values":[],"labels":[],"rssi":null,"battery_pct":null}"#;
        let result = decode_event(EventType::Telemetry, json);
        assert!(result.is_err());
    }

    #[test]
    fn decode_unknown_version_returns_error() {
        let json = br#"{"v":99,"adapter_id":"test","ts":0,"device_key":"k","sensor_type":"temperature","ingested_at":0,"values":[],"labels":[],"rssi":null,"battery_pct":null}"#;
        let result = decode_event(EventType::Telemetry, json);
        assert!(matches!(result, Err(DecodeError::UnknownVersion(99))));
    }
}
