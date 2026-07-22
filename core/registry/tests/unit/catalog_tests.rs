use super::*;

#[test]
fn standard_catalog_parses_with_10_keys_and_version() {
    let c = standard_catalog();
    assert_eq!(c.catalog_version, "1.0.0");
    assert_eq!(c.measurements.len(), 10);
    for k in [
        "contact_state",
        "contact_output_state",
        "voltage_mv",
        "distance_mm",
        "temperature_c",
        "acceleration_mg",
        "differential_pressure_pa",
        "illuminance_lux",
        "current_ma",
        "vibration_spectrum",
    ] {
        assert!(c.find(k).is_some(), "{k} must be in the standard catalog");
    }
}

#[test]
fn acceleration_is_fixed_xyz_and_temperature_is_single() {
    let c = standard_catalog();
    let acc = c.find("acceleration_mg").unwrap();
    assert_eq!(acc.channel_mode, ChannelMode::Fixed);
    assert_eq!(acc.channel_roles, vec!["x", "y", "z"]);
    assert_eq!(
        acc.physical_range,
        Some(Range {
            min: -16000.0,
            max: 16000.0
        })
    );
    let t = c.find("temperature_c").unwrap();
    assert_eq!(t.channel_mode, ChannelMode::Single);
    assert_eq!(t.unit_ucum.as_deref(), Some("Cel"));
    assert_eq!(t.unit_display.as_deref(), Some("℃"));
}

#[test]
fn vibration_spectrum_is_reserved_record() {
    let v = standard_catalog().find("vibration_spectrum").unwrap();
    assert_eq!(v.value_type, ValueType::Record);
    assert!(v.physical_range.is_none());
}

#[test]
fn all_catalog_keys_pass_contract_grammar() {
    for m in &standard_catalog().measurements {
        assert!(
            iotkit_ingest_contract::validate_measurement_key(&m.key).is_ok(),
            "{} must satisfy D6決定2 grammar",
            m.key
        );
    }
}

#[test]
fn parse_rejects_duplicate_keys() {
    let dup = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "temperature_c"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
[[measurement]]
key = "temperature_c"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
"#;
    assert!(matches!(parse_catalog(dup), Err(CatalogError::Invalid(_))));
}

#[test]
fn parse_rejects_fixed_without_roles_and_roles_on_non_fixed() {
    let fixed_no_roles = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "a"
value_type = "float"
semantic_class = "sensor"
channel_mode = "fixed"
"#;
    assert!(matches!(
        parse_catalog(fixed_no_roles),
        Err(CatalogError::Invalid(_))
    ));
    let single_with_roles = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "a"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
channel_roles = ["x"]
"#;
    assert!(matches!(
        parse_catalog(single_with_roles),
        Err(CatalogError::Invalid(_))
    ));
}

#[test]
fn parse_rejects_bad_key_grammar_and_inverted_range() {
    let bad_key = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "Bad:Key"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
"#;
    assert!(matches!(
        parse_catalog(bad_key),
        Err(CatalogError::Invalid(_))
    ));
    let inverted = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "a"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
physical_range = { min = 10.0, max = 1.0 }
"#;
    assert!(matches!(
        parse_catalog(inverted),
        Err(CatalogError::Invalid(_))
    ));
}

#[test]
fn revision_is_stable_and_content_sensitive() {
    let c = standard_catalog();
    let t = c.find("temperature_c").unwrap();
    let r1 = t.revision();
    let r2 = t.revision();
    assert_eq!(r1, r2, "same content → same revision");
    assert_eq!(r1.len(), 64, "sha256 hex");
    let mut altered = t.clone();
    altered.physical_range = Some(Range {
        min: -200.0,
        max: 9999.0,
    });
    assert_ne!(r1, altered.revision(), "content change → revision change");
}
