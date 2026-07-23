use super::*;
use iotkit_core_types::SensorType;
use std::collections::BTreeMap;

#[test]
fn configured_target_is_resolved_by_the_adapter_owned_model_catalog() {
    let mcp9600 = configured_target(
        "mcp9600",
        0x61,
        &BTreeMap::from([(
            "thermocouple_type".into(),
            AdapterConfigScalar::String("T".into()),
        )]),
    )
    .unwrap();
    let opt3001 = configured_target("opt3001", 0x45, &BTreeMap::new()).unwrap();

    assert_eq!(mcp9600.device_model_id(), "mcp9600");
    assert_eq!(mcp9600.address(), 0x61);
    assert!(matches!(
        mcp9600,
        RpiLocalTarget::MCP9600 {
            thermocouple_type: ThermocoupleType::T,
            ..
        }
    ));
    assert_eq!(opt3001.device_model_id(), "opt3001");
    assert_eq!(opt3001.address(), 0x45);
}

#[test]
fn configured_target_rejects_unknown_models_and_model_specific_settings() {
    let unknown = configured_target("unknown", 0x44, &BTreeMap::new()).unwrap_err();
    assert!(unknown.contains("unsupported device model"));

    let missing_type = configured_target("mcp9600", 0x60, &BTreeMap::new()).unwrap_err();
    assert!(missing_type.contains("thermocouple_type"));

    let bad_type = configured_target(
        "mcp9600",
        0x60,
        &BTreeMap::from([(
            "thermocouple_type".into(),
            AdapterConfigScalar::String("X".into()),
        )]),
    )
    .unwrap_err();
    assert!(bad_type.contains("thermocouple_type"));

    let opt_setting = configured_target(
        "opt3001",
        0x44,
        &BTreeMap::from([(
            "thermocouple_type".into(),
            AdapterConfigScalar::String("K".into()),
        )]),
    )
    .unwrap_err();
    assert!(opt_setting.contains("unsupported setting"));
}

/// Tokio runtime が無い状態で start() を呼ぶと panic せず Err を返す。
#[test]
fn start_without_runtime_returns_error() {
    let config = RpiLocalConfig {
        bus_path: "/dev/i2c-1".to_string(),
        poll_interval_ms: 1000,
        targets: vec![RpiLocalTarget::MCP9600 {
            address: 0x60,
            thermocouple_type: ThermocoupleType::K,
        }],
    };
    let result = start(config, None);
    assert!(
        result.is_err(),
        "start() should return Err without tokio runtime"
    );
}

/// Config validation runs before runtime check, so this test verifies
/// that invalid config produces a config-specific error message even
/// without a tokio runtime.
#[test]
fn start_with_invalid_config_returns_config_error() {
    let config = RpiLocalConfig {
        bus_path: "/dev/i2c-1".to_string(),
        poll_interval_ms: 0,
        targets: vec![],
    };
    let err = start(config, None).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("poll_interval_ms"),
        "expected config validation error, got: {}",
        msg,
    );
}

#[test]
fn validate_rejects_short_poll_interval_for_opt3001() {
    let cfg = RpiLocalConfig {
        bus_path: "/dev/i2c-1".into(),
        poll_interval_ms: 50,
        targets: vec![RpiLocalTarget::OPT3001 { address: 0x44 }],
    };
    let err = validate(&cfg).unwrap_err();
    assert!(err.contains("poll_interval_ms"), "unexpected error: {err}");
}

#[test]
fn validate_accepts_valid_config() {
    let cfg = RpiLocalConfig {
        bus_path: "/dev/i2c-1".into(),
        poll_interval_ms: 1000,
        targets: vec![
            RpiLocalTarget::MCP9600 {
                address: 0x60,
                thermocouple_type: ThermocoupleType::K,
            },
            RpiLocalTarget::OPT3001 { address: 0x44 },
        ],
    };
    assert!(validate(&cfg).is_ok());
}

#[test]
fn built_in_devices_own_model_mapping_and_inventory_metadata() {
    let targets = devices::built_in_targets();
    assert_eq!(
        targets
            .iter()
            .map(RpiLocalTarget::device_model_id)
            .collect::<Vec<_>>(),
        ["mcp9600", "opt3001"]
    );
    assert_eq!(
        targets
            .iter()
            .map(RpiLocalTarget::address)
            .collect::<Vec<_>>(),
        [0x60, 0x44]
    );
    assert_eq!(
        targets
            .iter()
            .map(RpiLocalTarget::inventory_label)
            .collect::<Vec<_>>(),
        ["MCP9600 thermocouple", "OPT3001 illuminance"]
    );
}

