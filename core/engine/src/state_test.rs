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

#[tokio::test]
async fn device_config_updates_config() {
    let engine = Engine::new();
    let aid = adapter_a();
    let dk = device_key_1();

    engine.apply(discovered_event(&aid, &dk)).await;
    engine.apply(EngineEvent {
        adapter_id: aid.clone(),
        event: AdapterEvent::DeviceConfig {
            device_key: dk.clone(),
            config: DeviceConfigData {
                firmware_version: Some("1.2.3".to_string()),
                uplink_interval_secs: Some(60),
                properties: BTreeMap::new(),
            },
        },
    }).await;

    let key = engine_key(&aid, &dk);
    let view = engine.device(&key).await.unwrap();
    let cfg = view.config.unwrap();
    assert_eq!(cfg.firmware_version.as_deref(), Some("1.2.3"));
    assert_eq!(cfg.uplink_interval_secs, Some(60));
}

#[tokio::test]
async fn device_config_for_unknown_device_is_ignored() {
    let engine = Engine::new();
    let aid = adapter_a();
    let dk = device_key_1();

    engine.apply(EngineEvent {
        adapter_id: aid.clone(),
        event: AdapterEvent::DeviceConfig {
            device_key: dk.clone(),
            config: DeviceConfigData {
                firmware_version: None,
                uplink_interval_secs: None,
                properties: BTreeMap::new(),
            },
        },
    }).await;

    assert!(engine.devices().await.is_empty());
}

#[tokio::test]
async fn device_lost_removes_device() {
    let engine = Engine::new();
    let aid = adapter_a();
    let dk = device_key_1();

    engine.apply(discovered_event(&aid, &dk)).await;
    assert_eq!(engine.devices().await.len(), 1);

    engine.apply(EngineEvent {
        adapter_id: aid.clone(),
        event: AdapterEvent::DeviceLost {
            device_key: dk.clone(),
            reason: "timeout".to_string(),
        },
    }).await;

    let key = engine_key(&aid, &dk);
    assert!(engine.device(&key).await.is_none());
    assert!(engine.devices().await.is_empty());
}

#[tokio::test]
async fn device_lost_then_rediscovered_is_new_insert() {
    let engine = Engine::new();
    let aid = adapter_a();
    let dk = device_key_1();

    // Discover → send reading → lose → rediscover
    engine.apply(discovered_event(&aid, &dk)).await;
    engine.apply(EngineEvent {
        adapter_id: aid.clone(),
        event: AdapterEvent::SensorData {
            device_key: dk.clone(),
            reading: sample_reading(),
            rssi: Some(-50),
            battery_pct: Some(100),
        },
    }).await;
    engine.apply(EngineEvent {
        adapter_id: aid.clone(),
        event: AdapterEvent::DeviceLost {
            device_key: dk.clone(),
            reason: "gone".to_string(),
        },
    }).await;
    engine.apply(discovered_event(&aid, &dk)).await;

    let key = engine_key(&aid, &dk);
    let view = engine.device(&key).await.unwrap();
    // Fresh insert — no reading carried over
    assert!(view.last_reading.is_none());
    assert!(view.rssi.is_none());
}

#[tokio::test]
async fn adapter_error_with_device_key_updates_last_error() {
    let engine = Engine::new();
    let aid = adapter_a();
    let dk = device_key_1();

    engine.apply(discovered_event(&aid, &dk)).await;
    engine.apply(EngineEvent {
        adapter_id: aid.clone(),
        event: AdapterEvent::AdapterError {
            device_key: Some(dk.clone()),
            error: "sensor timeout".to_string(),
        },
    }).await;

    let key = engine_key(&aid, &dk);
    let view = engine.device(&key).await.unwrap();
    assert_eq!(view.last_error.as_deref(), Some("sensor timeout"));
}

#[tokio::test]
async fn adapter_error_without_device_key_does_not_affect_devices() {
    let engine = Engine::new();
    let aid = adapter_a();
    let dk = device_key_1();

    engine.apply(discovered_event(&aid, &dk)).await;
    engine.apply(EngineEvent {
        adapter_id: aid.clone(),
        event: AdapterEvent::AdapterError {
            device_key: None,
            error: "serial disconnected".to_string(),
        },
    }).await;

    let key = engine_key(&aid, &dk);
    let view = engine.device(&key).await.unwrap();
    assert!(view.last_error.is_none());
}

#[tokio::test]
async fn multiple_adapters_are_separated() {
    let engine = Engine::new();
    let aid_a = AdapterId::new("adapter-a");
    let aid_b = AdapterId::new("adapter-b");
    let dk = device_key_1(); // same device_key in both adapters

    engine.apply(EngineEvent {
        adapter_id: aid_a.clone(),
        event: AdapterEvent::DeviceDiscovered {
            device_key: dk.clone(),
            identity: sample_identity(),
        },
    }).await;
    engine.apply(EngineEvent {
        adapter_id: aid_b.clone(),
        event: AdapterEvent::DeviceDiscovered {
            device_key: dk.clone(),
            identity: SensorIdentity {
                manufacturer: "OtherCorp".to_string(),
                ic_part_number: "OC-200".to_string(),
                sensor_type: SensorType::Illuminance,
                connection: ConnectionInfo {
                    kind: ConnectionKind::I2c,
                    parameters: BTreeMap::new(),
                },
            },
        },
    }).await;

    assert_eq!(engine.devices().await.len(), 2);

    let key_a = engine_key(&aid_a, &dk);
    let key_b = engine_key(&aid_b, &dk);
    assert_eq!(engine.device(&key_a).await.unwrap().identity.ic_part_number, "TC-100");
    assert_eq!(engine.device(&key_b).await.unwrap().identity.ic_part_number, "OC-200");
}

#[tokio::test]
async fn device_discovered_resend_updates_identity_keeps_reading() {
    let engine = Engine::new();
    let aid = adapter_a();
    let dk = device_key_1();

    engine.apply(discovered_event(&aid, &dk)).await;
    engine.apply(EngineEvent {
        adapter_id: aid.clone(),
        event: AdapterEvent::SensorData {
            device_key: dk.clone(),
            reading: sample_reading(),
            rssi: Some(-60),
            battery_pct: Some(90),
        },
    }).await;

    // Re-discover with different identity
    engine.apply(EngineEvent {
        adapter_id: aid.clone(),
        event: AdapterEvent::DeviceDiscovered {
            device_key: dk.clone(),
            identity: SensorIdentity {
                manufacturer: "NewCorp".to_string(),
                ic_part_number: "NC-300".to_string(),
                sensor_type: SensorType::Temperature,
                connection: ConnectionInfo {
                    kind: ConnectionKind::Uart,
                    parameters: BTreeMap::new(),
                },
            },
        },
    }).await;

    let key = engine_key(&aid, &dk);
    let view = engine.device(&key).await.unwrap();
    // Identity updated
    assert_eq!(view.identity.ic_part_number, "NC-300");
    // Reading preserved
    assert!(view.last_reading.is_some());
    assert_eq!(view.rssi, Some(-60));
}
