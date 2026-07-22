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
