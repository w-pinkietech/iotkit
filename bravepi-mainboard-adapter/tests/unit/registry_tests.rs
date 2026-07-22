use super::*;
use iotkit_core_types::SensorType;

#[test]
fn all_known_raw_codes_resolve() {
    let expected = [
        (257, "contact_input"),
        (258, "contact_output"),
        (259, "adc"),
        (260, "ranging"),
        (261, "temperature"),
        (262, "acceleration"),
        (263, "differential_pressure"),
        (264, "illuminance"),
    ];
    for (raw, suffix) in expected {
        let handler = lookup_handler(raw).unwrap_or_else(|| panic!("raw {} should resolve", raw));
        assert_eq!(handler.key_suffix, suffix, "raw {} suffix mismatch", raw);
    }
}

#[test]
fn unknown_raw_code_returns_none() {
    assert!(lookup_handler(0).is_none());
    assert!(lookup_handler(9999).is_none());
}

#[test]
fn handler_sensor_types_are_correct() {
    assert_eq!(
        lookup_handler(261).unwrap().sensor_type,
        SensorType::Temperature
    );
    assert_eq!(
        lookup_handler(257).unwrap().sensor_type,
        SensorType::ContactInput
    );
    assert_eq!(
        lookup_handler(258).unwrap().sensor_type,
        SensorType::ContactOutput
    );
}

#[test]
fn no_duplicate_raw_codes() {
    let mut seen = std::collections::HashSet::new();
    for entry in REGISTRY.iter() {
        assert!(
            seen.insert(entry.raw_sensor_type),
            "duplicate raw code: {}",
            entry.raw_sensor_type,
        );
    }
}
