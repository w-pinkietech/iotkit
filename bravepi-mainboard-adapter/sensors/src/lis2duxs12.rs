//! LIS2DUXS12 加速度センサー (sensor_type = 262)
//!
//! I2C: Int16LE × 3 × 0.244 → mG
//! UART (BravePI): Float32LE × 3 → mG

use iotkit_core_types::{ConnectionInfo, SensorIdentity, SensorReading, SensorType};
use crate::UartSample;

fn sensor_type() -> SensorType { SensorType::Acceleration }

pub const MANUFACTURER: &str = "Braveridge";
pub const IC_PART_NUMBER: &str = "LIS2DUXS12";

pub fn identity(connection: ConnectionInfo) -> SensorIdentity {
    SensorIdentity {
        manufacturer: MANUFACTURER.into(),
        ic_part_number: IC_PART_NUMBER.into(),
        sensor_type: sensor_type(),
        connection,
    }
}

/// 生値 → mG の変換係数
const MG_SCALE: f64 = 0.244;

/// I2C レジスタアドレス
pub const REG_WHO_AM_I: u8 = 0x0F;
pub const REG_OUT: u8 = 0x28;

/// WHO_AM_I の期待値
pub const WHO_AM_I_VALUE: u8 = 0x47;

/// I2C 生レジスタ値（6 bytes, Int16LE × 3）から mG に変換。
pub fn from_i2c_raw(data: &[u8; 6]) -> SensorReading {
    let x = i16::from_le_bytes([data[0], data[1]]) as f64 * MG_SCALE;
    let y = i16::from_le_bytes([data[2], data[3]]) as f64 * MG_SCALE;
    let z = i16::from_le_bytes([data[4], data[5]]) as f64 * MG_SCALE;
    SensorReading::new(sensor_type(), vec![x, y, z], vec!["x_mg".to_string(), "y_mg".to_string(), "z_mg".to_string()])
}

/// UART (BravePI) フレームのペイロードから変換。
/// Float32LE × 3 (mG 単位)。
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 12 {
        return SensorReading::empty(sensor_type());
    }
    let x = f32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64;
    let y = f32::from_le_bytes([data[4], data[5], data[6], data[7]]) as f64;
    let z = f32::from_le_bytes([data[8], data[9], data[10], data[11]]) as f64;
    SensorReading::new(sensor_type(), vec![x, y, z], vec!["x_mg".to_string(), "y_mg".to_string(), "z_mg".to_string()])
}

fn decode_uart(sample: UartSample<'_>) -> SensorReading {
    from_uart_payload(sample.payload)
}

pub const HANDLER: crate::SensorHandler = crate::SensorHandler {
    sensor_type: SensorType::Acceleration,
    key_suffix: "acceleration",
    identity,
    decode_uart,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i2c_values_are_passed_through_in_mg() {
        // ワイヤはInt16LE×3×0.244(mG)。ドライバは単位変換せず素通しする。
        let raw_x = 50i16;
        let raw_y = -100i16;
        let raw_z = 4090i16;
        let data = [
            raw_x.to_le_bytes()[0], raw_x.to_le_bytes()[1],
            raw_y.to_le_bytes()[0], raw_y.to_le_bytes()[1],
            raw_z.to_le_bytes()[0], raw_z.to_le_bytes()[1],
        ];
        let reading = from_i2c_raw(&data);
        assert_eq!(reading.values.len(), 3, "派生値はワイヤに乗せない(D6決定11)");
        assert!((reading.values[0] - 12.2).abs() < 1e-3);
        assert!((reading.values[1] - (-24.4)).abs() < 1e-3);
        assert!((reading.values[2] - 997.96).abs() < 1e-3);
        assert_eq!(reading.labels, vec!["x_mg", "y_mg", "z_mg"]);
    }

    #[test]
    fn uart_values_are_passed_through_in_mg() {
        // ワイヤはFloat32LE×3(mG)。ドライバは単位変換せず素通しする(D4: データシートの数学のみ)。
        // 旧実装は÷1000でg化し旧ブリッジが×1000で戻す往復変換をしていた(計画3で解消)。
        let mut payload = Vec::new();
        for v in [12.0f32, -34.0, 998.0] {
            payload.extend_from_slice(&v.to_le_bytes());
        }
        let reading = from_uart_payload(&payload);
        assert_eq!(reading.values.len(), 3, "派生値はワイヤに乗せない(D6決定11)");
        assert!((reading.values[0] - 12.0).abs() < 1e-3);
        assert!((reading.values[2] - 998.0).abs() < 1e-3);
        assert_eq!(reading.labels, vec!["x_mg", "y_mg", "z_mg"]);
    }

    #[test]
    fn both_sources_same_sensor_type() {
        let i2c = from_i2c_raw(&[0, 0, 0, 0, 0, 0]);
        let uart = from_uart_payload(&[0u8; 12]);
        assert_eq!(i2c.sensor_type, uart.sensor_type);
    }
}
