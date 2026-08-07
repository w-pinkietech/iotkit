use std::collections::BTreeMap;

use serde_json::Value;

use super::*;

#[test]
fn table_digest_verification_rejects_swapped_tables_even_when_the_aggregate_matches() {
    let first = [1_u8; 32];
    let second = [2_u8; 32];
    assert_eq!(
        digest_hashes(vec![first, second]),
        digest_hashes(vec![second, first]),
        "the aggregate deliberately ignores table ordering"
    );
    let source = BTreeMap::from([
        ("first", digest_hashes(vec![first])),
        ("second", digest_hashes(vec![second])),
    ]);
    let target = BTreeMap::from([
        ("first", digest_hashes(vec![second])),
        ("second", digest_hashes(vec![first])),
    ]);

    let error = verify_table_digests(&source, &target)
        .expect_err("a compensating aggregate must not hide a table mismatch");
    assert!(error.to_string().contains("for first"));
}

#[test]
fn sqlite_float_json_keeps_only_exactly_representable_i64_integers_as_json_integers() {
    assert_eq!(
        sqlite_float_json(-9_223_372_036_854_775_808.0).expect("i64 minimum"),
        Value::from(i64::MIN)
    );
    assert_eq!(
        sqlite_float_json(9_223_372_036_854_774_784.0).expect("largest f64 below 2^63"),
        Value::from(9_223_372_036_854_774_784_i64)
    );
    assert!(
        sqlite_float_json(9_223_372_036_854_775_808.0)
            .expect("finite 2^63")
            .is_f64(),
        "2^63 must remain a floating JSON number instead of saturating to i64::MAX"
    );
    let below_i64_minimum = f64::from_bits((i64::MIN as f64).to_bits() + 1);
    assert!(below_i64_minimum < i64::MIN as f64);
    assert!(
        sqlite_float_json(below_i64_minimum)
            .expect("finite value below i64 minimum")
            .is_f64(),
        "values below i64::MIN must remain floating JSON numbers"
    );
}
