use iotkit_core_publish::wire::{AcceptedThrough, RecordBatch, StatusHeartbeat, publication_id};

const BATCH_FIXTURE: &str = include_str!("../../../../testdata/egress/v1/record-batch.json");
const ACK_FIXTURE: &str = include_str!("../../../../testdata/egress/v1/accepted-through.json");
const RECORD_FAMILY_CASES: &str =
    include_str!("../../../../testdata/egress/v1/record-family-cases.json");
const STATUS_FIXTURE: &str = include_str!("../../../../testdata/egress/v1/status-heartbeat.json");

type StatusMutation = Box<dyn Fn(&mut serde_json::Value)>;

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
fn rust_decodes_and_validates_v1_status_heartbeat_fixture() {
    let heartbeat: StatusHeartbeat = serde_json::from_str(STATUS_FIXTURE).unwrap();
    heartbeat.validate().unwrap();
    heartbeat.validate_topic_edge_node("edge-node-01").unwrap();
    assert_eq!(heartbeat.status_seq, 1);
}

#[test]
fn rust_status_heartbeat_has_the_same_strict_boundary_as_edge_ingest() {
    let original: serde_json::Value = serde_json::from_str(STATUS_FIXTURE).unwrap();
    let cases: Vec<(&str, StatusMutation)> = vec![
        (
            "unknown secret field",
            Box::new(|value| value["password"] = "must-not-cross-mqtt".into()),
        ),
        (
            "unsafe topic identity",
            Box::new(|value| value["edge_node_id"] = "edge-node/other".into()),
        ),
        (
            "unsafe ledger identity",
            Box::new(|value| value["ledger_epoch"] = "epoch\nunsafe".into()),
        ),
        (
            "unsafe boot identity",
            Box::new(|value| value["boot_id"] = "boot:unsafe".into()),
        ),
        (
            "duplicate adapter",
            Box::new(|value| {
                let adapter = value["adapters"][0].clone();
                value["adapters"].as_array_mut().unwrap().push(adapter);
            }),
        ),
    ];

    for (name, mutate) in cases {
        let mut value = original.clone();
        mutate(&mut value);
        let rejected = match serde_json::from_value::<StatusHeartbeat>(value) {
            Ok(heartbeat) => heartbeat.validate().is_err(),
            Err(_) => true,
        };
        assert!(rejected, "{name} was accepted");
    }
}

#[test]
fn rust_decodes_and_correlates_v1_ack_fixture() {
    let batch: RecordBatch = serde_json::from_str(BATCH_FIXTURE).unwrap();
    let ack: AcceptedThrough = serde_json::from_str(ACK_FIXTURE).unwrap();
    ack.validate_for(&batch, 0).unwrap();
}

#[test]
fn validated_prior_ack_is_a_safe_stale_duplicate() {
    let mut current: RecordBatch = serde_json::from_str(BATCH_FIXTURE).unwrap();
    current.cursor_start = 2;
    current.cursor_end = 2;
    current.publication_id = publication_id("edge-node-01", "epoch-01", 2, 2);
    current.records[0]["pub_seq"] = serde_json::json!(2);
    current.validate().unwrap();

    let stale = AcceptedThrough {
        schema_version: 1,
        edge_node_id: "edge-node-01".into(),
        ledger_epoch: "epoch-01".into(),
        publication_id: publication_id("edge-node-01", "epoch-01", 1, 1),
        accepted_through: 1,
    };

    stale.validate_stale_for(&current, 1).unwrap();
}

#[test]
fn validated_prior_prefix_ack_starts_at_the_current_batch() {
    let mut current: RecordBatch = serde_json::from_str(BATCH_FIXTURE).unwrap();
    current.cursor_end = 2;
    current.publication_id = publication_id("edge-node-01", "epoch-01", 1, 2);
    let mut second = current.records[0].clone();
    second["pub_seq"] = serde_json::json!(2);
    current.records.push(second);
    current.validate().unwrap();

    let prefix = AcceptedThrough {
        schema_version: 1,
        edge_node_id: "edge-node-01".into(),
        ledger_epoch: "epoch-01".into(),
        publication_id: publication_id("edge-node-01", "epoch-01", 1, 1),
        accepted_through: 1,
    };

    prefix.validate_prior_prefix_for(&current, 0).unwrap();
}

