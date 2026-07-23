use iotkit_edge::{
    diagnostics::{DiagnosticState, StorageState, diagnostics, storage_status},
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
