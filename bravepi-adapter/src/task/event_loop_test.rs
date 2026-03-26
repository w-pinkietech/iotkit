use iotkit_core_types::{AdapterCommand, AdapterEvent, SensorType};
use tokio::sync::mpsc;

use crate::transport::TransportError;
use super::event_loop::event_loop;

/// Build raw frame bytes for the BravePI codec.
/// Format: [payload_len:u16 LE][device_number:u64 LE][sensor_type:u16 LE][rssi:i8][flag:u8][payload...]
fn build_sensor_frame_bytes(device_number: u64, sensor_type: u16, rssi: i8, battery: u8, count: u16, values: &[u8]) -> Vec<u8> {
    let mut payload = vec![battery];
    payload.extend_from_slice(&count.to_le_bytes());
    payload.extend_from_slice(values);

    let payload_len = payload.len() as u16;
    let mut frame = Vec::new();
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&device_number.to_le_bytes());
    frame.extend_from_slice(&sensor_type.to_le_bytes());
    frame.push(rssi as u8);
    frame.push(0x00); // flag = no continuation
    frame.extend_from_slice(&payload);
    frame
}

#[tokio::test]
async fn shutdown_command_exits_event_loop() {
    let (_bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);

    let handle = tokio::spawn(event_loop("test".into(), bytes_rx, event_tx, command_rx));

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();

    assert!(event_rx.recv().await.is_none());
}

#[tokio::test]
async fn bytes_channel_error_produces_adapter_error() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(16);

    let handle = tokio::spawn(event_loop("test".into(), bytes_rx, event_tx, command_rx));

    bytes_tx.send(Err(TransportError { message: "serial port disconnected".to_string() })).await.unwrap();
    handle.await.unwrap();

    match event_rx.recv().await.expect("should receive event") {
        AdapterEvent::AdapterError { device_key, error } => {
            assert!(device_key.is_none());
            assert!(error.contains("serial port disconnected"));
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }
}

#[tokio::test]
async fn bytes_channel_close_produces_adapter_error() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(16);

    let handle = tokio::spawn(event_loop("test".into(), bytes_rx, event_tx, command_rx));

    drop(bytes_tx);
    handle.await.unwrap();

    match event_rx.recv().await.expect("should receive event") {
        AdapterEvent::AdapterError { error, .. } => {
            assert!(error.contains("exited unexpectedly"));
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }
}

#[tokio::test]
async fn normal_data_flow_produces_device_discovered_then_sensor_data() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);

    let handle = tokio::spawn(event_loop("/dev/test".into(), bytes_rx, event_tx, command_rx));

    let device: u64 = 0x246880020140018b;
    let frame_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();

    match event_rx.recv().await.expect("should receive DeviceDiscovered") {
        AdapterEvent::DeviceDiscovered { device_key, identity } => {
            assert_eq!(device_key.as_str(), "bravepi:246880020140018b:temperature");
            assert_eq!(identity.manufacturer, "Braveridge");
            assert_eq!(identity.ic_part_number, "MCP9600");
        }
        other => panic!("expected DeviceDiscovered, got {:?}", other),
    }

    match event_rx.recv().await.expect("should receive SensorData") {
        AdapterEvent::SensorData { device_key, reading, .. } => {
            assert_eq!(device_key.as_str(), "bravepi:246880020140018b:temperature");
            assert_eq!(reading.sensor_type, SensorType::Temperature);
            assert!((reading.values[0] - 22.4375).abs() < 0.01);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }

    let frame_bytes2 = build_sensor_frame_bytes(device, 261, -55, 90, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(frame_bytes2)).await.unwrap();

    match event_rx.recv().await.expect("should receive SensorData") {
        AdapterEvent::SensorData { .. } => {}
        other => panic!("expected SensorData (no DeviceDiscovered), got {:?}", other),
    }

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}
