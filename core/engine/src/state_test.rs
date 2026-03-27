//! Tests for State apply logic.

use std::collections::BTreeMap;
use iotkit_core_types::*;
use crate::*;

fn adapter_a() -> AdapterId {
    AdapterId::new("adapter-a")
}

fn device_key_1() -> DeviceKey {
    DeviceKey::new("device-1")
}

fn engine_key(adapter_id: &AdapterId, device_key: &DeviceKey) -> EngineDeviceKey {
    EngineDeviceKey {
        adapter_id: adapter_id.clone(),
        device_key: device_key.clone(),
    }
}

fn sample_identity() -> SensorIdentity {
    SensorIdentity {
        manufacturer: "TestCorp".to_string(),
        ic_part_number: "TC-100".to_string(),
        sensor_type: SensorType::Temperature,
        connection: ConnectionInfo {
            kind: ConnectionKind::Uart,
            parameters: BTreeMap::new(),
        },
    }
}

fn sample_reading() -> SensorReading {
    SensorReading::new(SensorType::Temperature, vec![22.5], vec!["temperature_c"])
}

fn discovered_event(adapter_id: &AdapterId, device_key: &DeviceKey) -> EngineEvent {
    EngineEvent {
        adapter_id: adapter_id.clone(),
        event: AdapterEvent::DeviceDiscovered {
            device_key: device_key.clone(),
            identity: sample_identity(),
        },
    }
}

#[tokio::test]
async fn new_engine_has_no_devices() {
    let engine = Engine::new();
    assert!(engine.devices().await.is_empty());
}

#[tokio::test]
async fn device_discovered_adds_device() {
    let engine = Engine::new();
    let aid = adapter_a();
    let dk = device_key_1();

    engine.apply(discovered_event(&aid, &dk)).await;

    let devices = engine.devices().await;
    assert_eq!(devices.len(), 1);

    let key = engine_key(&aid, &dk);
    let view = engine.device(&key).await.unwrap();
    assert_eq!(view.key, key);
    assert_eq!(view.identity.ic_part_number, "TC-100");
    assert!(view.last_reading.is_none());
}

#[tokio::test]
async fn sensor_data_updates_reading() {
    let engine = Engine::new();
    let aid = adapter_a();
    let dk = device_key_1();

    engine.apply(discovered_event(&aid, &dk)).await;
    engine.apply(EngineEvent {
        adapter_id: aid.clone(),
        event: AdapterEvent::SensorData {
            device_key: dk.clone(),
            reading: sample_reading(),
            rssi: Some(-70),
            battery_pct: Some(85),
        },
    }).await;

    let key = engine_key(&aid, &dk);
    let view = engine.device(&key).await.unwrap();
    assert_eq!(view.last_reading.as_ref().unwrap().values, vec![22.5]);
    assert_eq!(view.rssi, Some(-70));
    assert_eq!(view.battery_pct, Some(85));
}

#[tokio::test]
async fn sensor_data_for_unknown_device_is_ignored() {
    let engine = Engine::new();
    let aid = adapter_a();
    let dk = device_key_1();

    // No DeviceDiscovered — send SensorData directly
    engine.apply(EngineEvent {
        adapter_id: aid.clone(),
        event: AdapterEvent::SensorData {
            device_key: dk.clone(),
            reading: sample_reading(),
            rssi: None,
            battery_pct: None,
        },
    }).await;

    assert!(engine.devices().await.is_empty());
}
