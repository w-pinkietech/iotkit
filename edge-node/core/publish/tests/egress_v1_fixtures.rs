use iotkit_core_publish::wire::{AcceptedThrough, RecordBatch};

const BATCH_FIXTURE: &str = include_str!("../../../../testdata/egress/v1/record-batch.json");
const ACK_FIXTURE: &str = include_str!("../../../../testdata/egress/v1/accepted-through.json");
const RECORD_FAMILY_CASES: &str =
    include_str!("../../../../testdata/egress/v1/record-family-cases.json");

#[derive(serde::Deserialize)]
struct RecordFamilyCases {
    cases: Vec<RecordFamilyCase>,
}

#[derive(serde::Deserialize)]
struct RecordFamilyCase {
    name: String,
    valid: bool,
    record: serde_json::Value,
}

#[test]
fn rust_decodes_and_validates_v1_batch_fixture() {
    let batch: RecordBatch = serde_json::from_str(BATCH_FIXTURE).unwrap();
    batch.validate().unwrap();
    assert_eq!(batch.edge_node_id, "edge-node-01");
    assert_eq!(batch.cursor_start, 1);
    assert_eq!(batch.cursor_end, 1);
    assert_eq!(batch.records.len(), 1);
}

#[test]
fn rust_decodes_and_correlates_v1_ack_fixture() {
    let batch: RecordBatch = serde_json::from_str(BATCH_FIXTURE).unwrap();
    let ack: AcceptedThrough = serde_json::from_str(ACK_FIXTURE).unwrap();
    ack.validate_for(&batch, 0).unwrap();
}

#[test]
fn ack_for_another_edge_node_is_rejected() {
    let batch: RecordBatch = serde_json::from_str(BATCH_FIXTURE).unwrap();
    let mut ack: AcceptedThrough = serde_json::from_str(ACK_FIXTURE).unwrap();
    ack.edge_node_id = "edge-node-other".into();
    assert!(ack.validate_for(&batch, 0).is_err());
}

#[test]
fn batch_with_only_legacy_gateway_identity_is_rejected() {
    let legacy = BATCH_FIXTURE.replace(
        r#""edge_node_id": "edge-node-01""#,
        r#""gateway_identity": "gateway-01""#,
    );
    assert!(serde_json::from_str::<RecordBatch>(&legacy).is_err());
}

#[test]
fn batch_with_edge_node_id_and_legacy_gateway_identity_is_rejected() {
    let mixed = BATCH_FIXTURE.replacen(
        r#""edge_node_id": "edge-node-01""#,
        r#""edge_node_id": "edge-node-01", "gateway_identity": "gateway-01""#,
        1,
    );
    assert!(serde_json::from_str::<RecordBatch>(&mixed).is_err());
}

#[test]
fn ack_with_edge_node_id_and_legacy_gateway_identity_is_rejected() {
    let mixed = ACK_FIXTURE.replacen(
        r#""edge_node_id": "edge-node-01""#,
        r#""edge_node_id": "edge-node-01", "gateway_identity": "gateway-01""#,
        1,
    );
    assert!(serde_json::from_str::<AcceptedThrough>(&mixed).is_err());
}

#[test]
fn puback_cannot_be_represented_as_application_ack() {
    let transport_only = r#"{"packet_id":7}"#;
    assert!(serde_json::from_str::<AcceptedThrough>(transport_only).is_err());
}

#[test]
fn batch_with_unknown_record_family_is_rejected() {
    let mut value: serde_json::Value = serde_json::from_str(BATCH_FIXTURE).unwrap();
    value["records"][0]["family"] = serde_json::json!("future_family");
    let batch: RecordBatch = serde_json::from_value(value).unwrap();
    assert!(batch.validate().is_err());
}

#[test]
fn batch_with_unknown_record_field_is_rejected() {
    let mut value: serde_json::Value = serde_json::from_str(BATCH_FIXTURE).unwrap();
    value["records"][0]["unexpected"] = serde_json::json!(true);
    let batch: RecordBatch = serde_json::from_value(value).unwrap();
    assert!(batch.validate().is_err());
}

#[test]
fn malformed_known_record_families_are_rejected() {
    for record in [
        serde_json::json!({
            "family": "measurement",
            "schema_version": 1,
            "epoch": "epoch-01",
            "pub_seq": 1
        }),
        serde_json::json!({
            "family": "annotation",
            "schema_version": 1,
            "epoch": "epoch-01",
            "pub_seq": 1,
            "subtype": "epoch_start"
        }),
        serde_json::json!({
            "family": "commissioning_smoke",
            "schema_version": 1,
            "epoch": "epoch-01",
            "pub_seq": 1,
            "test_id": "smoke-invalid"
        }),
    ] {
        let mut value: serde_json::Value = serde_json::from_str(BATCH_FIXTURE).unwrap();
        value["records"][0] = record;
        let batch: RecordBatch = serde_json::from_value(value).unwrap();
        assert!(batch.validate().is_err());
    }
}

#[test]
fn rust_matches_record_family_conformance_cases() {
    let cases: RecordFamilyCases = serde_json::from_str(RECORD_FAMILY_CASES).unwrap();
    for case in cases.cases {
        let batch = RecordBatch {
            schema_version: 1,
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: "edge-node-01:epoch-01:1:1".into(),
            cursor_start: 1,
            cursor_end: 1,
            records: vec![case.record],
        };
        assert_eq!(
            batch.validate().is_ok(),
            case.valid,
            "conformance case: {}",
            case.name
        );
    }
}
