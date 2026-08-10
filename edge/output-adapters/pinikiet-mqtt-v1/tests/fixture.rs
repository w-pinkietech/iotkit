use iotkit_output_adapter_api::{
    IdentityScope, Observation, ObservationKind, ObservationValue, OutputAdapter, ProfilePolicy,
    ProfileRequest,
};
use iotkit_output_adapter_pinikiet_mqtt_v1::{
    PinikietMqttAdapter, PinikietProfilePolicy, source_status,
};
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
    source_id: String,
    sensor_id: String,
    kind: FixturePublicationKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    reason: String,
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FixturePublicationKind {
    Production,
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
    kind: FixturePublicationKind,
    value: u64,
}

#[test]
fn matches_the_shared_production_fixture() {
    let fixture: Fixture = serde_json::from_str(include_str!(
        "../../../../testdata/output/v1/pinikiet-production.json"
    ))
    .expect("decode shared Pinikiet output fixture");
    assert_eq!(
        fixture.observation.kind,
        FixtureObservationKind::CumulativeValue,
        "shared fixture observation kind",
    );
    assert_eq!(
        fixture.publication.payload.kind,
        FixturePublicationKind::Production,
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
        .expect("encode shared Pinikiet output config");
    let expected_payload = serde_json::to_string(&fixture.publication.payload)
        .expect("encode shared Pinikiet output payload");

    assert_adapter_conformance(
        &PinikietMqttAdapter,
        &[ConformanceCase {
            config: &config,
            observation: &observation,
            expected_topic: &fixture.publication.topic,
            expected_qos: fixture.publication.qos,
            expected_retain: fixture.publication.retain,
            expected_payload: &expected_payload,
        }],
    )
    .expect("adapter matches shared Pinikiet output fixture");

    assert_eq!(PinikietMqttAdapter.descriptor().id, fixture.adapter_id);
}

#[test]
fn shared_fixture_is_closed_about_fields_and_observation_kind() {
    let mut unknown_field: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../testdata/output/v1/pinikiet-production.json"
    ))
    .expect("decode fixture JSON");
    unknown_field["observation"]["unexpected"] = true.into();
    assert!(serde_json::from_value::<Fixture>(unknown_field).is_err());

    let mut wrong_kind: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../testdata/output/v1/pinikiet-production.json"
    ))
    .expect("decode fixture JSON");
    wrong_kind["observation"]["kind"] = "numeric".into();
    assert!(serde_json::from_value::<Fixture>(wrong_kind).is_err());
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

#[test]
fn source_status_is_online_qos_one_and_retained() {
    let status = source_status("edge-0123456789abcdef0123456789abcdef", 1_784_190_000_123)
        .expect("valid Pinikiet status");
    assert_eq!(
        status.topic(),
        "pinikiet/v1/sources/edge-0123456789abcdef0123456789abcdef/status"
    );
    assert_eq!(status.qos(), 1);
    assert!(status.retain());
    assert_eq!(
        status.payload().get(),
        r#"{"schema_version":1,"reported_at":1784190000123,"state":"online"}"#
    );
}
