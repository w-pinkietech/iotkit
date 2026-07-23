use super::*;

#[test]
fn i2c_volts_to_mv() {
    let reading = from_i2c_volts(1.5, -0.8);
    assert!((reading.values[0] - 1500.0).abs() < 0.1);
    assert!((reading.values[1] - (-800.0)).abs() < 0.1);
    assert_eq!(reading.labels, vec!["ch1_mv", "ch2_mv"]);
}

#[test]
fn uart_payload() {
    // ch1=500, ch2=-300
    let mut data = Vec::new();
    data.extend_from_slice(&500i16.to_le_bytes());
    data.extend_from_slice(&(-300i16).to_le_bytes());
    let reading = from_uart_payload(&data);
    assert!((reading.values[0] - 500.0).abs() < 0.1);
    assert!((reading.values[1] - (-300.0)).abs() < 0.1);
    assert_eq!(reading.labels, vec!["ch1_mv", "ch2_mv"]);
}
