//! OPT3001 照度センサー (sensor_type = 264)
//!
//! I2C: 独自フォーマット（指数+仮数） → Lux
//! UART (BravePI): Float32LE → Lux

use iotkit_core_types::{ConnectionInfo, SensorIdentity, SensorReading, SensorType};
use crate::UartSample;

fn sensor_type() -> SensorType { SensorType::Illuminance }

pub const MANUFACTURER: &str = "Braveridge";
pub const IC_PART_NUMBER: &str = "OPT3001";

pub fn identity(connection: ConnectionInfo) -> SensorIdentity {
    SensorIdentity {
        manufacturer: MANUFACTURER.into(),
        ic_part_number: IC_PART_NUMBER.into(),
        sensor_type: sensor_type(),
        connection,
    }
}

/// I2C のデバイスID (0x3001)
pub const DEVICE_ID: u16 = 0x3001;

/// I2C のレジスタアドレス
pub const REG_RESULT: u8 = 0x00;
pub const REG_CONFIG: u8 = 0x01;
pub const REG_DEVICE_ID: u8 = 0x7F;

/// I2C 初期化用コンフィグ値
pub const INIT_CONFIG: u16 = 0x10CC;

/// I2C 生レジスタ値から Lux に変換。
///
/// OPT3001 の Result レジスタは 16bit:
/// - bit[15:12] = exponent
/// - bit[11:0]  = fractional
///   ただし smbus2 の word_data はバイトスワップされて返るため、
///   Python コードでは独自のビット抽出をしている。
pub fn from_i2c_raw(raw: u16) -> SensorReading {
    // Python: exponent = (raw & 0x00F0) >> 4
    //         fractional = ((raw & 0xFF00) >> 8) + ((raw & 0x000F) << 8)
    // これは smbus2 の word_data がバイトスワップされている前提のパース
    let exponent = (raw & 0x00F0) >> 4;
    let fractional = ((raw & 0xFF00) >> 8) + ((raw & 0x000F) << 8);
    let lux = (1u32 << exponent) as f64 * fractional as f64 * 0.01;
    SensorReading::new(sensor_type(), vec![lux], vec!["lux".into()])
}

/// UART (BravePI) フレームのペイロードから変換。
/// メインボードが変換済みの Float32LE。
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 4 {
        return SensorReading::empty(sensor_type());
    }
    let lux = f32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64;
    SensorReading::new(sensor_type(), vec![lux], vec!["lux".into()])
}

fn decode_uart(sample: UartSample<'_>) -> SensorReading {
    from_uart_payload(sample.payload)
}

pub const HANDLER: crate::SensorHandler = crate::SensorHandler {
    sensor_type: SensorType::Illuminance,
    key_suffix: "illuminance",
    identity,
    decode_uart,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i2c_raw_known_value() {
        // exponent=5, fractional=1000 → 2^5 * 1000 * 0.01 = 320.0 Lux
        // smbus2 のバイトスワップ済みフォーマットで構成:
        // fractional の下位8bit → raw の上位バイト: 0xE8 (1000 & 0xFF = 0xE8)
        // fractional の上位4bit → raw の下位ニブル: 0x03 (1000 >> 8 = 0x03)
        // exponent → raw の bit[7:4]: 0x50
        let raw: u16 = 0xE853; // fractional_low=0xE8, exponent=5(0x50), fractional_high=0x03
        let reading = from_i2c_raw(raw);
        assert_eq!(reading.sensor_type, SensorType::Illuminance);
        assert!((reading.values[0] - 320.0).abs() < 0.1);
    }

    #[test]
    fn uart_payload_known_value() {
        let lux_bytes = 500.0f32.to_le_bytes();
        let reading = from_uart_payload(&lux_bytes);
        assert!((reading.values[0] - 500.0).abs() < 0.1);
    }

    #[test]
    fn uart_payload_too_short() {
        let reading = from_uart_payload(&[0x00, 0x01]);
        assert!(reading.values.is_empty());
    }
}
