use iotkit_output_adapter_api::{AdapterError, MqttPublication, Observation, ObservationValue};

#[test]
fn observation_rejects_non_canonical_identity_and_sequence_zero() {
    let error = Observation::new(
        "D36CB7B3-7010-43B3-AFC6-1931ED705DEA",
        "a921df88-6af2-46ca-a5f1-f346bf4433bb",
        0,
        1_784_190_000_123,
        ObservationValue::CumulativeValue(1),
    )
    .unwrap_err();
    assert_eq!(error, AdapterError::InvalidObservation);
}

#[test]
fn publication_rejects_wildcard_topic_and_non_qos_one() {
    let payload = serde_json::value::to_raw_value(&serde_json::json!({"value": 1})).unwrap();
    assert_eq!(
        MqttPublication::new("factory/+/value", 1, false, payload).unwrap_err(),
        AdapterError::InvalidPublication
    );
    let payload = serde_json::value::to_raw_value(&serde_json::json!({"value": 1})).unwrap();
    assert_eq!(
        MqttPublication::new("factory/value", 0, false, payload).unwrap_err(),
        AdapterError::InvalidPublication
    );
}
