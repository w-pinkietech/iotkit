//! ContactInput / ContactOutput endpoint decoder。
//! IC ドライバーではなく、module-level の endpoint として扱う。

use iotkit_core_types::{ConnectionInfo, SensorIdentity, SensorReading, SensorType};

use crate::{SensorHandler, UartSample};

fn decode_values(sample: &UartSample<'_>) -> Vec<f64> {
    sample
        .payload
        .iter()
        .take(sample.data_count as usize)
        .map(|&b| if b != 0 { 1.0 } else { 0.0 })
        .collect()
}

fn decode_contact_input(sample: UartSample<'_>) -> SensorReading {
    SensorReading::new(SensorType::ContactInput, decode_values(&sample), Vec::<String>::new())
}

fn decode_contact_output(sample: UartSample<'_>) -> SensorReading {
    SensorReading::new(SensorType::ContactOutput, decode_values(&sample), Vec::<String>::new())
}

fn contact_input_identity(connection: ConnectionInfo) -> SensorIdentity {
    SensorIdentity {
        manufacturer: "Braveridge".to_string(),
        ic_part_number: "Contact Input Module".to_string(),
        sensor_type: SensorType::ContactInput,
        connection,
    }
}

fn contact_output_identity(connection: ConnectionInfo) -> SensorIdentity {
    SensorIdentity {
        manufacturer: "Braveridge".to_string(),
        ic_part_number: "Contact Output Module".to_string(),
        sensor_type: SensorType::ContactOutput,
        connection,
    }
}

pub const CONTACT_INPUT: SensorHandler = SensorHandler {
    sensor_type: SensorType::ContactInput,
    key_suffix: "contact_input",
    identity: contact_input_identity,
    decode_uart: decode_contact_input,
};

pub const CONTACT_OUTPUT: SensorHandler = SensorHandler {
    sensor_type: SensorType::ContactOutput,
    key_suffix: "contact_output",
    identity: contact_output_identity,
    decode_uart: decode_contact_output,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use iotkit_core_types::ConnectionKind;

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
}
