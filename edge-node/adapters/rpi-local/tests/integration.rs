//! Integration tests — require real I2C hardware.
//! Run with: cargo test -p rpi-local-adapter --test integration -- --ignored

use std::time::Duration;

use rpi_local_adapter::PollingEvent as AdapterEvent;
use rpi_local_adapter::{RpiLocalConfig, RpiLocalTarget, ThermocoupleType};

#[tokio::test]
#[ignore]
async fn real_i2c_discovers_and_reads_mcp9600() {
    let config = RpiLocalConfig {
        bus_path: "/dev/i2c-1".to_string(),
        poll_interval_ms: 1000,
        targets: vec![RpiLocalTarget::MCP9600 {
            address: 0x60,
            thermocouple_type: ThermocoupleType::K,
        }],
    };

    let mut handle = rpi_local_adapter::start(config, None).expect("start() should succeed");

    // First event should be DeviceDiscovered
    let event = tokio::time::timeout(Duration::from_secs(5), handle.event_rx.recv())
        .await
        .expect("timeout waiting for DeviceDiscovered")
        .expect("channel should not be closed");
    assert!(
        matches!(event, AdapterEvent::DeviceDiscovered { .. }),
        "expected DeviceDiscovered, got {:?}",
        event,
    );

    // SensorData arrives on next poll tick (not same cycle as probe,
    // because sensors may need conversion latency after init).
    let event = tokio::time::timeout(Duration::from_secs(5), handle.event_rx.recv())
        .await
        .expect("timeout waiting for SensorData")
        .expect("channel should not be closed");
    assert!(
        matches!(event, AdapterEvent::SensorData { .. }),
        "expected SensorData, got {:?}",
        event,
    );

    // Shutdown cleanly
    handle.shutdown().await;
}

#[tokio::test]
#[ignore]
async fn real_i2c_discovers_and_reads_opt3001() {
    let config = RpiLocalConfig {
        bus_path: "/dev/i2c-1".to_string(),
        poll_interval_ms: 1000,
        targets: vec![RpiLocalTarget::OPT3001 { address: 0x44 }],
    };

    let mut handle = rpi_local_adapter::start(config, None).expect("start() should succeed");

    // DeviceDiscovered from startup probe.
    let event = tokio::time::timeout(Duration::from_secs(5), handle.event_rx.recv())
        .await
        .expect("timeout waiting for DeviceDiscovered")
        .expect("channel should not be closed");
    assert!(
        matches!(event, AdapterEvent::DeviceDiscovered { .. }),
        "expected DeviceDiscovered, got {:?}",
        event,
    );

    // SensorData arrives on next poll tick (conversion latency).
    let event = tokio::time::timeout(Duration::from_secs(5), handle.event_rx.recv())
        .await
        .expect("timeout waiting for SensorData")
        .expect("channel should not be closed");
    assert!(
        matches!(event, AdapterEvent::SensorData { .. }),
        "expected SensorData, got {:?}",
        event,
    );

    handle.shutdown().await;
}