#[test]
fn prior_prefix_ack_rejects_non_prefix_or_noncanonical_correlation() {
    let mut current: RecordBatch = serde_json::from_str(BATCH_FIXTURE).unwrap();
    current.cursor_end = 2;
    current.publication_id = publication_id("edge-node-01", "epoch-01", 1, 2);
    let mut second = current.records[0].clone();
    second["pub_seq"] = serde_json::json!(2);
    current.records.push(second);
    current.validate().unwrap();

    let invalid_for_prefix = [
        (
            AcceptedThrough {
                schema_version: 2,
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: "epoch-01".into(),
                publication_id: publication_id("edge-node-01", "epoch-01", 1, 1),
                accepted_through: 1,
            },
            0,
        ),
        (
            AcceptedThrough {
                schema_version: 1,
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: "epoch-01".into(),
                publication_id: "not-a-publication-id".into(),
                accepted_through: 1,
            },
            0,
        ),
        (
            AcceptedThrough {
                schema_version: 1,
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: "epoch-01".into(),
                publication_id: "edge-node-01:epoch-01:01:01".into(),
                accepted_through: 1,
            },
            0,
        ),
        (
            AcceptedThrough {
                schema_version: 1,
                edge_node_id: "edge-other".into(),
                ledger_epoch: "epoch-01".into(),
                publication_id: publication_id("edge-other", "epoch-01", 1, 1),
                accepted_through: 1,
            },
            0,
        ),
        (
            AcceptedThrough {
                schema_version: 1,
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: "epoch-other".into(),
                publication_id: publication_id("edge-node-01", "epoch-other", 1, 1),
                accepted_through: 1,
            },
            0,
        ),
        (
            AcceptedThrough {
                schema_version: 1,
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: "epoch-01".into(),
                publication_id: publication_id("edge-node-01", "epoch-01", 2, 2),
                accepted_through: 2,
            },
            0,
        ),
        (
            AcceptedThrough {
                schema_version: 1,
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: "epoch-01".into(),
                publication_id: publication_id("edge-node-01", "epoch-01", 1, 2),
                accepted_through: 2,
            },
            0,
        ),
        (
            AcceptedThrough {
                schema_version: 1,
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: "epoch-01".into(),
                publication_id: publication_id("edge-node-01", "epoch-01", 1, 3),
                accepted_through: 3,
            },
            0,
        ),
        (
            AcceptedThrough {
                schema_version: 1,
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: "epoch-01".into(),
                publication_id: publication_id("edge-node-01", "epoch-01", 1, 1),
                accepted_through: 1,
            },
            1,
        ),
    ];

    for (ack, prior_cursor) in invalid_for_prefix {
        assert!(
            ack.validate_prior_prefix_for(&current, prior_cursor)
                .is_err(),
            "{ack:?}",
        );
    }
}

#[test]
fn stale_ack_requires_a_prior_deterministic_correlation() {
    let mut current: RecordBatch = serde_json::from_str(BATCH_FIXTURE).unwrap();
    current.cursor_start = 2;
    current.cursor_end = 2;
    current.publication_id = publication_id("edge-node-01", "epoch-01", 2, 2);
    current.records[0]["pub_seq"] = serde_json::json!(2);

    let invalid_for_stale = [
        AcceptedThrough {
            schema_version: 2,
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: publication_id("edge-node-01", "epoch-01", 1, 1),
            accepted_through: 1,
        },
        AcceptedThrough {
            schema_version: 1,
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: "not-a-publication-id".into(),
            accepted_through: 1,
        },
        AcceptedThrough {
            schema_version: 1,
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: publication_id("edge-node-01", "epoch-01", 0, 0),
            accepted_through: 0,
        },
        AcceptedThrough {
            schema_version: 1,
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: publication_id("edge-node-01", "epoch-01", 2, 1),
            accepted_through: 1,
        },
        AcceptedThrough {
            schema_version: 1,
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: publication_id("edge-node-01", "epoch-01", 1, 2),
            accepted_through: 1,
        },
        AcceptedThrough {
            schema_version: 1,
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: "edge-node-01:epoch-01:01:01".into(),
            accepted_through: 1,
        },
        AcceptedThrough {
            schema_version: 1,
            edge_node_id: "edge-other".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: publication_id("edge-other", "epoch-01", 1, 1),
            accepted_through: 1,
        },
        AcceptedThrough {
            schema_version: 1,
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-other".into(),
            publication_id: publication_id("edge-node-01", "epoch-other", 1, 1),
            accepted_through: 1,
        },
        AcceptedThrough {
            schema_version: 1,
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: publication_id("edge-node-01", "epoch-01", 2, 2),
            accepted_through: 1,
        },
        AcceptedThrough {
            schema_version: 1,
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: publication_id("edge-node-01", "epoch-01", 3, 3),
            accepted_through: 3,
        },
        AcceptedThrough {
            schema_version: 1,
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: publication_id("edge-node-01", "epoch-01", 2, 2),
            accepted_through: 2,
        },
    ];

    for ack in invalid_for_stale {
        assert!(ack.validate_stale_for(&current, 1).is_err(), "{ack:?}");
    }
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
