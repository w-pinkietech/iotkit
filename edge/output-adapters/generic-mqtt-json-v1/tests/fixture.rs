use iotkit_output_adapter_api::{
    IdentityScope, Observation, ObservationKind, ObservationValue, OutputAdapter, ProfilePolicy,
    ProfileRequest,
};
use iotkit_output_adapter_generic_mqtt_json_v1::{GenericMqttJsonAdapter, GenericMqttJsonPolicy};
use iotkit_output_adapter_testkit::{ConformanceCase, assert_adapter_conformance};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    adapter_id: String,
    config: FixtureConfig,
    observation: FixtureObservation,
    publication: FixturePublication,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FixtureConfig {
    schema_version: u32,
    topic: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureObservation {
    observation_id: String,
    series_id: String,
    sequence: u64,
    observed_at: i64,
    kind: FixtureObservationKind,
    value: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FixtureObservationKind {
    CumulativeValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixturePublication {
    topic: String,
    qos: u8,
    retain: bool,
    payload: FixturePayload,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FixturePayload {
    schema_version: u32,
    observation_id: String,
    series_id: String,
    sequence: u64,
    observed_at: i64,
    kind: FixtureObservationKind,
    value: u64,
}

#[test]
fn matches_the_shared_cumulative_value_fixture() {
    let fixture: Fixture = serde_json::from_str(include_str!(
        "../../../../testdata/output/v1/iotkit-cumulative-value.json"
    ))
    .expect("decode shared generic output fixture");
    assert_eq!(
        fixture.observation.kind,
        FixtureObservationKind::CumulativeValue,
        "shared fixture observation kind",
    );
    assert_eq!(
        fixture.publication.payload.kind,
        FixtureObservationKind::CumulativeValue,
        "shared fixture payload kind",
    );
    let observation = Observation::new(
        &fixture.observation.observation_id,
        &fixture.observation.series_id,
        fixture.observation.sequence,
        fixture.observation.observed_at,
        ObservationValue::CumulativeValue(fixture.observation.value),
    )
    .expect("fixture observation is valid");
    let config = serde_json::value::to_raw_value(&fixture.config)
        .expect("encode shared generic output config");
    let expected_payload = serde_json::to_string(&fixture.publication.payload)
        .expect("encode shared generic output payload");

    assert_adapter_conformance(
        &GenericMqttJsonAdapter,
        &[ConformanceCase {
            config: &config,
            observation: &observation,
            expected_topic: &fixture.publication.topic,
            expected_qos: fixture.publication.qos,
            expected_retain: fixture.publication.retain,
            expected_payload: &expected_payload,
        }],
    )
    .expect("adapter matches shared generic output fixture");

    assert_eq!(GenericMqttJsonAdapter.descriptor().id, fixture.adapter_id);
}

#[test]
fn shared_fixture_is_closed_about_fields_and_observation_kind() {
    let mut unknown_field: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../testdata/output/v1/iotkit-cumulative-value.json"
    ))
    .expect("decode fixture JSON");
    unknown_field["observation"]["unexpected"] = true.into();
    assert!(serde_json::from_value::<Fixture>(unknown_field).is_err());

    let mut wrong_kind: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../testdata/output/v1/iotkit-cumulative-value.json"
    ))
    .expect("decode fixture JSON");
    wrong_kind["observation"]["kind"] = "numeric".into();
    assert!(serde_json::from_value::<Fixture>(wrong_kind).is_err());
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
