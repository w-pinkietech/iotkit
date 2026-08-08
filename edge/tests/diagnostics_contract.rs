use iotkit_edge::{
    application::{
        output_profiles::OutputProfiles,
        semantics::{SemanticRuleDraft, Semantics},
    },
    composition::registered_output_adapters,
    diagnostics::{
        DiagnosticState, StorageState, diagnostics, diagnostics_with_certificate, storage_status,
    },
    semantics::{Detector, RuleSpec, SemanticKind, TriggerMode},
    storage::{AcceptBatch, RawRecord, Storage, StorageProfile},
};
use iotkit_edge_custody_contract::DescriptorSnapshot;
use serde_json::Map;
use tempfile::TempDir;

#[tokio::test]
async fn sqlite_capacity_reports_current_projection_queue_and_output_tables() {
    let directory = TempDir::new().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("edge.db"),
    })
    .await
    .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    let descriptor = DescriptorSnapshot::decode(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "edge_node_id": "node",
            "ledger_epoch": "epoch",
            "descriptor_revision": 1,
            "complete": true,
            "devices": [{
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "identifier": "diagnostic-device",
                "state": "active",
                "model_id": "contract"
            }],
            "signals": [{
                "series_key": "018f0000-0000-7000-8000-000000000001:temperature:na:primary",
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "measurement_key": "temperature",
                "channel_index": null,
                "variant": "primary",
                "unit": null,
                "value_type": "float"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    storage.apply_descriptor(&descriptor, 2).await.unwrap();
    let semantics = Semantics::new(storage.clone());
    semantics
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: "node".into(),
                series_key: descriptor.signals[0].series_key.clone(),
                display_name: "Diagnostic temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            3,
        )
        .await
        .unwrap();
    OutputProfiles::new(storage.clone(), registered_output_adapters())
        .activate("Diagnostic output", "iotkit.mqtt-json.v1", Map::new(), 4)
        .await
        .unwrap();
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "node".into(),
            ledger_epoch: "epoch".into(),
            publication_id: "publication".into(),
            received_at: 5,
            records: vec![
                RawRecord::new(
                    1,
                    serde_json::to_vec(&serde_json::json!({
                        "family": "measurement",
                        "schema_version": 1,
                        "epoch": "epoch",
                        "pub_seq": 1,
                        "series_key": descriptor.signals[0].series_key,
                        "values": [21.5],
                        "event_time": 5,
                        "event_time_source": "received_at",
                        "time_source": "edge_node",
                        "time_quality": "unsynced",
                        "received_at": 5,
                        "device_time": null
                    }))
                    .unwrap(),
                )
                .unwrap(),
            ],
        })
        .await
        .unwrap();

    let capacity = storage_status(&storage, 90).await.unwrap();
    assert!(capacity.filesystem_available);
    assert!(matches!(
        capacity.state,
        StorageState::Healthy | StorageState::Warning | StorageState::Critical
    ));
    assert_eq!(capacity.raw_record_count, 1);
    assert_eq!(capacity.semantic_observation_count, 0);
    assert_eq!(capacity.pending_semantic_projection_count, 1);
    assert_eq!(capacity.pending_output_count, 0);
    semantics
        .project_pending(1, registered_output_adapters())
        .await
        .unwrap();
    let capacity = storage_status(&storage, 90).await.unwrap();
    assert_eq!(capacity.semantic_observation_count, 1);
    assert_eq!(capacity.pending_semantic_projection_count, 0);
    assert_eq!(capacity.pending_output_count, 1);

    let report = diagnostics(&storage, 90, 300_006).await.unwrap();
    assert_eq!(report.state, DiagnosticState::Attention);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "edge_backup_missing")
    );
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "output_delivery_stale")
    );
}

#[tokio::test]
async fn invalid_warning_threshold_is_rejected() {
    let directory = TempDir::new().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("edge.db"),
    })
    .await
    .unwrap();
    assert!(storage_status(&storage, 49).await.is_err());
    assert!(storage_status(&storage, 100).await.is_err());
}

#[tokio::test]
async fn recovery_and_certificate_causes_are_visible() {
    let directory = TempDir::new().unwrap();
    let database = directory.path().join("edge.db");
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: database.clone(),
    })
    .await
    .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", database.display()))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO edge_node_activations(edge_node_ref,edge_node_id,ledger_epoch,state,revision,\
         created_at,updated_at) VALUES('node-ref','node','epoch','recovery_hold',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO edge_restore_events VALUES('restore','backup',1,1,(SELECT edge_id FROM \
         edge_meta),5,?)",
    )
    .bind("0".repeat(64))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO edge_restore_cursor_checks(restore_id,edge_node_id,ledger_epoch,\
         backup_accepted_through,state,observed_cursor_start,updated_at) \
         VALUES('restore','node','epoch',1,'recovery_required',5,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let report = diagnostics_with_certificate(
        &storage,
        90,
        1_000_000,
        Some(&directory.path().join("missing.pem")),
    )
    .await
    .unwrap();
    assert_eq!(report.state, DiagnosticState::Critical);
    let certificate = report
        .broker_certificate
        .as_ref()
        .expect("certificate view");
    assert!(!certificate.available);
    assert!(certificate.needs_action);
    for code in [
        "edge_node_recovery_hold",
        "archive_recovery_required",
        "broker_certificate_unavailable",
    ] {
        assert!(
            report.issues.iter().any(|issue| issue.code == code),
            "missing diagnostic issue {code}"
        );
    }
}
