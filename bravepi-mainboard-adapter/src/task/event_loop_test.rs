use iotkit_core_types::{
    AdapterCommand, AdapterEvent, ConfigValue, DeviceCommand, DeviceCommandPayload, DeviceKey,
    SensorType,
};
use iotkit_ingest_client::channel_for_test;
use tokio::sync::mpsc;

use super::event_loop::event_loop;
use crate::transport::TransportError;

/// Build raw frame bytes for a BravePI config frame.
/// sensor_type=0 in the header signals a config frame.
fn build_config_frame_bytes(device_number: u64, true_sensor_type: u16) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&true_sensor_type.to_le_bytes());
    payload.extend_from_slice(&[1, 2, 3]); // firmware "1.2.3"
    payload.push(9); // timezone
    payload.push(1); // ble_mode
    payload.push(4); // tx_power
    payload.extend_from_slice(&1000u16.to_le_bytes()); // advertise_interval
    payload.extend_from_slice(&60u32.to_le_bytes()); // uplink_interval

    let payload_len = payload.len() as u16;
    let mut frame = Vec::new();
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&device_number.to_le_bytes());
    frame.extend_from_slice(&0u16.to_le_bytes()); // sensor_type = 0 means config
    frame.push((-50i8) as u8); // rssi
    frame.push(0x00); // flag
    frame.extend_from_slice(&payload);
    frame
}

/// Build raw frame bytes for the BravePI codec.
/// Format: [payload_len:u16 LE][device_number:u64 LE][sensor_type:u16 LE][rssi:i8][flag:u8][payload...]
fn build_sensor_frame_bytes(
    device_number: u64,
    sensor_type: u16,
    rssi: i8,
    battery: u8,
    count: u16,
    values: &[u8],
) -> Vec<u8> {
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
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        None,
    ));

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();

    assert!(event_rx.recv().await.is_none());
}