#[test]
fn configured_device_selects_its_canonical_measurement_projection() {
    let target = RpiLocalTarget::OPT3001 { address: 0x44 };
    let reading = SensorReading::new(SensorType::Illuminance, vec![512.0], vec!["lux".into()]);

    let projection = target.project(&reading).unwrap();

    assert_eq!(projection.measurement_key, "illuminance_lux");
    assert_eq!(projection.channel_index, None);
    assert_eq!(projection.values, [512.0]);
}

#[test]
fn configured_device_rejects_a_reading_from_another_model_family() {
    let target = RpiLocalTarget::OPT3001 { address: 0x44 };
    let reading = SensorReading::new(SensorType::Temperature, vec![25.0], vec!["celsius".into()]);

    assert!(target.project(&reading).is_err());
}

#[test]
fn configured_device_rejects_wrong_value_shape_or_unit() {
    let target = RpiLocalTarget::OPT3001 { address: 0x44 };
    let wrong_shape =
        SensorReading::new(SensorType::Illuminance, vec![1.0, 2.0], vec!["lux".into()]);
    let wrong_unit = SensorReading::new(SensorType::Illuminance, vec![1.0], vec!["percent".into()]);

    assert!(target.project(&wrong_shape).is_err());
    assert!(target.project(&wrong_unit).is_err());
}

#[test]
fn positional_inventory_comes_from_the_same_configured_devices() {
    let config = RpiLocalConfig {
        bus_path: "/dev/i2c-1".into(),
        poll_interval_ms: 1000,
        targets: devices::built_in_targets(),
    };

    assert_eq!(
        positional_inventory(&config),
        [
            PositionalDeviceMetadata {
                locator: "i2c:0x60".into(),
                model_id: "mcp9600".into(),
                label: "MCP9600 thermocouple".into(),
            },
            PositionalDeviceMetadata {
                locator: "i2c:0x44".into(),
                model_id: "opt3001".into(),
                label: "OPT3001 illuminance".into(),
            },
        ]
    );
}

#[test]
fn mapping_preserves_legacy_positional_identity_and_canonical_units() {
    let config = RpiLocalConfig {
        bus_path: "/dev/i2c-1".into(),
        poll_interval_ms: 1000,
        targets: vec![RpiLocalTarget::OPT3001 { address: 0x44 }],
    };
    let items = to_items(
        &config,
        "rpi-local:default",
        &DeviceKey::new("i2c:0x44:opt3001"),
        &SensorReading::new(SensorType::Illuminance, vec![512.0], vec!["lux".into()]),
    )
    .unwrap();
    assert_eq!(
        items[0].subject_hint.as_deref(),
        Some("rpi-local:default:i2c:0x44")
    );
    assert_eq!(items[0].measurement_key, "illuminance_lux");
}

#[tokio::test]
async fn panic_completion_waits_for_lower_runtime_cleanup() {
    let instance_id =
        iotkit_input_adapter_host_api::AdapterInstanceId::new("cleanup_test").unwrap();
    let (runtime, running) = runtime_channels(instance_id, 1);
    let completion = runtime.completion;
    let composition = tokio::spawn(async {
        panic!("test composition panic");
        #[allow(unreachable_code)]
        AdapterCompletion::RequestedStop
    });
    let (cleanup_tx, cleanup_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(finalize_after_cleanup(
        composition,
        async move {
            cleanup_rx
                .await
                .map_err(|_| "cleanup signal dropped".to_string())
        },
        completion,
    ));

    let completion_wait = running.completion.wait();
    tokio::pin!(completion_wait);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut completion_wait)
            .await
            .is_err(),
        "completion must remain pending until lower runtime cleanup finishes"
    );
    cleanup_tx.send(()).unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), &mut completion_wait)
            .await
            .expect("completion after cleanup"),
        AdapterCompletion::Panic
    );
}

/// OPT3001 driver rejects poll intervals shorter than 200ms.
#[test]
fn opt3001_rejects_short_poll_interval() {
    let config = RpiLocalConfig {
        bus_path: "/dev/i2c-1".to_string(),
        poll_interval_ms: 50,
        targets: vec![RpiLocalTarget::OPT3001 { address: 0x44 }],
    };
    let err = start(config, None).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("OPT3001"),
        "expected OPT3001 validation error, got: {}",
        msg,
    );
}
