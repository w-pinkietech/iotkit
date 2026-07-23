use super::*;

#[test]
fn sensor_type_db_str_round_trip() {
    let variants: Vec<SensorType> = vec![
        SensorType::ContactInput,
        SensorType::ContactOutput,
        SensorType::Adc,
        SensorType::Ranging,
        SensorType::Temperature,
        SensorType::Acceleration,
        SensorType::DifferentialPressure,
        SensorType::Illuminance,
    ];
    for v in variants {
        let db_str = v.as_db_str();
        let round_tripped = SensorType::from_db_str(db_str);
        assert_eq!(
            v, round_tripped,
            "round-trip failed for {v:?} -> {db_str:?}"
        );
    }
}

#[test]
fn sensor_type_unknown_round_trip() {
    let original = SensorType::Unknown("custom_xyz".to_string());
    let db_str = original.as_db_str();
    assert_eq!(db_str, "custom_xyz");
    let round_tripped = SensorType::from_db_str(db_str);
    assert_eq!(round_tripped, SensorType::Unknown("custom_xyz".to_string()));
}
