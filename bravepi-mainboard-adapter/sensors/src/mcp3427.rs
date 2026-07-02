//! MCP3427 ADC センサー (sensor_type = 259)
//!
//! I2C: MCP342x ライブラリ → Volt → mV (×1000)
//! UART (BravePI): Int16LE × 2ch → mV

use iotkit_core_types::{ConnectionInfo, SensorIdentity, SensorReading, SensorType};
use crate::UartSample;

fn sensor_type() -> SensorType { SensorType::Adc }

pub const MANUFACTURER: &str = "Braveridge";
pub const IC_PART_NUMBER: &str = "MCP3427";

pub fn identity(connection: ConnectionInfo) -> SensorIdentity {
    SensorIdentity {
        manufacturer: MANUFACTURER.into(),
        ic_part_number: IC_PART_NUMBER.into(),
        sensor_type: sensor_type(),
        connection,
    }
}

/// I2C のアドレス候補
pub const I2C_ADDRESSES: [u8; 3] = [0x68, 0x6B, 0x6F];

/// I2C 経由の電圧値（Volt）から mV に変換。2ch 分。
pub fn from_i2c_volts(ch1_volt: f64, ch2_volt: f64) -> SensorReading {
    SensorReading::new(sensor_type(), vec![ch1_volt * 1000.0, ch2_volt * 1000.0], vec!["ch1_volt".to_string(), "ch2_volt".to_string()])
}

/// UART (BravePI) フレームのペイロードから変換。
/// Int16LE × 2ch per sample。
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 4 {
        return SensorReading::empty(sensor_type());
    }
    let ch1 = i16::from_le_bytes([data[0], data[1]]) as f64;
    let ch2 = i16::from_le_bytes([data[2], data[3]]) as f64;
    SensorReading::new(sensor_type(), vec![ch1, ch2], vec!["ch1_volt".to_string(), "ch2_volt".to_string()])
}

fn decode_uart(sample: UartSample<'_>) -> SensorReading {
    from_uart_payload(sample.payload)
}

pub const HANDLER: crate::SensorHandler = crate::SensorHandler {
    sensor_type: SensorType::Adc,
    key_suffix: "adc",
    identity,
    decode_uart,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i2c_volts_to_mv() {
        let reading = from_i2c_volts(1.5, -0.8);
        assert!((reading.values[0] - 1500.0).abs() < 0.1);
        assert!((reading.values[1] - (-800.0)).abs() < 0.1);
    }

    #[test]
    fn uart_payload() {
        // ch1=500, ch2=-300
        let mut data = Vec::new();
        data.extend_from_slice(&500i16.to_le_bytes());
        data.extend_from_slice(&(-300i16).to_le_bytes());
        let reading = from_uart_payload(&data);
        assert!((reading.values[0] - 500.0).abs() < 0.1);
        assert!((reading.values[1] - (-300.0)).abs() < 0.1);
    }
}
