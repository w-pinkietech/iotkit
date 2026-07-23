use super::*;

#[test]
fn device_command_construction() {
    let cmd = DeviceCommand {
        device_key: DeviceKey::new("bravepi:abc:temperature"),
        payload: DeviceCommandPayload::RequestReading,
    };
    assert_eq!(cmd.device_key.as_str(), "bravepi:abc:temperature");
}

#[test]
fn device_command_set_output() {
    let cmd = DeviceCommand {
        device_key: DeviceKey::new("bravepi:abc:contact_output"),
        payload: DeviceCommandPayload::SetOutput {
            value: true,
            duration_ms: Some(5000),
        },
    };
    match cmd.payload {
        DeviceCommandPayload::SetOutput { value, duration_ms } => {
            assert!(value);
            assert_eq!(duration_ms, Some(5000));
        }
        _ => panic!("expected SetOutput"),
    }
}

#[test]
fn adapter_command_device_command_variant() {
    let cmd = AdapterCommand::DeviceCommand(DeviceCommand {
        device_key: DeviceKey::new("test"),
        payload: DeviceCommandPayload::QueryConfig,
    });
    match cmd {
        AdapterCommand::DeviceCommand(dc) => {
            assert_eq!(dc.device_key.as_str(), "test");
        }
        _ => panic!("expected DeviceCommand"),
    }
}

#[test]
fn device_config_data_construction() {
    let config = DeviceConfigData {
        firmware_version: Some("1.2.3".to_string()),
        uplink_interval_secs: Some(60),
        properties: BTreeMap::from([
            ("timezone".into(), ConfigValue::Integer(9)),
            ("ble_mode".into(), ConfigValue::Integer(1)),
        ]),
    };
    assert_eq!(config.firmware_version.as_deref(), Some("1.2.3"));
    assert_eq!(config.uplink_interval_secs, Some(60));
    assert_eq!(config.properties.len(), 2);
}

#[test]
fn config_value_variants() {
    assert_eq!(
        ConfigValue::String("hello".into()),
        ConfigValue::String("hello".into())
    );
    assert_eq!(ConfigValue::Integer(42), ConfigValue::Integer(42));
    assert_eq!(ConfigValue::Float(1.5_f64), ConfigValue::Float(1.5_f64));
    assert_eq!(ConfigValue::Bool(true), ConfigValue::Bool(true));
}

#[test]
fn adapter_event_device_config_variant() {
    let event = AdapterEvent::DeviceConfig {
        device_key: DeviceKey::new("bravepi:abc:temperature"),
        config: DeviceConfigData {
            firmware_version: Some("1.0.0".to_string()),
            uplink_interval_secs: None,
            properties: BTreeMap::new(),
        },
    };
    match event {
        AdapterEvent::DeviceConfig { device_key, config } => {
            assert_eq!(device_key.as_str(), "bravepi:abc:temperature");
            assert_eq!(config.firmware_version.as_deref(), Some("1.0.0"));
        }
        _ => panic!("expected DeviceConfig"),
    }
}
