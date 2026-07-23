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
