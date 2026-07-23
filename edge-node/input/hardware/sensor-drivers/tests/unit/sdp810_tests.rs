use super::*;

#[test]
fn crc8_known() {
    // SDP810 CRC example
    assert_eq!(crc8(&[0x00, 0x00]), 0x81);
}

#[test]
fn i2c_raw_with_valid_crc() {
    // dp = 100 (0x0064), scale = 60 (0x003C)
    // pressure = 100/60 = 1.6667 Pa
    let dp_bytes = 100i16.to_be_bytes();
    let scale_bytes = 60i16.to_be_bytes();
    let data: [u8; 9] = [
        dp_bytes[0],
        dp_bytes[1],
        crc8(&dp_bytes),
        0x00,
        0x00,
        0x00, // bytes 3-5 (temperature, unused here)
        scale_bytes[0],
        scale_bytes[1],
        crc8(&scale_bytes),
    ];
    let reading = from_i2c_raw(&data);
    assert!((reading.values[0] - 1.6667).abs() < 0.01);
}

#[test]
fn i2c_raw_bad_crc_returns_empty() {
    let data: [u8; 9] = [0x00, 0x64, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x00];
    let reading = from_i2c_raw(&data);
    assert!(reading.values.is_empty());
}

#[test]
fn uart_payload() {
    let reading = from_uart_payload(&123.456f32.to_le_bytes());
    assert!((reading.values[0] - 123.456).abs() < 0.01);
}
