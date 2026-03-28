//! SDP810-500Pa 差圧センサー (sensor_type = 263)
//!
//! I2C: 9byte読み → CRC検証 → dp/scale_factor → Pa
//! UART (BravePI): Float32LE → Pa

use iotkit_core_types::{ConnectionInfo, SensorIdentity, SensorReading, SensorType};
use crate::UartSample;

fn sensor_type() -> SensorType { SensorType::DifferentialPressure }

pub const MANUFACTURER: &str = "Braveridge";
pub const IC_PART_NUMBER: &str = "SDP810";

pub fn identity(connection: ConnectionInfo) -> SensorIdentity {
    SensorIdentity {
        manufacturer: MANUFACTURER.into(),
        ic_part_number: IC_PART_NUMBER.into(),
        sensor_type: sensor_type(),
        connection,
    }
}

/// I2C のアドレス
pub const I2C_ADDRESS: u8 = 0x25;

/// プロダクト番号
pub const PRODUCT_NUMBER: u32 = 0x03020A01;

/// CRC-8 検証（SDP810 仕様: polynomial 0x31, init 0xFF）
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0xFF;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            if crc & 0x80 != 0 {
                crc = (crc << 1) ^ 0x31;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// I2C 生データ（9 bytes）から Pa に変換。
/// data[0:2] = differential pressure (BE i16), data[2] = CRC
/// data[6:8] = scale factor (BE i16), data[8] = CRC
pub fn from_i2c_raw(data: &[u8; 9]) -> SensorReading {
    // CRC 検証
    if crc8(&data[0..2]) != data[2] {
        return SensorReading::empty(sensor_type());
    }
    if crc8(&data[6..8]) != data[8] {
        return SensorReading::empty(sensor_type());
    }

    let dp = i16::from_be_bytes([data[0], data[1]]) as f64;
    let scale_factor = i16::from_be_bytes([data[6], data[7]]) as f64;

    if scale_factor == 0.0 {
        return SensorReading::empty(sensor_type());
    }

    let pressure = dp / scale_factor;
    SensorReading::new(sensor_type(), vec![pressure], vec!["pascal"])
}

/// UART (BravePI) フレームのペイロードから変換。
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 4 {
        return SensorReading::empty(sensor_type());
    }
    let pa = f32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64;
    SensorReading::new(sensor_type(), vec![pa], vec!["pascal"])
}

fn decode_uart(sample: UartSample<'_>) -> SensorReading {
    from_uart_payload(sample.payload)
}

pub const HANDLER: crate::SensorHandler = crate::SensorHandler {
    sensor_type: SensorType::DifferentialPressure,
    key_suffix: "differential_pressure",
    identity,
    decode_uart,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc8_known() {
        // SDP810 CRC example
        assert_eq!(crc8(&[0x00, 0x00]), 0x81);
    }

    #[test]
    fn i2c_raw_with_valid_crc() {
        // dp = 100 (0x0064), scale = 60 (0x003C)
        // pressure = 100/60 = 1.6667 Pa
        let dp_bytes = 100i16.to_be_bytes();
        let scale_bytes = 60i16.to_be_bytes();
        let data: [u8; 9] = [
            dp_bytes[0], dp_bytes[1], crc8(&dp_bytes),
            0x00, 0x00, 0x00, // bytes 3-5 (temperature, unused here)
            scale_bytes[0], scale_bytes[1], crc8(&scale_bytes),
        ];
        let reading = from_i2c_raw(&data);
        assert!((reading.values[0] - 1.6667).abs() < 0.01);
    }

    #[test]
    fn i2c_raw_bad_crc_returns_empty() {
        let data: [u8; 9] = [0x00, 0x64, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x00];
        let reading = from_i2c_raw(&data);
        assert!(reading.values.is_empty());
    }

    #[test]
    fn uart_payload() {
        let reading = from_uart_payload(&123.456f32.to_le_bytes());
        assert!((reading.values[0] - 123.456).abs() < 0.01);
    }
}
