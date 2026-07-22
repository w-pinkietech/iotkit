use super::*;

#[test]
fn accepts_standard_and_custom_keys() {
    for k in [
        "temperature_c",
        "voltage_mv",
        "custom.tank_level",
        "a",
        "x9_z.b_1",
    ] {
        assert!(validate_measurement_key(k).is_ok(), "{k} should be valid");
    }
}

#[test]
fn rejects_colon_uppercase_and_bad_segments() {
    for k in [
        "custom:temp",
        "Temp",
        "9abc",
        "a..b",
        ".a",
        "a.",
        "",
        "温度",
    ] {
        assert!(
            validate_measurement_key(k).is_err(),
            "{k} should be invalid"
        );
    }
}

#[test]
fn rejects_over_64_chars() {
    let k = "a".repeat(65);
    assert!(matches!(
        validate_measurement_key(&k),
        Err(MeasurementKeyError::TooLong { .. })
    ));
}

#[test]
fn envelope_id_recipe_is_stable() {
    assert_eq!(external_envelope_id("dev1", 3, 42), "dev1-3-42");
}
