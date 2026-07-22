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