#[tokio::test]
async fn bytes_channel_error_produces_adapter_error() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        None,
    ));

    bytes_tx
        .send(Err(TransportError {
            message: "serial port disconnected".to_string(),
        }))
        .await
        .unwrap();
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
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        None,
    ));

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
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "/dev/test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        None,
    ));

    let device: u64 = 0x246880020140018b;
    let frame_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();

    match event_rx
        .recv()
        .await
        .expect("should receive DeviceDiscovered")
    {
        AdapterEvent::DeviceDiscovered {
            device_key,
            identity,
        } => {
            assert_eq!(
                device_key.as_str(),
                "bravepi-mainboard:246880020140018b:temperature"
            );
            assert_eq!(identity.manufacturer, "Braveridge");
            assert_eq!(identity.ic_part_number, "MCP9600");
        }
        other => panic!("expected DeviceDiscovered, got {:?}", other),
    }

    match event_rx.recv().await.expect("should receive SensorData") {
        AdapterEvent::SensorData {
            device_key,
            reading,
            ..
        } => {
            assert_eq!(
                device_key.as_str(),
                "bravepi-mainboard:246880020140018b:temperature"
            );
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

#[tokio::test]
async fn sensor_data_submits_ingest_envelope_before_adapter_event() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);
    let (ingest, mut ingest_rx) = channel_for_test(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "/dev/test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        Some(ingest),
    ));

    let device: u64 = 0x246880020140018b;
    let frame_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();

    let envelope = tokio::time::timeout(std::time::Duration::from_secs(1), ingest_rx.recv())
        .await
        .expect("timed out waiting for ingest envelope")
        .expect("ingest receiver closed");
    assert_eq!(envelope.source, "bravepi-mainboard:/dev/ttyAMA0");
    assert_eq!(envelope.items.len(), 1);
    assert_eq!(
        envelope.items[0].subject_hint.as_deref(),
        Some("ble:246880020140018b")
    );
    assert_eq!(envelope.items[0].measurement_key, "temperature_c");

    match event_rx
        .recv()
        .await
        .expect("should receive DeviceDiscovered")
    {
        AdapterEvent::DeviceDiscovered { device_key, .. } => {
            assert_eq!(
                device_key.as_str(),
                "bravepi-mainboard:246880020140018b:temperature"
            );
        }
        other => panic!("expected DeviceDiscovered, got {:?}", other),
    }
    match event_rx.recv().await.expect("should receive SensorData") {
        AdapterEvent::SensorData { device_key, .. } => {
            assert_eq!(
                device_key.as_str(),
                "bravepi-mainboard:246880020140018b:temperature"
            );
        }
        other => panic!("expected SensorData, got {:?}", other),
    }

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn contact_input_over_256_samples_submits_multiple_scalar_ingest_envelopes() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, _event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);
    let (ingest, mut ingest_rx) = channel_for_test(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "/dev/test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        Some(ingest),
    ));

    let device: u64 = 0x246880020140018b;
    let samples: Vec<u8> = (0..300).map(|i| (i % 2) as u8).collect();
    let frame_bytes = build_sensor_frame_bytes(device, 257, -60, 95, 300, &samples);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();

    let first = tokio::time::timeout(std::time::Duration::from_secs(1), ingest_rx.recv())
        .await
        .expect("timed out waiting for first ingest envelope")
        .expect("ingest receiver closed before first envelope");
    let second = tokio::time::timeout(std::time::Duration::from_secs(1), ingest_rx.recv())
        .await
        .expect("timed out waiting for second ingest envelope")
        .expect("ingest receiver closed before second envelope");

    assert_eq!(first.items.len(), 256);
    assert_eq!(second.items.len(), 44);
    assert!(first.items.iter().chain(second.items.iter()).all(|item| {
        item.measurement_key == "contact_state"
            && item.channel_index.is_none()
            && item.values.len() == 1
    }));
    let actual: Vec<f64> = first
        .items
        .iter()
        .chain(second.items.iter())
        .map(|item| item.values[0])
        .collect();
    let expected: Vec<f64> = samples.iter().map(|v| f64::from(*v)).collect();
    assert_eq!(actual, expected);

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn unknown_raw_sensor_type_does_not_submit_ingest_envelope() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, _event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);
    let (ingest, mut ingest_rx) = channel_for_test(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "/dev/test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        Some(ingest),
    ));

    let frame_bytes = build_sensor_frame_bytes(0x0123456789abcdef, 9999, -60, 95, 1, &[0x01]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
    assert!(
        ingest_rx.try_recv().is_err(),
        "unknown raw sensor type should not submit an envelope"
    );
}

