use std::path::PathBuf;

use iotkit_edge::{
    application::{
        profiles::{InventoryProfiles, SignalProfileInput},
        semantics::{SemanticRuleDraft, Semantics},
    },
    composition::registered_output_adapters,
    semantics::{Detector, RuleSpec, SemanticKind, TriggerMode},
    storage::{AcceptBatch, AuditActor, RawHistoryQuery, RawRecord, Storage, StorageProfile},
};
use iotkit_edge_custody_contract::DescriptorSnapshot;

async fn fixture() -> (tempfile::TempDir, Storage, String, String) {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("edge.db"),
    })
    .await
    .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    storage.apply_descriptor(&descriptor, 2).await.unwrap();
    let signal = InventoryProfiles::new(storage.clone())
        .signals()
        .await
        .unwrap()[0]
        .clone();
    storage
        .update_signal_profile(
            AuditActor::local_cli(),
            &signal.signal_ref,
            SignalProfileInput {
                display_name: "Boiler temperature".into(),
                display_sensor_type: "temperature".into(),
                display_sensor_type_label: "Temperature".into(),
                display_value_kind: "numeric".into(),
                display_unit_mode: "unit".into(),
                display_unit: "°C".into(),
                decimal_places: 1,
            },
            None,
            3,
        )
        .await
        .unwrap();
    (
        directory,
        storage,
        signal.signal_ref,
        descriptor.signals[0].series_key.clone(),
    )
}

fn record(sequence: i64, series_key: &str, value: f64, received_at: i64) -> RawRecord {
    RawRecord::new(
        sequence,
        serde_json::to_vec(&serde_json::json!({
            "family":"measurement","schema_version":1,"epoch":"epoch-a",
            "pub_seq":sequence,"series_key":series_key,"values":[value],
            "event_time":received_at-1,"event_time_source":"received_at",
            "time_source":"edge_node","time_quality":"unsynced",
            "received_at":received_at,"device_time":null
        }))
        .unwrap(),
    )
    .unwrap()
}

fn boolean_record(sequence: i64, series_key: &str, value: bool, received_at: i64) -> RawRecord {
    RawRecord::new(
        sequence,
        serde_json::to_vec(&serde_json::json!({
            "family":"measurement","schema_version":1,"epoch":"epoch-a",
            "pub_seq":sequence,"series_key":series_key,"values":[value],
            "event_time":received_at-1,"event_time_source":"received_at",
            "time_source":"edge_node","time_quality":"unsynced",
            "received_at":received_at,"device_time":null
        }))
        .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn raw_history_pages_stably_across_epochs_and_keeps_profile_metadata() {
    let (_directory, storage, signal_ref, series_key) = fixture().await;
    for (epoch, sequence, value) in [
        ("epoch-a", 1, 1.0),
        ("epoch-b", 1, 99.0),
        ("epoch-b", 2, 2.0),
    ] {
        storage
            .accept_batch(AcceptBatch {
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: epoch.into(),
                publication_id: format!("{epoch}-{sequence}"),
                received_at: 100,
                records: vec![record(sequence, &series_key, value, 100)],
            })
            .await
            .unwrap();
    }
    let query = |cursor| RawHistoryQuery {
        from: 0,
        to: 1_000,
        limit: 2,
        cursor,
        signal_ref: Some(signal_ref.clone()),
        edge_node_id: None,
    };
    let first = storage.query_raw_history(query(None)).await.unwrap();
    assert!(first.has_more);
    assert_eq!(first.rows.len(), 2);
    assert_eq!(first.rows[0].display_name, "Boiler temperature");
    assert_eq!(first.rows[0].unit, "°C");
    let second = storage
        .query_raw_history(query(first.next_cursor))
        .await
        .unwrap();
    assert_eq!(second.rows.len(), 1);
    assert!(!second.has_more);
    let keys: std::collections::BTreeSet<_> = first
        .rows
        .into_iter()
        .chain(second.rows)
        .map(|row| (row.ledger_epoch, row.pub_seq))
        .collect();
    assert_eq!(keys.len(), 3);
}

#[tokio::test]
async fn semantic_history_preserves_exact_provenance_and_series_buckets_preserve_spikes() {
    let (_directory, storage, signal_ref, series_key) = fixture().await;
    let semantics = Semantics::new(storage.clone());
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-a".into(),
            publication_id: "history-seed".into(),
            received_at: 50,
            records: vec![record(1, &series_key, 0.0, 50)],
        })
        .await
        .unwrap();
    let rule = semantics
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: "edge-node-01".into(),
                series_key: series_key.clone(),
                display_name: "Temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            10,
        )
        .await
        .unwrap();
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-a".into(),
            publication_id: "history-observations".into(),
            received_at: 100,
            records: vec![
                record(2, &series_key, 1.0, 100),
                record(3, &series_key, 100.0, 110),
                record(4, &series_key, 2.0, 120),
            ],
        })
        .await
        .unwrap();
    let progress = semantics
        .project_pending(10, registered_output_adapters())
        .await
        .unwrap();
    assert_eq!(progress.observations, 3, "{progress:?}");
    let rows = storage
        .query_semantic_history(0, 1_000, 100_001, Some(&signal_ref), None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row.rule_revision == 1));
    assert!(rows.iter().all(|row| row.calibration_revision == 1));
    assert_eq!(rows[0].source_pub_seq, 4);
    assert_eq!(rows[0].rule_name, rule.display_name);
    let buckets = storage
        .query_history_series(&signal_ref, 100, 200, 100)
        .await
        .unwrap();
    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].minimum, 1.0);
    assert_eq!(buckets[0].maximum, 100.0);
    assert_eq!(buckets[0].count, 3);
}

#[tokio::test]
async fn history_series_preserves_boolean_contact_transitions_as_zero_and_one() {
    let (_directory, storage, signal_ref, series_key) = fixture().await;
    for (sequence, value, received_at) in [(1, false, 100), (2, true, 110)] {
        storage
            .accept_batch(AcceptBatch {
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: "epoch-a".into(),
                publication_id: format!("boolean-history-{sequence}"),
                received_at,
                records: vec![boolean_record(sequence, &series_key, value, received_at)],
            })
            .await
            .unwrap();
    }

    let buckets = storage
        .query_history_series(&signal_ref, 100, 200, 10)
        .await
        .unwrap();

    assert_eq!(buckets.len(), 2);
    assert_eq!(buckets[0].average, 0.0);
    assert_eq!(buckets[1].average, 1.0);
}
