//! MCP9600 熱電対センサー (sensor_type = 261)
//!
//! I2C: Int16 BE × 0.0625 → ℃
//! UART (BravePI): Float32LE → ℃

use crate::UartSample;
use iotkit_core_types::{ConnectionInfo, SensorIdentity, SensorReading, SensorType};

fn sensor_type() -> SensorType {
    SensorType::Temperature
}

pub const MANUFACTURER: &str = "Microchip";
pub const IC_PART_NUMBER: &str = "MCP9600";

/// センサーの素性を返す。
pub fn identity(connection: ConnectionInfo) -> SensorIdentity {
    SensorIdentity {
        manufacturer: MANUFACTURER.into(),
        ic_part_number: IC_PART_NUMBER.into(),
        sensor_type: sensor_type(),
        connection,
    }
}

/// I2C レジスタアドレス
pub const REG_HOT_JUNCTION: u8 = 0x00;
pub const REG_STATUS: u8 = 0x04;
pub const REG_SENSOR_CONFIGURATION: u8 = 0x05;
pub const REG_DEVICE_ID: u8 = 0x20;

/// I2C のデバイスID
pub const DEVICE_ID: u8 = 0x40;

/// 熱電対タイプ
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum ThermocoupleType {
    K = 0,
    J = 1,
    T = 2,
    N = 3,
    S = 4,
    E = 5,
    B = 6,
    R = 7,
}

/// 熱電対タイプからコンフィグレジスタ値を作成
pub fn config_value(tc_type: ThermocoupleType) -> u8 {
    (tc_type as u8) << 4
}

/// I2C 生レジスタ値（2byte big-endian）から ℃ に変換。
pub fn from_i2c_raw(data: &[u8; 2]) -> SensorReading {
    let raw = i16::from_be_bytes(*data);
    let temp = raw as f64 * 0.0625;
    SensorReading::new(sensor_type(), vec![temp], vec!["celsius".to_string()])
}

/// UART (BravePI) フレームのペイロードから変換。
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 4 {
        return SensorReading::empty(sensor_type());
    }
    let temp = f32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64;
    SensorReading::new(sensor_type(), vec![temp], vec!["celsius".to_string()])
}

fn decode_uart(sample: UartSample<'_>) -> SensorReading {
    from_uart_payload(sample.payload)
}

pub const HANDLER: crate::SensorHandler = crate::SensorHandler {
    sensor_type: SensorType::Temperature,
    key_suffix: "temperature",
    identity,
    decode_uart,
};

#[cfg(test)]
#[path = "../tests/unit/mcp9600_tests.rs"]
mod tests;
