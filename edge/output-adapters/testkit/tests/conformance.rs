use iotkit_output_adapter_api::{
    AdapterError, Descriptor, Mode, MqttPublication, Observation, ObservationKind,
    ObservationValue, OutputAdapter,
};
use iotkit_output_adapter_testkit::{ConformanceCase, assert_adapter_conformance};

static MODES: &[Mode] = &[Mode {
    key: "numeric",
    display_name: "Numeric",
    accepts: &[ObservationKind::Numeric],
}];
static DESCRIPTOR: Descriptor = Descriptor {
    id: "example.numeric.v1",
    display_name: "Example",
    config_schema_version: 1,
    modes: MODES,
};

struct Adapter;

impl OutputAdapter for Adapter {
    fn descriptor(&self) -> &'static Descriptor {
        &DESCRIPTOR
    }

    fn validate_config(
        &self,
        _: &serde_json::value::RawValue,
        kind: ObservationKind,
    ) -> Result<(), AdapterError> {
        if kind == ObservationKind::Numeric {
            Ok(())
        } else {
            Err(AdapterError::UnsupportedObservation)
        }
    }

    fn transform(
        &self,
        _: &serde_json::value::RawValue,
        _: &Observation,
    ) -> Result<MqttPublication, AdapterError> {
        MqttPublication::new(
            "example/numeric",
            1,
            false,
            serde_json::value::to_raw_value(&serde_json::json!({"value": 12.5})).unwrap(),
        )
    }
}

#[test]
fn accepts_a_deterministic_adapter() {
    let config = serde_json::value::to_raw_value(&serde_json::json!({
        "schema_version": 1
    }))
    .unwrap();
    let observation = Observation::new(
        "d36cb7b3-7010-43b3-afc6-1931ed705dea",
        "a921df88-6af2-46ca-a5f1-f346bf4433bb",
        42,
        1_784_190_000_123,
        ObservationValue::Numeric(12.5),
    )
    .unwrap();

    assert_adapter_conformance(
        &Adapter,
        &[ConformanceCase {
            config: &config,
            observation: &observation,
            expected_topic: "example/numeric",
            expected_qos: 1,
            expected_retain: false,
            expected_payload: r#"{"value":12.5}"#,
        }],
    )
    .unwrap();
}
