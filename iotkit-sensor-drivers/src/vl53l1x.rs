//! VL53L1X 測距センサー (sensor_type = 260)
//!
//! I2C: qwiic ライブラリ → mm (int, 0=無効)
//! UART (BravePI): UInt16LE → mm

use crate::UartSample;
use iotkit_core_types::{ConnectionInfo, SensorIdentity, SensorReading, SensorType};

fn sensor_type() -> SensorType {
    SensorType::Ranging
}

pub const MANUFACTURER: &str = "STMicroelectronics";
pub const IC_PART_NUMBER: &str = "VL53L1X";

pub fn identity(connection: ConnectionInfo) -> SensorIdentity {
    SensorIdentity {
        manufacturer: MANUFACTURER.into(),
        ic_part_number: IC_PART_NUMBER.into(),
        sensor_type: sensor_type(),
        connection,
    }
}
// 値域のcapはしない: ドライバはデータシートの数学のみ(R3①)。値域判定はR8/カタログ物理限界
// (distance_mm: 0..4000)の仕事(D6決定8)。旧cap(2000)は実測3mを2mに改変する実データ破壊だった。

/// I2C のアドレス
pub const I2C_ADDRESS: u8 = 0x29;

/// I2C 経由の測距値（mm）から変換。0 は無効値として空を返す。
pub fn from_i2c_distance(distance_mm: u16) -> SensorReading {
    if distance_mm == 0 {
        return SensorReading::empty(sensor_type());
    }
    SensorReading::new(
        sensor_type(),
        vec![distance_mm as f64],
        vec!["distance_mm".to_string()],
    )
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
    SensorReading::new(
        sensor_type(),
        vec![mm as f64],
        vec!["distance_mm".to_string()],
    )
}

fn decode_uart(sample: UartSample<'_>) -> SensorReading {
    from_uart_payload(sample.payload)
}

pub const HANDLER: crate::SensorHandler = crate::SensorHandler {
    sensor_type: SensorType::Ranging,
    key_suffix: "ranging",
    identity,
    decode_uart,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i2c_normal() {
        let reading = from_i2c_distance(750);
        assert!((reading.values[0] - 750.0).abs() < 0.1);
    }

    #[test]
    fn i2c_long_range_is_preserved_not_capped() {
        // 旧実装は2000mmでcapし実測3mを改変していた(codex最終レビュー指摘)。
        // 値域判定はレジストリ(カタログ物理限界0..4000)の仕事。
        let reading = from_i2c_distance(3000);
        assert!((reading.values[0] - 3000.0).abs() < 0.1);
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
