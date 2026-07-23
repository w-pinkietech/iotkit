use iotkit_output_adapter_api::{
    IdentityScope, Observation, ObservationKind, ObservationValue, OutputAdapter, ProfilePolicy,
    ProfileRequest,
};
use iotkit_output_adapter_generic_mqtt_json_v1::{GenericMqttJsonAdapter, GenericMqttJsonPolicy};
use iotkit_output_adapter_testkit::{ConformanceCase, assert_adapter_conformance};

#[test]
fn matches_the_shared_cumulative_value_fixture() {
    let config = serde_json::value::to_raw_value(&serde_json::json!({
        "schema_version": 1,
        "topic": "factory/line-a/production"
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
        &GenericMqttJsonAdapter,
        &[ConformanceCase {
            config: &config,
            observation: &observation,
            expected_topic: "factory/line-a/production",
            expected_qos: 1,
            expected_retain: false,
            expected_payload: r#"{"schema_version":1,"observation_id":"d36cb7b3-7010-43b3-afc6-1931ed705dea","series_id":"a921df88-6af2-46ca-a5f1-f346bf4433bb","sequence":42,"observed_at":1784190000123,"kind":"cumulative_value","value":1524}"#,
        }],
    )
    .unwrap();

    assert_eq!(
        GenericMqttJsonAdapter.descriptor().id,
        "iotkit.mqtt-json.v1"
    );
}

#[test]
fn profile_policy_generates_a_stable_common_topic_without_user_topic_input() {
    let values = serde_json::Map::new();
    let proposals = GenericMqttJsonPolicy
        .propose(&ProfileRequest {
            edge_id: "edge-0123456789abcdef0123456789abcdef",
            rule_id: "rule-01",
            signal_ref: "edge-node-01:series-01",
            external_id: "sig-0123456789abcdef0123456789abcdef",
            observation_kind: ObservationKind::Numeric,
            mode: "observation",
            values: &values,
        })
        .expect("propose generic route");
    assert_eq!(
        GenericMqttJsonPolicy.identity_policy().scope,
        IdentityScope::RuleMode
    );
    assert!(GenericMqttJsonPolicy.setup().fields.is_empty());
    assert!(proposals[0].config.get().contains(
        "iotkit/v1/sources/edge-0123456789abcdef0123456789abcdef/signals/sig-0123456789abcdef0123456789abcdef/observations"
    ));
}
