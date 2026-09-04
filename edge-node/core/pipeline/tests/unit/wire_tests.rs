use super::*;

const FIXTURE_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../testdata/observation/v1"
);

struct Fixture {
    name: String,
    kind: PipelineKind,
    topic: String,
    payload: String,
}

fn observation_fixtures() -> Vec<Fixture> {
    let mut fixtures: Vec<Fixture> = std::fs::read_dir(FIXTURE_DIR)
        .expect("fixture directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_name()?.to_str()?.to_owned();
            if !name.starts_with("observation-") || !name.ends_with(".json") {
                return None;
            }
            let value: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
            let kind = match value["kind"].as_str()? {
                "measurement" => PipelineKind::Measurement,
                "state" => PipelineKind::State,
                "accumulated-count" => PipelineKind::AccumulatedCount,
                other => panic!("{name}: unknown kind {other}"),
            };
            Some(Fixture {
                name,
                kind,
                topic: value["topic"].as_str()?.to_owned(),
                payload: value["payload"].as_str()?.to_owned(),
            })
        })
        .collect();
    fixtures.sort_by(|a, b| a.name.cmp(&b.name));
    assert!(
        fixtures.len() >= 5,
        "expected the contract fixtures, found {}",
        fixtures.len()
    );
    fixtures
}

fn topic_parts(topic: &str) -> (EdgeNodeId, PipelineId) {
    let parts: Vec<&str> = topic.split('/').collect();
    assert_eq!(parts[..3], ["iotkit", "v1", "edge-node"]);
    assert_eq!(parts[4], "observation");
    (parts[3].parse().unwrap(), parts[5].parse().unwrap())
}

#[test]
fn producer_output_matches_every_contract_fixture_byte_for_byte() {
    for fixture in observation_fixtures() {
        let (edge_node_id, pipeline_id) = topic_parts(&fixture.topic);
        if fixture.payload.is_empty() {
            assert_eq!(
                observation_topic(&edge_node_id, &pipeline_id, fixture.kind),
                fixture.topic,
                "{}",
                fixture.name
            );
            continue;
        }
        let expected: serde_json::Value = serde_json::from_str(&fixture.payload).unwrap();
        let value = match fixture.kind {
            PipelineKind::Measurement => {
                ObservationValue::Measurement(expected["value"].as_f64().unwrap())
            }
            PipelineKind::State => ObservationValue::State(expected["value"].as_bool().unwrap()),
            PipelineKind::AccumulatedCount => {
                ObservationValue::AccumulatedCount(expected["value"].as_i64().unwrap())
            }
        };
        assert!(
            expected.as_object().unwrap().contains_key("unix_epoch_ms"),
            "{}: the wall-clock key is always present",
            fixture.name
        );
        let observation = Observation {
            pipeline_id,
            series_id: expected["series_id"].as_str().unwrap().to_owned(),
            sequence: expected["sequence"].as_u64().unwrap(),
            at: InputTime {
                uptime_ms: expected["uptime_ms"].as_i64().unwrap(),
                unix_epoch_ms: expected["unix_epoch_ms"].as_i64(),
            },
            value,
        };
        assert_eq!(
            observation.topic(&edge_node_id),
            fixture.topic,
            "{}",
            fixture.name
        );
        assert_eq!(
            String::from_utf8(observation.payload()).unwrap(),
            fixture.payload,
            "{}",
            fixture.name
        );
    }
}

#[test]
fn measurement_values_keep_fraction_only_when_present() {
    assert_eq!(
        ObservationValue::Measurement(24.0).to_json().to_string(),
        "24"
    );
    assert_eq!(
        ObservationValue::Measurement(23.5).to_json().to_string(),
        "23.5"
    );
    assert_eq!(
        ObservationValue::Measurement(-0.25).to_json().to_string(),
        "-0.25"
    );
    assert_eq!(
        ObservationValue::Measurement(1e15).to_json().to_string(),
        "1000000000000000"
    );
    // Beyond 2^53 an integral float stays a float so no precision is implied.
    assert_eq!(
        ObservationValue::Measurement(1e16).to_json().to_string(),
        "1e+16"
    );
    assert_eq!(
        ObservationValue::State(false).to_json().to_string(),
        "false"
    );
    assert_eq!(
        ObservationValue::AccumulatedCount(0).to_json().to_string(),
        "0"
    );
}

#[test]
fn uptime_is_monotonic_and_null_wall_clock_serializes_as_null() {
    let first = uptime_ms();
    let second = uptime_ms();
    assert!(first > 0 && second >= first);

    let observation = Observation {
        pipeline_id: "p".parse().unwrap(),
        series_id: "s".into(),
        sequence: 1,
        at: InputTime {
            uptime_ms: 42,
            unix_epoch_ms: None,
        },
        value: ObservationValue::State(true),
    };
    assert_eq!(
        String::from_utf8(observation.payload()).unwrap(),
        "{\"series_id\":\"s\",\"sequence\":1,\"uptime_ms\":42,\"unix_epoch_ms\":null,\"value\":true}"
    );
}
