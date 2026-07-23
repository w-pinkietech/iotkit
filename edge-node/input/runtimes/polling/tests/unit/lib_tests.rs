use super::*;
use iotkit_core_types::{
    ConnectionInfo, ConnectionKind, SensorIdentity, SensorReading, SensorType,
};
use std::collections::BTreeMap;

/// Minimal no-op driver for tests.
struct StubDriver;

impl SensorDriver for StubDriver {
    fn detect(&self, _bus_path: &str, _address: u8) -> Result<SensorIdentity, String> {
        Ok(SensorIdentity {
            manufacturer: "Test".into(),
            ic_part_number: "STUB".into(),
            sensor_type: SensorType::Temperature,
            connection: ConnectionInfo {
                kind: ConnectionKind::I2c,
                parameters: BTreeMap::new(),
            },
        })
    }
    fn init(&self, _bus_path: &str, _address: u8) -> Result<(), String> {
        Ok(())
    }
    fn read(&self, _bus_path: &str, _address: u8) -> Result<SensorReading, String> {
        Ok(SensorReading::empty(SensorType::Temperature))
    }
    fn ic_name(&self) -> &'static str {
        "STUB"
    }
}

/// Driver that rejects poll intervals below a threshold.
struct StrictDriver {
    min_interval_ms: u64,
}

impl SensorDriver for StrictDriver {
    fn detect(&self, _bus_path: &str, _address: u8) -> Result<SensorIdentity, String> {
        Ok(SensorIdentity {
            manufacturer: "Test".into(),
            ic_part_number: "STRICT".into(),
            sensor_type: SensorType::Temperature,
            connection: ConnectionInfo {
                kind: ConnectionKind::I2c,
                parameters: BTreeMap::new(),
            },
        })
    }
    fn init(&self, _bus_path: &str, _address: u8) -> Result<(), String> {
        Ok(())
    }
    fn read(&self, _bus_path: &str, _address: u8) -> Result<SensorReading, String> {
        Ok(SensorReading::empty(SensorType::Temperature))
    }
    fn ic_name(&self) -> &'static str {
        "STRICT"
    }
    fn validate(&self, poll_interval_ms: u64) -> Result<(), String> {
        if poll_interval_ms < self.min_interval_ms {
            return Err(format!(
                "poll_interval_ms {} too short, minimum is {}",
                poll_interval_ms, self.min_interval_ms,
            ));
        }
        Ok(())
    }
}

fn stub_config() -> PollingAdapterConfig {
    PollingAdapterConfig {
        bus_path: "/dev/i2c-1".into(),
        poll_interval_ms: 1000,
        targets: vec![SensorTargetConfig {
            address: 0x40,
            driver: Arc::new(StubDriver),
            key_suffix: None,
        }],
    }
}

#[test]
fn valid_config_passes() {
    assert!(validate_config(&stub_config()).is_ok());
}

#[test]
fn empty_bus_path_rejected() {
    let mut cfg = stub_config();
    cfg.bus_path = String::new();
    let err = validate_config(&cfg).unwrap_err();
    assert!(err.contains("bus_path"), "unexpected error: {err}");
}

#[test]
fn empty_targets_rejected() {
    let mut cfg = stub_config();
    cfg.targets.clear();
    let err = validate_config(&cfg).unwrap_err();
    assert!(err.contains("targets"), "unexpected error: {err}");
}

#[test]
fn zero_poll_interval_rejected() {
    let mut cfg = stub_config();
    cfg.poll_interval_ms = 0;
    let err = validate_config(&cfg).unwrap_err();
    assert!(err.contains("poll_interval_ms"), "unexpected error: {err}");
}

#[test]
fn duplicate_address_rejected() {
    let mut cfg = stub_config();
    cfg.targets.push(SensorTargetConfig {
        address: 0x40,
        driver: Arc::new(StubDriver),
        key_suffix: Some("second".into()),
    });
    let err = validate_config(&cfg).unwrap_err();
    assert!(err.contains("duplicate"), "unexpected error: {err}");
}

#[test]
fn address_out_of_range_rejected() {
    for bad_addr in [0x00, 0x07, 0x78, 0xFF] {
        let cfg = PollingAdapterConfig {
            bus_path: "/dev/i2c-1".into(),
            poll_interval_ms: 1000,
            targets: vec![SensorTargetConfig {
                address: bad_addr,
                driver: Arc::new(StubDriver),
                key_suffix: None,
            }],
        };
        let err = validate_config(&cfg).unwrap_err();
        assert!(
            err.contains("outside valid I2C range"),
            "addr 0x{bad_addr:02X}: unexpected error: {err}",
        );
    }
}

#[test]
fn driver_validate_called() {
    let cfg = PollingAdapterConfig {
        bus_path: "/dev/i2c-1".into(),
        poll_interval_ms: 50,
        targets: vec![SensorTargetConfig {
            address: 0x40,
            driver: Arc::new(StrictDriver {
                min_interval_ms: 100,
            }),
            key_suffix: None,
        }],
    };
    let err = validate_config(&cfg).unwrap_err();
    assert!(err.contains("too short"), "unexpected error: {err}");
}

#[tokio::test]
async fn into_parts_preserves_id_and_channels() {
    use iotkit_core_types::SensorType;

    let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(1);
    let (command_tx, mut command_rx) = mpsc::channel::<AdapterCommand>(1);
    let handle = AdapterHandle {
        id: AdapterId::new("test:into-parts"),
        event_rx,
        command_tx,
        task_handle: None,
    };
    let parts = handle.into_parts();

    assert_eq!(parts.id.as_str(), "test:into-parts");

    let mut event_rx = parts.event_rx;
    event_tx
        .send(AdapterEvent::SensorData {
            device_key: iotkit_core_types::DeviceKey::new("test:0"),
            reading: SensorReading::empty(SensorType::Temperature),
            rssi: None,
            battery_pct: None,
            ingested_at: std::time::SystemTime::now(),
        })
        .await
        .unwrap();
    let received = event_rx.recv().await;
    assert!(received.is_some(), "event_rx should receive the sent event");

    parts
        .shutdown
        .shutdown()
        .await
        .expect("shutdown should succeed");
    let cmd = command_rx.recv().await;
    assert!(
        matches!(cmd, Some(AdapterCommand::Shutdown)),
        "shutdown should send Shutdown command"
    );
}

#[test]
fn start_without_runtime_returns_error() {
    // No Tokio runtime active on this thread.
    let cfg = stub_config();
    let err = start(AdapterId::new("test"), cfg, None).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("no reactor") || msg.contains("runtime"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn start_with_bad_bus_path() {
    let cfg = PollingAdapterConfig {
        bus_path: "/tmp/iotkit-nonexistent-bus-path-test".into(),
        poll_interval_ms: 1000,
        targets: vec![SensorTargetConfig {
            address: 0x40,
            driver: Arc::new(StubDriver),
            key_suffix: None,
        }],
    };
    let err = start(AdapterId::new("test"), cfg, None).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("cannot open bus_path"),
        "unexpected error: {msg}"
    );
}
