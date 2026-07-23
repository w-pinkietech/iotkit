use iotkit_output_adapter_api::{Observation, ObservationValue};
use iotkit_output_adapter_example::ExampleNumericAdapter;
use iotkit_output_adapter_testkit::{ConformanceCase, assert_adapter_conformance};

#[test]
fn example_is_a_complete_conforming_adapter_but_not_a_builtin() {
    let config = serde_json::value::to_raw_value(&serde_json::json!({
        "schema_version": 1,
        "topic": "example/numeric"
    }))
    .unwrap();
    let observation = Observation::new(
        "d36cb7b3-7010-43b3-afc6-1931ed705dea",
        "a921df88-6af2-46ca-a5f1-f346bf4433bb",
        1,
        1_784_190_000_123,
        ObservationValue::Numeric(12.5),
    )
    .unwrap();

    assert_adapter_conformance(
        &ExampleNumericAdapter,
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
