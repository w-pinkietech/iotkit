use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};

use iotkit_edge::{
    application::recovery::{
        BackupInspection, BrokerFenceReceipt, RecoveryService, RestoreReceipt,
    },
    recovery_control::{
        RecoveryControlError, RecoveryControlRequest, RecoveryControlResponse,
        call_recovery_control, run_recovery_control,
    },
    storage::{AcceptBatch, RawRecord, Storage, StorageProfile},
};
use iotkit_edge_custody_contract::{ActivationRequest, ActivationResult, DescriptorSnapshot};
use tempfile::TempDir;
use tokio::{
    io::AsyncWriteExt,
    net::{UnixListener, UnixStream},
};
use tokio_util::sync::CancellationToken;

async fn active_store() -> (TempDir, Storage, PathBuf) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).unwrap();
    let directory = TempDir::new_in(root).unwrap();
    let database = directory.path().join("recovery-control.db");
    let store = Storage::connect(StorageProfile::Sqlite {
        path: database.clone(),
    })
    .await
    .unwrap();
    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    store.apply_descriptor(&descriptor, 1).await.unwrap();
    let command = store
        .request_activation(&descriptor.edge_node_id, 2)
        .await
        .unwrap();
    let activation = ActivationRequest::decode(&command.payload_json).unwrap();
    store
        .apply_activation_result(
            &ActivationResult {
                schema_version: 1,
                activation_id: activation.activation_id,
                edge_id: activation.edge_id,
                edge_node_id: activation.edge_node_id,
                ledger_epoch: activation.expected_ledger_epoch,
                status: "applied".into(),
                discard_through_reading_seq: 0,
                first_publication_seq: 1,
                applied_at: 3,
            },
            3,
        )
        .await
        .unwrap();
    store
        .accept_active_batch(AcceptBatch {
            edge_node_id: descriptor.edge_node_id,
            ledger_epoch: descriptor.ledger_epoch,
            publication_id: "edge-node-01:epoch-01:1:1".into(),
            received_at: 4,
            records: vec![
                RawRecord::new(
                    1,
                    br#"{"schema_version":1,"series_key":"signal-01","event_time":4,"values":{"temperature":20}}"#,
                )
                .unwrap(),
            ],
        })
        .await
        .unwrap();
    (directory, store, database)
}

async fn wait_for_socket(path: &std::path::Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("recovery control socket did not appear");
}

#[tokio::test]
async fn running_storage_accepts_prepare_authorize_and_report_over_owner_only_socket() {
    let (_directory, store, database) = active_store().await;
    assert!(
        Storage::connect(StorageProfile::Sqlite { path: database })
            .await
            .is_err(),
        "the acceptance path must not rely on taking a second storage lock"
    );
    let socket_directory = tempfile::tempdir().unwrap();
    let socket = socket_directory.path().join("recovery-control.sock");
    let cancellation = CancellationToken::new();
    let server = tokio::spawn(run_recovery_control(
        socket.clone(),
        RecoveryService::new(store.clone()),
        cancellation.clone(),
    ));
    wait_for_socket(&socket).await;
    assert_eq!(
        std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let second = run_recovery_control(
        socket.clone(),
        RecoveryService::new(store.clone()),
        CancellationToken::new(),
    )
    .await;
    assert!(matches!(second, Err(RecoveryControlError::SocketInUse)));

    // A client that disconnects without a request is isolated to that client.
    drop(UnixStream::connect(&socket).await.unwrap());

    let handoff = match call_recovery_control(
        &socket,
        &RecoveryControlRequest::Prepare {
            inspection: BackupInspection {
                status: "authenticated".into(),
                artifact_kind: "iotkit-node-backup".into(),
                format_version: 1,
                backup_id: "backup-0123456789abcdef0123456789abcdef".into(),
                edge_node_id: "edge-node-01".into(),
                ledger_epoch: "epoch-01".into(),
                // A Node clock ahead of the Edge/Broker clock is valid.
                created_at_ms: i64::MAX - 1,
                accepted_cursor: 1,
                allocation_high_water: 5,
                epoch_start_publication_seq: Some(1),
                snapshot_mode: "online".into(),
                schema_version: 24,
                database_length: 4096,
            },
            fence: BrokerFenceReceipt {
                schema_version: 1,
                status: "fenced".into(),
                fence_id: "fence-0123456789abcdef0123456789abcdef".into(),
                edge_node_id: "edge-node-01".into(),
                credential_generation: 2,
                fenced_at: 1,
            },
        },
    )
    .await
    .unwrap()
    {
        RecoveryControlResponse::Prepared { handoff } => handoff,
        _ => panic!("prepare was not accepted"),
    };
    match call_recovery_control(
        &socket,
        &RecoveryControlRequest::Authorize {
            receipt: RestoreReceipt {
                schema_version: 2,
                status: "durably_fenced_candidate".into(),
                recovery_id: handoff.recovery_id.clone(),
                candidate_instance_id: "candidate-0123456789abcdef0123456789abcdef".into(),
                backup_id: handoff.expected_backup_id.clone().unwrap(),
                edge_id: handoff.edge_id,
                edge_node_id: handoff.edge_node_id,
                old_ledger_epoch: handoff.old_ledger_epoch,
                proposed_new_epoch: handoff.proposed_new_epoch,
                credential_generation: handoff.credential_generation,
                device_auth_generation: 3,
            },
        },
    )
    .await
    .unwrap()
    {
        RecoveryControlResponse::Authorized { .. } => {}
        _ => panic!("authorize was not accepted"),
    }
    match call_recovery_control(
        &socket,
        &RecoveryControlRequest::Report {
            recovery_id: handoff.recovery_id,
        },
    )
    .await
    .unwrap()
    {
        RecoveryControlResponse::Report { report } => {
            assert_eq!(report.state, "authorized");
            assert!(!report.completion_acknowledged);
            assert_eq!(report.recovery_window_ms, None);
        }
        _ => panic!("report was not returned"),
    }

    cancellation.cancel();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn partial_client_times_out_without_terminating_the_control_server() {
    let (_directory, store, _) = active_store().await;
    let socket_directory = tempfile::tempdir().unwrap();
    let socket = socket_directory.path().join("recovery-control.sock");
    let cancellation = CancellationToken::new();
    let server = tokio::spawn(run_recovery_control(
        socket.clone(),
        RecoveryService::new(store),
        cancellation.clone(),
    ));
    wait_for_socket(&socket).await;

    let mut partial = UnixStream::connect(&socket).await.unwrap();
    partial.write_all(b"{").await.unwrap();
    tokio::time::sleep(Duration::from_millis(5_100)).await;
    drop(partial);

    let response = call_recovery_control(
        &socket,
        &RecoveryControlRequest::Report {
            recovery_id: "recovery-ffffffffffffffffffffffffffffffff".into(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(response, RecoveryControlResponse::Rejected { .. }));

    cancellation.cancel();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn operator_client_times_out_when_a_listener_never_responds() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("wedged.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let wedged = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(7)).await;
    });

    let error = call_recovery_control(
        &socket,
        &RecoveryControlRequest::Report {
            recovery_id: "recovery-ffffffffffffffffffffffffffffffff".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(error, RecoveryControlError::Timeout));
    wedged.abort();
}
