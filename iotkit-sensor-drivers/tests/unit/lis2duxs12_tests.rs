use super::*;

#[test]
fn i2c_values_are_passed_through_in_mg() {
    // ワイヤはInt16LE×3×0.244(mG)。ドライバは単位変換せず素通しする。
    let raw_x = 50i16;
    let raw_y = -100i16;
    let raw_z = 4090i16;
    let data = [
        raw_x.to_le_bytes()[0],
        raw_x.to_le_bytes()[1],
        raw_y.to_le_bytes()[0],
        raw_y.to_le_bytes()[1],
        raw_z.to_le_bytes()[0],
        raw_z.to_le_bytes()[1],
    ];
    let reading = from_i2c_raw(&data);
    assert_eq!(
        reading.values.len(),
        3,
        "派生値はワイヤに乗せない(D6決定11)"
    );
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
    assert_eq!(
        reading.values.len(),
        3,
        "派生値はワイヤに乗せない(D6決定11)"
    );
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
