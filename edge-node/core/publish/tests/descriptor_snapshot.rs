use iotkit_core_publish::{
    PublishError,
    descriptor::{DescriptorDevice, DescriptorSnapshot, MAX_DESCRIPTOR_BYTES},
};
use serde::Deserialize;
use std::collections::HashSet;

const EXPECTED_DESCRIPTOR_CASES: [(&str, &str); 16] = [
    ("canonical_uuid_revision_one", "valid"),
    ("canonical_uuid_revision_i64_max", "valid"),
    ("optional_descriptor_fields_omitted", "valid"),
    ("noncanonical_uppercase_uuid", "noncanonical_uuid"),
    ("noncanonical_compact_uuid", "noncanonical_uuid"),
    ("descriptor_revision_zero", "descriptor_revision"),
    ("descriptor_revision_above_i64_max", "descriptor_revision"),
    ("measurement_key_exact_64_bytes", "valid"),
    ("measurement_key_over_64_bytes", "measurement_key"),
    ("measurement_key_invalid_segment", "measurement_key"),
    ("unknown_descriptor_field", "unknown_field"),
    ("unknown_descriptor_schema_version", "schema_version"),
    ("edge_node_id_over_255_bytes", "identity_boundary"),
    ("edge_node_id_control_character", "identity_control"),
    ("ledger_epoch_control_character", "identity_control"),
    ("variant_control_character", "identity_control"),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptorConformanceCorpus {
    schema_version: u32,
    cases: Vec<DescriptorConformanceCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptorConformanceCase {
    name: String,
    valid: bool,
    reason_category: String,
    descriptor: serde_json::Value,
}

fn shared_descriptor_conformance_cases() -> DescriptorConformanceCorpus {
    let fixture = std::fs::read("../../../testdata/egress/v2/descriptor-conformance-cases.json")
        .expect("read shared descriptor conformance corpus");
    let corpus: DescriptorConformanceCorpus =
        serde_json::from_slice(&fixture).expect("decode shared descriptor conformance corpus");
    assert_eq!(
        corpus.schema_version, 1,
        "unsupported descriptor corpus version"
    );
    assert!(
        !corpus.cases.is_empty(),
        "descriptor corpus must not be empty"
    );
    let mut names = HashSet::with_capacity(corpus.cases.len());
    for case in &corpus.cases {
        assert!(
            names.insert(case.name.as_str()),
            "duplicate descriptor case name: {}",
            case.name
        );
        assert_eq!(
            case.valid,
            case.reason_category == "valid",
            "descriptor corpus validity/category mismatch: {}",
            case.name
        );
    }
    let cases: Vec<(&str, &str)> = corpus
        .cases
        .iter()
        .map(|case| (case.name.as_str(), case.reason_category.as_str()))
        .collect();
    assert_eq!(
        cases.as_slice(),
        EXPECTED_DESCRIPTOR_CASES.as_slice(),
        "descriptor corpus names/categories changed"
    );
    corpus
}

#[test]
fn producer_matches_the_shared_descriptor_conformance_cases() {
    for case in shared_descriptor_conformance_cases().cases {
        let payload = serde_json::to_vec(&case.descriptor).expect("encode descriptor case");
        let result = DescriptorSnapshot::decode_bounded(&payload);
        assert_eq!(
            result.is_ok(),
            case.valid,
            "conformance case outcome: {} ({})",
            case.name,
            case.reason_category,
        );
        match result {
            Ok(_) => assert!(case.valid, "accepted invalid case: {}", case.name),
            Err(PublishError::Invalid(message)) if case.reason_category == "unknown_field" => {
                assert!(
                    message.starts_with("descriptor decoding failed:"),
                    "unknown-field case must fail during decode: {}",
                    case.name
                );
            }
            Err(PublishError::Invalid(message)) => assert_eq!(
                message, case.reason_category,
                "conformance reason category: {}",
                case.name,
            ),
            Err(other) => panic!("unexpected descriptor error for {}: {other}", case.name),
        }
    }
}

#[test]
fn strict_decode_accepts_fixture_and_rejects_unknown_or_inconsistent_content() {
    let fixture = std::fs::read("../../../testdata/egress/v2/descriptor-snapshot.json").unwrap();
    let snapshot = DescriptorSnapshot::decode_bounded(&fixture).unwrap();
    assert_eq!(snapshot.descriptor_revision, 5);
    assert_eq!(snapshot.devices[0].model_id.as_deref(), Some("mcp9600"));

    let mut unknown: serde_json::Value = serde_json::from_slice(&fixture).unwrap();
    unknown["provider_payload"] = serde_json::json!({"secret": true});
    assert!(DescriptorSnapshot::decode_bounded(&serde_json::to_vec(&unknown).unwrap()).is_err());

    let mut inconsistent: serde_json::Value = serde_json::from_slice(&fixture).unwrap();
    inconsistent["signals"][0]["series_key"] = serde_json::json!("wrong");
    assert!(
        DescriptorSnapshot::decode_bounded(&serde_json::to_vec(&inconsistent).unwrap()).is_err()
    );
}

#[test]
fn schema_two_accepts_valid_model_id_and_schema_one_is_unsupported() {
    let fixture = std::fs::read("../../../testdata/egress/v2/descriptor-snapshot.json").unwrap();
    let snapshot = DescriptorSnapshot::decode_bounded(&fixture).unwrap();
    assert_eq!(snapshot.schema_version, 2);
    assert_eq!(snapshot.devices[0].model_id.as_deref(), Some("mcp9600"));

    let mut unsupported_v1: serde_json::Value = serde_json::from_slice(&fixture).unwrap();
    unsupported_v1["schema_version"] = serde_json::json!(1);
    unsupported_v1["devices"][0]
        .as_object_mut()
        .unwrap()
        .remove("model_id");
    assert!(
        DescriptorSnapshot::decode_bounded(&serde_json::to_vec(&unsupported_v1).unwrap()).is_err()
    );
}

#[test]
fn schema_two_rejects_non_canonical_model_ids() {
    let fixture = std::fs::read("../../../testdata/egress/v2/descriptor-snapshot.json").unwrap();
    for invalid in [
        "",
        "MCP9600",
        "-mcp9600",
        "vendor//model",
        "model id",
        "model-",
    ] {
        let mut value: serde_json::Value = serde_json::from_slice(&fixture).unwrap();
        value["devices"][0]["model_id"] = serde_json::json!(invalid);
        assert!(
            DescriptorSnapshot::decode_bounded(&serde_json::to_vec(&value).unwrap()).is_err(),
            "accepted invalid model_id {invalid:?}"
        );
    }
}

#[test]
fn bounded_encoding_rejects_oversize_without_truncation() {
    let snapshot = DescriptorSnapshot {
        schema_version: 2,
        edge_node_id: "edge-node-01".into(),
        ledger_epoch: "epoch-01".into(),
        descriptor_revision: 1,
        complete: true,
        devices: (0..10_000)
            .map(|index| DescriptorDevice {
                system_id: format!("00000000-0000-0000-0000-{index:012x}"),
                identifier: Some("x".repeat(64)),
                state: "active".into(),
                model_id: None,
            })
            .collect(),
        signals: Vec::new(),
    };

    let error = snapshot
        .encode_bounded()
        .expect_err("valid oversized descriptor must reach the byte limit");
    let PublishError::Invalid(message) = error else {
        panic!("unexpected descriptor encoding error");
    };
    assert_eq!(
        message,
        format!("descriptor snapshot exceeds {MAX_DESCRIPTOR_BYTES} encoded bytes")
    );
    assert_eq!(
        snapshot.devices.len(),
        10_000,
        "oversize data was truncated"
    );
}

#[test]
fn bounded_decode_accepts_exact_limit_and_rejects_one_byte_over() {
    let mut payload =
        std::fs::read("../../../testdata/egress/v2/descriptor-snapshot.json").unwrap();
    payload.resize(MAX_DESCRIPTOR_BYTES, b' ');
    assert_eq!(payload.len(), MAX_DESCRIPTOR_BYTES);
    DescriptorSnapshot::decode_bounded(&payload).expect("valid descriptor at exact byte limit");

    payload.push(b' ');
    let error = DescriptorSnapshot::decode_bounded(&payload)
        .expect_err("descriptor over byte limit must be rejected");
    let PublishError::Invalid(message) = error else {
        panic!("unexpected descriptor decoding error");
    };
    assert_eq!(
        message,
        format!("descriptor snapshot exceeds {MAX_DESCRIPTOR_BYTES} encoded bytes")
    );
}
