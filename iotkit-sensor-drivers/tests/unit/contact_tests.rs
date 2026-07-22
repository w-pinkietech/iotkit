use super::*;
use iotkit_core_types::ConnectionKind;
use std::collections::BTreeMap;

fn test_conn() -> ConnectionInfo {
    ConnectionInfo {
        kind: ConnectionKind::Uart,
        parameters: BTreeMap::from([
            ("port".into(), "/dev/test".into()),
            ("transmitter_id".into(), "test123".into()),
        ]),
    }
}

#[test]
fn contact_input_decode_maps_bytes_to_float() {
    let sample = UartSample {
        payload: &[0x01, 0x00, 0x01, 0xFF],
        data_count: 3,
    };
    let reading = decode_contact_input(sample);
    assert_eq!(reading.sensor_type, SensorType::ContactInput);
    assert_eq!(reading.values, vec![1.0, 0.0, 1.0]);
}

#[test]
fn contact_output_decode_maps_bytes_to_float() {
    let sample = UartSample {
        payload: &[0x00, 0x01],
        data_count: 2,
    };
    let reading = decode_contact_output(sample);
    assert_eq!(reading.sensor_type, SensorType::ContactOutput);
    assert_eq!(reading.values, vec![0.0, 1.0]);
}

#[test]
fn data_count_limits_values() {
    let sample = UartSample {
        payload: &[0x01, 0x00, 0x01],
        data_count: 2,
    };
    let reading = decode_contact_input(sample);
    assert_eq!(reading.values.len(), 2);
}

#[test]
fn data_count_exceeds_payload_does_not_panic() {
    let sample = UartSample {
        payload: &[0x01, 0x00],
        data_count: 100,
    };
    let reading = decode_contact_input(sample);
    assert_eq!(reading.values.len(), 2);
}

#[test]
fn contact_input_identity_is_correct() {
    let id = contact_input_identity(test_conn());
    assert_eq!(id.manufacturer, "Braveridge");
    assert_eq!(id.ic_part_number, "Contact Input Module");
    assert_eq!(id.sensor_type, SensorType::ContactInput);
    assert_eq!(id.connection.kind, ConnectionKind::Uart);
}

#[test]
fn contact_output_identity_is_correct() {
    let id = contact_output_identity(test_conn());
    assert_eq!(id.manufacturer, "Braveridge");
    assert_eq!(id.ic_part_number, "Contact Output Module");
    assert_eq!(id.sensor_type, SensorType::ContactOutput);
}
