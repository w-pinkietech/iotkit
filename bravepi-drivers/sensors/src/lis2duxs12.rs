//! LIS2DUXS12 加速度センサー (sensor_type = 262)
//!
//! I2C: Int16LE × 3 × 0.244 → mG, magnitude 計算
//! UART (BravePI): Float32LE × 3 → mG, magnitude 計算

use crate::reading::{ConnectionType, SensorIdentity, SensorReading, SensorType};

const SENSOR_TYPE: SensorType = SensorType::Acceleration;

pub fn identity(connection_type: ConnectionType) -> SensorIdentity {
    SensorIdentity {
        manufacturer: "Braveridge",
        ic_part_number: "LIS2DUXS12",
        sensor_type: SENSOR_TYPE,
        connection_type,
    }
}

/// 生値 → mG の変換係数
const MG_SCALE: f64 = 0.244;

/// I2C レジスタアドレス
pub const REG_WHO_AM_I: u8 = 0x0F;
pub const REG_OUT: u8 = 0x28;

/// WHO_AM_I の期待値
pub const WHO_AM_I_VALUE: u8 = 0x47;

/// magnitude 計算: |sqrt(x² + y² + z²) - 1000| / 1000
/// x, y, z は mG 単位
fn magnitude(x: f64, y: f64, z: f64) -> f64 {
    ((x * x + y * y + z * z).sqrt() - 1000.0).abs() / 1000.0
}

/// I2C 生レジスタ値（6 bytes, Int16LE × 3）から mG + magnitude に変換。
pub fn from_i2c_raw(data: &[u8; 6]) -> SensorReading {
    let x = i16::from_le_bytes([data[0], data[1]]) as f64 * MG_SCALE;
    let y = i16::from_le_bytes([data[2], data[3]]) as f64 * MG_SCALE;
    let z = i16::from_le_bytes([data[4], data[5]]) as f64 * MG_SCALE;
    let mag = magnitude(x, y, z);
    SensorReading::new(SENSOR_TYPE, vec![x / 1000.0, y / 1000.0, z / 1000.0, mag])
}

/// UART (BravePI) フレームのペイロードから変換。
/// Float32LE × 3 (mG 単位)。
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 12 {
        return SensorReading::empty(SENSOR_TYPE);
    }
    let x = f32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64;
    let y = f32::from_le_bytes([data[4], data[5], data[6], data[7]]) as f64;
    let z = f32::from_le_bytes([data[8], data[9], data[10], data[11]]) as f64;
    let mag = magnitude(x, y, z);
    SensorReading::new(SENSOR_TYPE, vec![x / 1000.0, y / 1000.0, z / 1000.0, mag])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i2c_raw_1g_z_axis() {
        // 1G on Z axis ≈ 4098 raw (4098 * 0.244 ≈ 1000 mG)
        let raw_z = 4098i16;
        let data = [
            0, 0,  // X = 0
            0, 0,  // Y = 0
            raw_z.to_le_bytes()[0], raw_z.to_le_bytes()[1],  // Z ≈ 1000 mG
        ];
        let reading = from_i2c_raw(&data);
        assert_eq!(reading.values.len(), 4);
        // Z should be ≈ 1.0 (G)
        assert!((reading.values[2] - 1.0).abs() < 0.01);
        // magnitude should be ≈ 0.0 (sitting still at 1G)
        assert!(reading.values[3] < 0.01);
    }

    #[test]
    fn uart_payload() {
        let mut data = Vec::new();
        data.extend_from_slice(&100.0f32.to_le_bytes());  // X = 100 mG
        data.extend_from_slice(&200.0f32.to_le_bytes());  // Y = 200 mG
        data.extend_from_slice(&950.0f32.to_le_bytes());  // Z = 950 mG
        let reading = from_uart_payload(&data);
        assert_eq!(reading.values.len(), 4);
        assert!((reading.values[0] - 0.1).abs() < 0.01);   // X in G
        assert!((reading.values[1] - 0.2).abs() < 0.01);   // Y in G
        assert!((reading.values[2] - 0.95).abs() < 0.01);  // Z in G
    }

    #[test]
    fn both_sources_same_sensor_type() {
        let i2c = from_i2c_raw(&[0, 0, 0, 0, 0, 0]);
        let uart = from_uart_payload(&[0u8; 12]);
        assert_eq!(i2c.sensor_type, uart.sensor_type);
    }
}