#[tokio::test]
async fn contact_input_produces_device_discovered() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "/dev/test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        None,
    ));

    let device: u64 = 0xaabbccdd00112233;
    let frame_bytes = build_sensor_frame_bytes(device, 257, -50, 80, 1, &[0x01]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();

    match event_rx
        .recv()
        .await
        .expect("should receive DeviceDiscovered")
    {
        AdapterEvent::DeviceDiscovered {
            device_key,
            identity,
        } => {
            assert_eq!(
                device_key.as_str(),
                "bravepi-mainboard:aabbccdd00112233:contact_input"
            );
            assert_eq!(identity.manufacturer, "Braveridge");
            assert_eq!(identity.ic_part_number, "Contact Input Module");
            assert_eq!(identity.sensor_type, SensorType::ContactInput);
        }
        other => panic!("expected DeviceDiscovered, got {:?}", other),
    }

    match event_rx.recv().await.expect("should receive SensorData") {
        AdapterEvent::SensorData {
            device_key,
            reading,
            ..
        } => {
            assert_eq!(
                device_key.as_str(),
                "bravepi-mainboard:aabbccdd00112233:contact_input"
            );
            assert_eq!(reading.sensor_type, SensorType::ContactInput);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn same_transmitter_different_sensor_type_produces_two_discoveries() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "/dev/test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        None,
    ));

    let device: u64 = 0x246880020140018b;

    // --- Temperature frame ---
    let temp_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(temp_bytes)).await.unwrap();

    // DeviceDiscovered for temperature
    match event_rx
        .recv()
        .await
        .expect("should receive DeviceDiscovered #1")
    {
        AdapterEvent::DeviceDiscovered { device_key, .. } => {
            assert_eq!(
                device_key.as_str(),
                "bravepi-mainboard:246880020140018b:temperature"
            );
        }
        other => panic!("expected DeviceDiscovered, got {:?}", other),
    }
    // SensorData for temperature
    match event_rx.recv().await.expect("should receive SensorData #1") {
        AdapterEvent::SensorData { device_key, .. } => {
            assert_eq!(
                device_key.as_str(),
                "bravepi-mainboard:246880020140018b:temperature"
            );
        }
        other => panic!("expected SensorData, got {:?}", other),
    }

    // --- ContactInput frame (same transmitter, different sensor type) ---
    let contact_bytes = build_sensor_frame_bytes(device, 257, -55, 90, 1, &[0x01]);
    bytes_tx.send(Ok(contact_bytes)).await.unwrap();

    // DeviceDiscovered for contact_input (different logical device)
    match event_rx
        .recv()
        .await
        .expect("should receive DeviceDiscovered #2")
    {
        AdapterEvent::DeviceDiscovered { device_key, .. } => {
            assert_eq!(
                device_key.as_str(),
                "bravepi-mainboard:246880020140018b:contact_input"
            );
        }
        other => panic!("expected DeviceDiscovered, got {:?}", other),
    }
    // SensorData for contact_input
    match event_rx.recv().await.expect("should receive SensorData #2") {
        AdapterEvent::SensorData { device_key, .. } => {
            assert_eq!(
                device_key.as_str(),
                "bravepi-mainboard:246880020140018b:contact_input"
            );
        }
        other => panic!("expected SensorData, got {:?}", other),
    }

    // --- Repeat: temperature again (no new DeviceDiscovered) ---
    let temp_bytes2 = build_sensor_frame_bytes(device, 261, -58, 92, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(temp_bytes2)).await.unwrap();

    match event_rx
        .recv()
        .await
        .expect("should receive SensorData only")
    {
        AdapterEvent::SensorData { device_key, .. } => {
            assert_eq!(
                device_key.as_str(),
                "bravepi-mainboard:246880020140018b:temperature"
            );
        }
        other => panic!("expected SensorData (no re-discover), got {:?}", other),
    }

    // --- Repeat: contact again (no new DeviceDiscovered) ---
    let contact_bytes2 = build_sensor_frame_bytes(device, 257, -52, 88, 1, &[0x00]);
    bytes_tx.send(Ok(contact_bytes2)).await.unwrap();

    match event_rx
        .recv()
        .await
        .expect("should receive SensorData only")
    {
        AdapterEvent::SensorData { device_key, .. } => {
            assert_eq!(
                device_key.as_str(),
                "bravepi-mainboard:246880020140018b:contact_input"
            );
        }
        other => panic!("expected SensorData (no re-discover), got {:?}", other),
    }

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn device_command_request_reading_produces_downlink_bytes() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "/dev/test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        None,
    ));

    // Discover a temperature device
    let device: u64 = 0x246880020140018b;
    let frame_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();

    // Drain DeviceDiscovered + SensorData
    let _discovered = event_rx.recv().await.unwrap();
    let _sensor_data = event_rx.recv().await.unwrap();

    // Send RequestReading command
    command_tx
        .send(AdapterCommand::DeviceCommand(DeviceCommand {
            device_key: DeviceKey::new("bravepi-mainboard:246880020140018b:temperature"),
            payload: DeviceCommandPayload::RequestReading,
        }))
        .await
        .unwrap();

    // Assert bytes appear on write_rx
    let bytes = tokio::time::timeout(std::time::Duration::from_secs(1), write_rx.recv())
        .await
        .expect("timed out waiting for downlink bytes")
        .expect("write_rx closed");
    assert_eq!(
        bytes[0], 0x00,
        "first byte should be 0x00 (downlink direction marker)"
    );

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn device_command_unknown_device_produces_adapter_error() {
    let (_bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "/dev/test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        None,
    ));

    // Send command to unknown device (no discovery)
    command_tx
        .send(AdapterCommand::DeviceCommand(DeviceCommand {
            device_key: DeviceKey::new("bravepi-mainboard:unknown:temperature"),
            payload: DeviceCommandPayload::RequestReading,
        }))
        .await
        .unwrap();

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
        .await
        .expect("timed out waiting for error event")
        .expect("event_rx closed");

    match event {
        AdapterEvent::AdapterError { device_key, error } => {
            assert_eq!(
                device_key,
                Some(DeviceKey::new("bravepi-mainboard:unknown:temperature"))
            );
            assert!(
                error.contains("unknown device"),
                "error should contain 'unknown device', got: {}",
                error
            );
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn set_output_to_non_contact_device_produces_adapter_error() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "/dev/test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        None,
    ));

    // Discover a temperature device
    let device: u64 = 0x246880020140018b;
    let frame_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();

    // Drain DeviceDiscovered + SensorData
    let _discovered = event_rx.recv().await.unwrap();
    let _sensor_data = event_rx.recv().await.unwrap();

    // Send SetOutput to a temperature device (not ContactOutput)
    command_tx
        .send(AdapterCommand::DeviceCommand(DeviceCommand {
            device_key: DeviceKey::new("bravepi-mainboard:246880020140018b:temperature"),
            payload: DeviceCommandPayload::SetOutput {
                value: true,
                duration_ms: Some(1000),
            },
        }))
        .await
        .unwrap();

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
        .await
        .expect("timed out waiting for error event")
        .expect("event_rx closed");

    match event {
        AdapterEvent::AdapterError { device_key, error } => {
            assert!(device_key.is_some());
            assert!(
                error.contains("ContactOutput"),
                "error should contain 'ContactOutput', got: {}",
                error
            );
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn set_output_duration_exceeds_u16_max_produces_adapter_error() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "/dev/test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        None,
    ));

    // Discover a contact_output device
    let device: u64 = 0x1234567890abcdef;
    let frame_bytes = build_sensor_frame_bytes(device, 258, -50, 80, 1, &[0x01]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();

    // Drain DeviceDiscovered + SensorData
    let _discovered = event_rx.recv().await.unwrap();
    let _sensor_data = event_rx.recv().await.unwrap();

    // Send SetOutput with duration_ms > u16::MAX
    command_tx
        .send(AdapterCommand::DeviceCommand(DeviceCommand {
            device_key: DeviceKey::new("bravepi-mainboard:1234567890abcdef:contact_output"),
            payload: DeviceCommandPayload::SetOutput {
                value: true,
                duration_ms: Some(70000),
            },
        }))
        .await
        .unwrap();

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
        .await
        .expect("timed out waiting for error event")
        .expect("event_rx closed");

    match event {
        AdapterEvent::AdapterError { device_key, error } => {
            assert!(device_key.is_some());
            assert!(
                error.contains("duration_ms"),
                "error should contain 'duration_ms', got: {}",
                error
            );
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn config_frame_produces_device_config_event() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "/dev/test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        None,
    ));

    // Discover a temperature device (0x246880020140018b, sensor_type 261)
    let device: u64 = 0x246880020140018b;
    let sensor_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(sensor_bytes)).await.unwrap();

    // Drain DeviceDiscovered + SensorData
    let _discovered = event_rx.recv().await.unwrap();
    let _sensor_data = event_rx.recv().await.unwrap();

    // Send config frame for the same device (true_sensor_type 261)
    let config_bytes = build_config_frame_bytes(device, 261);
    bytes_tx.send(Ok(config_bytes)).await.unwrap();

    // Assert: AdapterEvent::DeviceConfig received
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
        .await
        .expect("timed out waiting for DeviceConfig event")
        .expect("event_rx closed");

    match event {
        AdapterEvent::DeviceConfig {
            device_key, config, ..
        } => {
            assert_eq!(
                device_key.as_str(),
                "bravepi-mainboard:246880020140018b:temperature"
            );
            assert_eq!(config.firmware_version, Some("1.2.3".to_string()));
            assert_eq!(config.uplink_interval_secs, Some(60));
            assert_eq!(
                config.properties.get("timezone"),
                Some(&ConfigValue::Integer(9))
            );
        }
        other => panic!("expected DeviceConfig, got {:?}", other),
    }

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn set_output_to_contact_output_device_produces_downlink_bytes() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "/dev/test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        None,
    ));

    // Discover a contact_output device
    let device: u64 = 0x1234567890abcdef;
    let frame_bytes = build_sensor_frame_bytes(device, 258, -70, 100, 2, &[0x00, 0x01]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();
    let _ = event_rx.recv().await.unwrap(); // DeviceDiscovered
    let _ = event_rx.recv().await.unwrap(); // SensorData

    // Send SetOutput command
    command_tx
        .send(AdapterCommand::DeviceCommand(
            iotkit_core_types::DeviceCommand {
                device_key: iotkit_core_types::DeviceKey::new(
                    "bravepi-mainboard:1234567890abcdef:contact_output",
                ),
                payload: iotkit_core_types::DeviceCommandPayload::SetOutput {
                    value: true,
                    duration_ms: Some(5000),
                },
            },
        ))
        .await
        .unwrap();

    let bytes = tokio::time::timeout(std::time::Duration::from_secs(1), write_rx.recv())
        .await
        .expect("should receive within timeout")
        .expect("write channel should have data");

    assert!(!bytes.is_empty());
    assert_eq!(bytes[0], 0x00); // downlink direction

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn query_config_produces_downlink_bytes() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "/dev/test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        None,
    ));

    // Discover a device
    let device: u64 = 0x246880020140018b;
    let frame_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();
    let _ = event_rx.recv().await.unwrap(); // DeviceDiscovered
    let _ = event_rx.recv().await.unwrap(); // SensorData

    // Send QueryConfig command
    command_tx
        .send(AdapterCommand::DeviceCommand(
            iotkit_core_types::DeviceCommand {
                device_key: iotkit_core_types::DeviceKey::new(
                    "bravepi-mainboard:246880020140018b:temperature",
                ),
                payload: iotkit_core_types::DeviceCommandPayload::QueryConfig,
            },
        ))
        .await
        .unwrap();

    let bytes = tokio::time::timeout(std::time::Duration::from_secs(1), write_rx.recv())
        .await
        .expect("should receive within timeout")
        .expect("write channel should have data");

    assert!(!bytes.is_empty());
    assert_eq!(bytes[0], 0x00); // downlink direction

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn config_frame_for_undiscovered_device_is_dropped() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);
    let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>(16);

    let handle = tokio::spawn(event_loop(
        "bravepi-mainboard:/dev/ttyAMA0".into(),
        "/dev/test".into(),
        bytes_rx,
        event_tx,
        command_rx,
        write_tx,
        None,
    ));

    // Send config frame WITHOUT any prior discovery
    let device: u64 = 0x246880020140018b;
    let config_bytes = build_config_frame_bytes(device, 261);
    bytes_tx.send(Ok(config_bytes)).await.unwrap();

    // Send Shutdown
    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();

    // Assert: no events received
    assert!(event_rx.recv().await.is_none());
}
