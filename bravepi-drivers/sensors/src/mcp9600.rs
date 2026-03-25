//! MCP9600 熱電対センサー (sensor_type = 261)
//!
//! I2C: Int16 BE × 0.0625 → ℃
//! UART (BravePI): Float32LE → ℃

use crate::reading::{ConnectionType, SensorIdentity, SensorReading, SensorType};

const SENSOR_TYPE: SensorType = SensorType::Temperature;

/// センサーの素性を返す。hardware_id は呼び出し元が設定する。
pub fn identity(connection_type: ConnectionType) -> SensorIdentity {
    SensorIdentity {
        manufacturer: "Braveridge",
        ic_part_number: "MCP9600",
        sensor_type: SENSOR_TYPE,
        connection_type,
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
    K = 0, J = 1, T = 2, N = 3,
    S = 4, E = 5, B = 6, R = 7,
}

/// 熱電対タイプからコンフィグレジスタ値を作成
pub fn config_value(tc_type: ThermocoupleType) -> u8 {
    (tc_type as u8) << 4
}

/// I2C 生レジスタ値（2byte big-endian）から ℃ に変換。
pub fn from_i2c_raw(data: &[u8; 2]) -> SensorReading {
    let raw = i16::from_be_bytes(*data);
    let temp = raw as f64 * 0.0625;
    SensorReading::new(SENSOR_TYPE, vec![temp])
}

/// UART (BravePI) フレームのペイロードから変換。
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 4 {
        return SensorReading::empty(SENSOR_TYPE);
    }
    let temp = f32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64;
    SensorReading::new(SENSOR_TYPE, vec![temp])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i2c_raw_positive_temp() {
        // 25.0℃ = 400 × 0.0625 = 25.0
        // 400 in BE = [0x01, 0x90]
        let reading = from_i2c_raw(&[0x01, 0x90]);
        assert!((reading.values[0] - 25.0).abs() < 0.01);
    }

    #[test]
    fn i2c_raw_negative_temp() {
        // -10.0℃ = -160 × 0.0625 = -10.0
        // -160 in BE i16 = [0xFF, 0x60]
        let reading = from_i2c_raw(&[0xFF, 0x60]);
        assert!((reading.values[0] - (-10.0)).abs() < 0.01);
    }

    #[test]
    fn uart_payload_real_capture() {
        // 実機キャプチャ: 22.25℃ = 0x41B30000 LE
        let reading = from_uart_payload(&[0x00, 0x00, 0xB2, 0x41]);
        assert!((reading.values[0] - 22.25).abs() < 0.1);
    }

    #[test]
    fn both_sources_agree() {
        // 22.0℃
        // I2C: 22.0 / 0.0625 = 352 → [0x01, 0x60]
        let i2c = from_i2c_raw(&[0x01, 0x60]);

        // UART: 22.0 as f32 LE
        let uart = from_uart_payload(&22.0f32.to_le_bytes());

        assert!((i2c.values[0] - uart.values[0]).abs() < 0.1);
        assert_eq!(i2c.sensor_type, uart.sensor_type);
    }
}
