use iotkit_output_adapter_api::{
    IdentityScope, Observation, ObservationKind, ObservationValue, ProfilePolicy, ProfileRequest,
};
use iotkit_output_adapter_pinikiet_mqtt_v1::{PinikietMqttAdapter, PinikietProfilePolicy};
use iotkit_output_adapter_testkit::{ConformanceCase, assert_adapter_conformance};

#[test]
fn matches_the_shared_production_fixture() {
    let config = serde_json::value::to_raw_value(&serde_json::json!({
        "schema_version": 1,
        "source_id": "iotkit-01",
        "sensor_id": "press-sensor",
        "kind": "production"
    }))
    .unwrap();
    let observation = Observation::new(
        "d36cb7b3-7010-43b3-afc6-1931ed705dea",
        "a921df88-6af2-46ca-a5f1-f346bf4433bb",
        42,
        1_784_190_000_123,
        ObservationValue::CumulativeValue(1524),
    )
    .unwrap();

    assert_adapter_conformance(
        &PinikietMqttAdapter,
        &[ConformanceCase {
            config: &config,
            observation: &observation,
            expected_topic:
                "pinikiet/v1/sources/iotkit-01/sensors/press-sensor/observations",
            expected_qos: 1,
            expected_retain: false,
            expected_payload: r#"{"schema_version":1,"observation_id":"d36cb7b3-7010-43b3-afc6-1931ed705dea","series_id":"a921df88-6af2-46ca-a5f1-f346bf4433bb","sequence":42,"observed_at":1784190000123,"kind":"production","value":1524}"#,
        }],
    )
    .unwrap();
}

#[test]
fn profile_policy_uses_edge_owned_source_and_signal_scoped_sensor_identity() {
    let values = serde_json::Map::new();
    let proposals = PinikietProfilePolicy
        .propose(&ProfileRequest {
            edge_id: "edge-0123456789abcdef0123456789abcdef",
            rule_id: "rule-01",
            signal_ref: "edge-node-01:series-01",
            external_id: "sen-0123456789abcdef0123456789abcdef",
            observation_kind: ObservationKind::CumulativeValue,
            mode: "production",
            values: &values,
        })
        .expect("propose Pinikiet route");
    assert_eq!(
        PinikietProfilePolicy.identity_policy().scope,
        IdentityScope::Signal
    );
    assert!(
        PinikietProfilePolicy
            .setup()
            .fields
            .iter()
            .all(|field| field.key != "source_id")
    );
    let config: serde_json::Value =
        serde_json::from_str(proposals[0].config.get()).expect("decode proposed config");
    assert_eq!(config["source_id"], "edge-0123456789abcdef0123456789abcdef");
    assert_eq!(config["sensor_id"], "sen-0123456789abcdef0123456789abcdef");
}

#[test]
fn profile_policy_rejects_incompatible_kind_and_mode() {
    let values = serde_json::Map::new();
    assert!(
        PinikietProfilePolicy
            .propose(&ProfileRequest {
                edge_id: "edge-0123456789abcdef0123456789abcdef",
                rule_id: "rule-01",
                signal_ref: "edge-node-01:series-01",
                external_id: "sen-0123456789abcdef0123456789abcdef",
                observation_kind: ObservationKind::Numeric,
                mode: "production",
                values: &values,
            })
            .is_err()
    );
}
