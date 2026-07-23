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
    SensorReading::new(SensorType::ContactInput, decode_values(&sample), vec![])
}

fn decode_contact_output(sample: UartSample<'_>) -> SensorReading {
    SensorReading::new(SensorType::ContactOutput, decode_values(&sample), vec![])
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
#[path = "../tests/unit/contact_tests.rs"]
mod tests;
