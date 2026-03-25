//! VL53L1X 測距センサー (sensor_type = 260)
//!
//! I2C: qwiic ライブラリ → mm (int, 0=無効, cap 2000)
//! UART (BravePI): UInt16LE → mm

use crate::reading::{BravepiConnection, SensorIdentity, SensorReading, SensorType};

fn sensor_type() -> SensorType { SensorType::Ranging }

pub fn identity(connection: BravepiConnection) -> SensorIdentity {
    SensorIdentity {
        manufacturer: "Braveridge".into(),
        ic_part_number: "VL53L1X".into(),
        sensor_type: sensor_type(),
        connection: connection.to_connection_info(),
    }
}
const MAX_DISTANCE_MM: u16 = 2000;

/// I2C のアドレス
pub const I2C_ADDRESS: u8 = 0x29;

/// I2C 経由の測距値（mm）から変換。0 は無効値として空を返す。
pub fn from_i2c_distance(distance_mm: u16) -> SensorReading {
    if distance_mm == 0 {
        return SensorReading::empty(sensor_type());
    }
    let capped = distance_mm.min(MAX_DISTANCE_MM);
    SensorReading::new(sensor_type(), vec![capped as f64])
}

/// UART (BravePI) フレームのペイロードから変換。
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 2 {
        return SensorReading::empty(sensor_type());
    }
    let mm = u16::from_le_bytes([data[0], data[1]]);
    if mm == 0 {
        return SensorReading::empty(sensor_type());
    }
    SensorReading::new(sensor_type(), vec![mm.min(MAX_DISTANCE_MM) as f64])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i2c_normal() {
        let reading = from_i2c_distance(750);
        assert!((reading.values[0] - 750.0).abs() < 0.1);
    }

    #[test]
    fn i2c_cap_at_2000() {
        let reading = from_i2c_distance(3000);
        assert!((reading.values[0] - 2000.0).abs() < 0.1);
    }

    #[test]
    fn i2c_zero_is_empty() {
        let reading = from_i2c_distance(0);
        assert!(reading.values.is_empty());
    }

    #[test]
    fn uart_payload() {
        let reading = from_uart_payload(&750u16.to_le_bytes());
        assert!((reading.values[0] - 750.0).abs() < 0.1);
    }

    #[test]
    fn both_sources_agree() {
        let i2c = from_i2c_distance(750);
        let uart = from_uart_payload(&750u16.to_le_bytes());
        assert_eq!(i2c, uart);
    }
}
