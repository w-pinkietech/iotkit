use bravepi_adapter::task::frame_to_event;
use bravepi_codec::codec::{BravePiFrame, ConfigFrame, SensorFrame};
use iotkit_core_types::{AdapterEvent, SensorType};

// ── ヘルパー ──────────────────────────────────────

fn make_sensor_frame(sensor_type_raw: u16, value_data: Vec<u8>) -> SensorFrame {
    SensorFrame {
        device_number: "246880020140018b".to_string(),
        sensor_type_raw,
        rssi: -60,
        battery: 95,
        data_count: 1,
        value_data,
    }
}

// ── Temperature (261, mcp9600) ──────────────────

#[test]
fn temperature_frame_produces_sensor_data() {
    // mcp9600 uart payload: 4 bytes, IEEE 754 float LE
    // 0x00 0x80 0xb3 0x41 = 22.4375°C
    let frame = BravePiFrame::Sensor(make_sensor_frame(261, vec![0x00, 0x80, 0xb3, 0x41]));
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData {
            device_key,
            reading,
            rssi,
            battery_pct,
        } => {
            assert_eq!(device_key.0, "246880020140018b");
            assert_eq!(reading.sensor_type, SensorType::Temperature);
            assert_eq!(reading.values.len(), 1);
            assert!((reading.values[0] - 22.4375).abs() < 0.01);
            assert_eq!(rssi, Some(-60));
            assert_eq!(battery_pct, Some(95));
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── Illuminance (264, opt3001) ──────────────────

#[test]
fn illuminance_frame_produces_sensor_data() {
    // opt3001 uart payload: Float32LE (メインボード変換済み)
    // 500.0f32 = 0x43fa0000
    let lux_bytes = 500.0f32.to_le_bytes().to_vec();
    let frame = BravePiFrame::Sensor(make_sensor_frame(264, lux_bytes));
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData { reading, .. } => {
            assert_eq!(reading.sensor_type, SensorType::Illuminance);
            assert_eq!(reading.values.len(), 1);
            assert!((reading.values[0] - 500.0).abs() < 0.1);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── ContactInput (257) ──────────────────────────

#[test]
fn contact_input_frame_maps_bytes_to_float() {
    let frame = BravePiFrame::Sensor(SensorFrame {
        device_number: "aabbccdd00112233".to_string(),
        sensor_type_raw: 257,
        rssi: -50,
        battery: 80,
        data_count: 3,
        value_data: vec![0x01, 0x00, 0x01, 0xff], // 4 bytes だが data_count=3 なので 3 つだけ使う
    });
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData { reading, .. } => {
            assert_eq!(reading.sensor_type, SensorType::ContactInput);
            assert_eq!(reading.values, vec![1.0, 0.0, 1.0]);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── ContactOutput (258) ─────────────────────────

#[test]
fn contact_output_frame_produces_sensor_data() {
    let frame = BravePiFrame::Sensor(SensorFrame {
        device_number: "1234567890abcdef".to_string(),
        sensor_type_raw: 258,
        rssi: -70,
        battery: 100,
        data_count: 2,
        value_data: vec![0x00, 0x01],
    });
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData { reading, .. } => {
            assert_eq!(reading.sensor_type, SensorType::ContactOutput);
            assert_eq!(reading.values, vec![0.0, 1.0]);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── Unknown sensor type → None ──────────────────

#[test]
fn unknown_sensor_type_returns_none() {
    let frame = BravePiFrame::Sensor(make_sensor_frame(9999, vec![0x01, 0x02]));
    assert!(frame_to_event(frame, "/dev/test").is_none());
}

// ── ConfigFrame → None (PoC) ────────────────────

#[test]
fn config_frame_returns_none() {
    let frame = BravePiFrame::Config(ConfigFrame {
        device_number: "246880020140018b".to_string(),
        rssi: -55,
        true_sensor_type: 261,
        firmware_version: "1.2.3".to_string(),
        timezone: 9,
        ble_mode: 1,
        tx_power: 4,
        advertise_interval: 1000,
        uplink_interval: 60,
    });
    assert!(frame_to_event(frame, "/dev/test").is_none());
}

// ── DecodeError → AdapterError ──────────────────

#[test]
fn decode_error_produces_adapter_error() {
    let frame = BravePiFrame::DecodeError {
        device_number: "bad_device".to_string(),
        sensor_type_raw: 261,
        reason: "payload too short".to_string(),
    };
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::AdapterError { device_key, error } => {
            assert_eq!(device_key.unwrap().0, "bad_device");
            assert!(error.contains("Decode error"));
            assert!(error.contains("payload too short"));
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }
}

// ── Ranging (260, vl53l1x) ──────────────────────

#[test]
fn ranging_frame_produces_sensor_data() {
    // vl53l1x uart payload: 2 bytes LE → u16 distance mm
    // 0xE8 0x03 = 1000mm
    let frame = BravePiFrame::Sensor(make_sensor_frame(260, vec![0xe8, 0x03]));
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData { reading, .. } => {
            assert_eq!(reading.sensor_type, SensorType::Ranging);
            assert!(!reading.values.is_empty());
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── ADC (259, mcp3427) ──────────────────────────

#[test]
fn adc_frame_produces_sensor_data() {
    // mcp3427 uart payload: 4 bytes IEEE 754 float LE
    let frame = BravePiFrame::Sensor(make_sensor_frame(259, vec![0x00, 0x00, 0x80, 0x3f]));
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData { reading, .. } => {
            assert_eq!(reading.sensor_type, SensorType::Adc);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── rssi / battery が正しく伝搬される ───────────

#[test]
fn rssi_and_battery_are_propagated() {
    let frame = BravePiFrame::Sensor(SensorFrame {
        device_number: "test_device".to_string(),
        sensor_type_raw: 257, // ContactInput
        rssi: -128,           // i8 の最小値
        battery: 0,
        data_count: 1,
        value_data: vec![0x01],
    });
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData {
            rssi, battery_pct, ..
        } => {
            assert_eq!(rssi, Some(-128));
            assert_eq!(battery_pct, Some(0));
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── 空の value_data でもパニックしない ────────────

#[test]
fn empty_value_data_does_not_panic() {
    // Temperature (mcp9600) に空データを渡す → empty reading が返るはず
    let frame = BravePiFrame::Sensor(make_sensor_frame(261, vec![]));
    let event = frame_to_event(frame, "/dev/test");
    // パニックしないことが目的。結果は SensorData(empty) になるはず。
    assert!(event.is_some());
}

// ── ContactInput で data_count > value_data.len() ─

#[test]
fn contact_input_data_count_exceeds_data_does_not_panic() {
    let frame = BravePiFrame::Sensor(SensorFrame {
        device_number: "test".to_string(),
        sensor_type_raw: 257,
        rssi: -50,
        battery: 50,
        data_count: 100, // value_data より大きい
        value_data: vec![0x01, 0x00],
    });
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData { reading, .. } => {
            // take(100) but only 2 bytes → 2 values
            assert_eq!(reading.values.len(), 2);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── SensorIdentity tests ────────────────────────

#[test]
fn temperature_frame_returns_identity() {
    let frame = BravePiFrame::Sensor(make_sensor_frame(261, vec![0x00, 0x80, 0xb3, 0x41]));
    let (_event, identity) = frame_to_event(frame, "/dev/ttyAMA0").expect("should produce event");

    let identity = identity.expect("temperature should have identity");
    assert_eq!(identity.manufacturer, "Braveridge");
    assert_eq!(identity.ic_part_number, "MCP9600");
    assert_eq!(identity.sensor_type, SensorType::Temperature);
    assert_eq!(identity.connection.kind, iotkit_core_types::ConnectionKind::Uart);
    assert_eq!(identity.connection.parameters.get("port").unwrap(), "/dev/ttyAMA0");
    assert_eq!(identity.connection.parameters.get("transmitter_id").unwrap(), "246880020140018b");
}

#[test]
fn contact_input_has_no_identity() {
    let frame = BravePiFrame::Sensor(SensorFrame {
        device_number: "test".to_string(),
        sensor_type_raw: 257,
        rssi: -50,
        battery: 80,
        data_count: 1,
        value_data: vec![0x01],
    });
    let (_event, identity) = frame_to_event(frame, "/dev/test").expect("should produce event");
    assert!(identity.is_none());
}
