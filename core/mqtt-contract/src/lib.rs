mod decode;
mod encode;
mod envelope;
mod error;
mod topic;

pub use decode::{decode_event, decode_inventory, decode_status};
pub use encode::{encode_event, encode_inventory, encode_status, now_ms, InventoryData};
pub use error::{DecodeError, EncodeError};
pub use topic::{decode_topic_segment, encode_topic_segment, inventory_topic, topic, EventType};


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
        let session = "abcd1234abcd1234abcd1234abcd1234";
        let ts = now_ms();
        let bytes = encode_status(&aid, true, ts, session);
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["v"], 1);
        assert_eq!(json["adapter_id"], "rpi-local:default");
        assert_eq!(json["online"], true);
        assert_eq!(json["session_id"], session);
    }

    #[test]
    fn status_lwt_uses_zero_ts() {
        let aid = sample_adapter_id();
        let session = "abcd1234abcd1234abcd1234abcd1234";
        let bytes = encode_status(&aid, false, 0, session);
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["ts"], 0);
        assert_eq!(json["online"], false);
        assert_eq!(json["session_id"], session);
    }

    #[test]
    fn encode_status_includes_session_id() {
        let aid = sample_adapter_id();
        let bytes = encode_status(&aid, true, 1000, "abcd1234abcd1234abcd1234abcd1234");
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["v"], 1);
        assert_eq!(json["adapter_id"], "rpi-local:default");
        assert_eq!(json["ts"], 1000);
        assert_eq!(json["online"], true);
        assert_eq!(json["session_id"], "abcd1234abcd1234abcd1234abcd1234");
    }

    #[test]
    fn encode_inventory_includes_session_id_and_first_seen_at() {
        let aid = sample_adapter_id();
        let dk = DeviceKey::new("i2c:0x60:mcp9600");
        let mut params = BTreeMap::new();
        params.insert("address".into(), "0x60".into());
        let data = InventoryData {
            device_key: dk,
            identity: SensorIdentity {
                manufacturer: "Microchip".into(),
                ic_part_number: "MCP9600".into(),
                sensor_type: SensorType::Temperature,
                connection: ConnectionInfo {
                    kind: ConnectionKind::I2c,
                    parameters: params,
                },
            },
            first_seen_at: 900000,
        };
        let bytes = encode_inventory(&aid, &data, "sess1234sess1234sess1234sess1234", 1000000);
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["v"], 1);
        assert_eq!(json["adapter_id"], "rpi-local:default");
        assert_eq!(json["session_id"], "sess1234sess1234sess1234sess1234");
        assert_eq!(json["first_seen_at"], 900000);
        assert_eq!(json["ts"], 1000000);
        assert_eq!(json["device_key"], "i2c:0x60:mcp9600");
        assert_eq!(json["identity"]["manufacturer"], "Microchip");
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

    #[test]
    fn decode_status_returns_session_id() {
        let aid = sample_adapter_id();
        let session = "abcd1234abcd1234abcd1234abcd1234";
        let bytes = encode_status(&aid, true, 5000, session);
        let (decoded_aid, online, ts, decoded_session) = decode_status(&bytes).unwrap();
        assert_eq!(decoded_aid.as_str(), "rpi-local:default");
        assert!(online);
        assert_eq!(ts, 5000);
        assert_eq!(decoded_session, session);
    }

    #[test]
    fn decode_status_lwt_ts_zero_accepted() {
        let aid = sample_adapter_id();
        let session = "abcd1234abcd1234abcd1234abcd1234";
        let bytes = encode_status(&aid, false, 0, session);
        let (_, online, ts, _) = decode_status(&bytes).unwrap();
        assert!(!online);
        assert_eq!(ts, 0);
    }

    #[test]
    fn decode_inventory_returns_session_and_first_seen() {
        let aid = sample_adapter_id();
        let dk = DeviceKey::new("i2c:0x60:mcp9600");
        let mut params = BTreeMap::new();
        params.insert("address".into(), "0x60".into());
        let data = InventoryData {
            device_key: dk,
            identity: SensorIdentity {
                manufacturer: "Microchip".into(),
                ic_part_number: "MCP9600".into(),
                sensor_type: SensorType::Temperature,
                connection: ConnectionInfo {
                    kind: ConnectionKind::I2c,
                    parameters: params,
                },
            },
            first_seen_at: 900000,
        };
        let bytes = encode_inventory(&aid, &data, "sess1234sess1234sess1234sess1234", 1000000);
        let (decoded_aid, event, session_id, first_seen_at) = decode_inventory(&bytes).unwrap();
        assert_eq!(decoded_aid.as_str(), "rpi-local:default");
        assert_eq!(session_id, "sess1234sess1234sess1234sess1234");
        assert_eq!(first_seen_at, 900000);
        if let AdapterEvent::DeviceDiscovered { device_key, identity } = event {
            assert_eq!(device_key.as_str(), "i2c:0x60:mcp9600");
            assert_eq!(identity.manufacturer, "Microchip");
        } else {
            panic!("expected DeviceDiscovered");
        }
    }

    #[test]
    fn decode_telemetry_label_value_mismatch() {
        let json = br#"{"v":1,"adapter_id":"test","ts":1000,"device_key":"k","sensor_type":"temperature","ingested_at":999,"values":[1.0,2.0],"labels":["a"],"rssi":null,"battery_pct":null}"#;
        let result = decode_event(EventType::Telemetry, json);
        assert!(matches!(result, Err(DecodeError::InvalidPayload(_))));
    }

    #[test]
    fn decode_negative_ts_rejected() {
        let json = br#"{"v":1,"adapter_id":"test","ts":-5,"device_key":"k","reason":"lost"}"#;
        let result = decode_event(EventType::Loss, json);
        assert!(matches!(result, Err(DecodeError::InvalidTimestamp(-5))));
    }

    #[test]
    fn decode_unknown_fields_ignored() {
        let json = br#"{"v":1,"adapter_id":"test","ts":1000,"device_key":"k","reason":"lost","future_field":"hello"}"#;
        let result = decode_event(EventType::Loss, json);
        assert!(result.is_ok());
    }

    #[test]
    fn connectionkind_as_str_from_str_symmetry() {
        let variants = [
            ConnectionKind::Uart,
            ConnectionKind::I2c,
            ConnectionKind::Gpio,
            ConnectionKind::Modbus,
            ConnectionKind::Other("custom".to_string()),
        ];
        for v in &variants {
            let s = v.as_str();
            let round_tripped = ConnectionKind::from_str(s);
            assert_eq!(&round_tripped, v, "round-trip failed for {v:?}");
        }
    }

    #[test]
    fn connectionkind_from_str_normalizes_known() {
        let result = ConnectionKind::from_str("i2c");
        assert_eq!(result, ConnectionKind::I2c);
    }

    #[test]
    fn encode_event_discovery_has_no_session_id() {
        let aid = sample_adapter_id();
        let event = AdapterEvent::DeviceDiscovered {
            device_key: DeviceKey::new("test"),
            identity: SensorIdentity {
                manufacturer: "Test".into(),
                ic_part_number: "T1".into(),
                sensor_type: SensorType::Temperature,
                connection: ConnectionInfo {
                    kind: ConnectionKind::I2c,
                    parameters: BTreeMap::new(),
                },
            },
        };
        let (_, bytes) = encode_event(&aid, &event).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(json.get("session_id").is_none(), "discovery notification must NOT include session_id");
    }

    #[test]
    fn inventory_payload_includes_session_id() {
        let aid = sample_adapter_id();
        let data = InventoryData {
            device_key: DeviceKey::new("test"),
            identity: SensorIdentity {
                manufacturer: "Test".into(),
                ic_part_number: "T1".into(),
                sensor_type: SensorType::Temperature,
                connection: ConnectionInfo {
                    kind: ConnectionKind::I2c,
                    parameters: BTreeMap::new(),
                },
            },
            first_seen_at: 1000,
        };
        let bytes = encode_inventory(&aid, &data, "sess1234sess1234sess1234sess1234", 2000);
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(json.get("session_id").is_some(), "inventory must include session_id");
    }

    #[test]
    fn status_decode_rejects_negative_ts() {
        let json = br#"{"v":1,"adapter_id":"test","ts":-1,"online":false,"session_id":"abcd1234abcd1234abcd1234abcd1234"}"#;
        let result = decode_status(json);
        assert!(matches!(result, Err(DecodeError::InvalidTimestamp(-1))));
    }

    #[test]
    fn decode_event_rejects_status_type() {
        let status_json = br#"{"v":1,"adapter_id":"test","ts":0,"online":true,"session_id":"x"}"#;
        let result = decode_event(EventType::Status, status_json);
        assert!(matches!(result, Err(DecodeError::InvalidPayload(_))));
    }

    #[test]
    fn decode_event_rejects_inventory_type() {
        let result = decode_event(EventType::Inventory, b"{}");
        assert!(matches!(result, Err(DecodeError::InvalidPayload(_))));
    }

    #[test]
    fn segment_encode_roundtrip_all_specials() {
        let input = "a:b/c+d#e%f";
        let encoded = encode_topic_segment(input);
        assert_eq!(encoded, "a%3Ab%2Fc%2Bd%23e%25f");
        let decoded = decode_topic_segment(&encoded).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn segment_encode_empty_string() {
        let encoded = encode_topic_segment("");
        assert_eq!(encoded, "");
        let decoded = decode_topic_segment(&encoded).unwrap();
        assert_eq!(decoded, "");
    }
}
