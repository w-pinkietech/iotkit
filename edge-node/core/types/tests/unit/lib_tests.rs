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

#[test]
fn identifier_accepts_contract_grammar() {
    for value in ["rpi1", "a", "0", "press-01-cycle-count", &"a".repeat(64)] {
        assert_eq!(validate_identifier(value), Ok(()), "{value:?}");
        assert_eq!(EdgeNodeId::parse(value).unwrap().as_str(), value);
        assert_eq!(PipelineId::parse(value).unwrap().to_string(), value);
    }
}

#[test]
fn identifier_rejects_contract_violations() {
    assert_eq!(validate_identifier(""), Err(IdentifierError::Empty));
    assert_eq!(
        validate_identifier(&"a".repeat(65)),
        Err(IdentifierError::TooLong { bytes: 65 })
    );
    assert_eq!(
        validate_identifier("-rpi1"),
        Err(IdentifierError::LeadingOrTrailingHyphen)
    );
    assert_eq!(
        validate_identifier("rpi1-"),
        Err(IdentifierError::LeadingOrTrailingHyphen)
    );
    assert_eq!(
        validate_identifier("-"),
        Err(IdentifierError::LeadingOrTrailingHyphen)
    );
    assert_eq!(
        validate_identifier("Rpi1"),
        Err(IdentifierError::InvalidChar {
            position: 0,
            ch: 'R'
        })
    );
    assert_eq!(
        validate_identifier("rpi_1"),
        Err(IdentifierError::InvalidChar {
            position: 3,
            ch: '_'
        })
    );
    assert_eq!(
        validate_identifier("rpi/1"),
        Err(IdentifierError::InvalidChar {
            position: 3,
            ch: '/'
        })
    );
    assert!(matches!(
        validate_identifier("端末1"),
        Err(IdentifierError::InvalidChar { position: 0, .. })
    ));
    assert!("rpi1 ".parse::<EdgeNodeId>().is_err());
}
