use iotkit_edge::{
    diagnostics::{
        DiagnosticState, StorageState, diagnostics, diagnostics_with_certificate, storage_status,
    },
    storage::{AcceptBatch, RawRecord, Storage, StorageProfile},
};
use tempfile::TempDir;

#[tokio::test]
async fn sqlite_capacity_and_missing_backup_are_reported_without_claiming_queue_tables_exist() {
    let directory = TempDir::new().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("edge.db"),
    })
    .await
    .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "node".into(),
            ledger_epoch: "epoch".into(),
            publication_id: "publication".into(),
            received_at: 1,
            records: vec![RawRecord::new(1, br#"{"value":1}"#).unwrap()],
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
    assert_eq!(capacity.pending_output_count, 0);

    let report = diagnostics(&storage, 90, 1_721_800_000_000).await.unwrap();
    assert_eq!(report.state, DiagnosticState::Attention);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "edge_backup_missing")
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
async fn recovery_output_and_certificate_causes_are_visible() {
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
    sqlx::query("CREATE TABLE output_outbox_v3(created_at INTEGER NOT NULL,published_at INTEGER)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO output_outbox_v3 VALUES(1,NULL)")
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
        "output_delivery_stale",
        "broker_certificate_unavailable",
    ] {
        assert!(
            report.issues.iter().any(|issue| issue.code == code),
            "missing diagnostic issue {code}"
        );
    }
}
