//! SDP810-500Pa 差圧センサー (sensor_type = 263)
//!
//! I2C: 9byte読み → CRC検証 → dp/scale_factor → Pa
//! UART (BravePI): Float32LE → Pa

use crate::UartSample;
use iotkit_core_types::{ConnectionInfo, SensorIdentity, SensorReading, SensorType};

fn sensor_type() -> SensorType {
    SensorType::DifferentialPressure
}

pub const MANUFACTURER: &str = "Sensirion";
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
    SensorReading::new(sensor_type(), vec![pressure], vec!["pascal".to_string()])
}

/// UART (BravePI) フレームのペイロードから変換。
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 4 {
        return SensorReading::empty(sensor_type());
    }
    let pa = f32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64;
    SensorReading::new(sensor_type(), vec![pa], vec!["pascal".to_string()])
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
#[path = "../tests/unit/sdp810_tests.rs"]
mod tests;
