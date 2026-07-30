use iotkit_edge::application::recovery::{BackupInspection, BrokerFenceReceipt, RecoveryService};
use iotkit_edge::storage::{
    AcceptBatch, RawRecord, RecoveryPrepare, Storage, StorageError, StorageProfile,
};
use iotkit_edge_custody_contract::{
    ActivationRequest, ActivationResult, DescriptorSnapshot, RecoveryActivationRequest,
    RecoveryActivationResult, RecoveryCompletionAck,
};
use std::path::PathBuf;
use tempfile::TempDir;

async fn active_store() -> (TempDir, Storage, String) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).unwrap();
    let directory = TempDir::new_in(root).unwrap();
    let store = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("recovery.db"),
    })
    .await
    .unwrap();
    let (store, edge_id) = seed_active_store(store).await;
    (directory, store, edge_id)
}

async fn seed_active_store(store: Storage) -> (Storage, String) {
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
    let edge_id = activation.edge_id.clone();
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
    (store, edge_id)
}

fn request(edge_id: &str) -> RecoveryActivationRequest {
    RecoveryActivationRequest {
        schema_version: 1,
        recovery_id: "recovery-0123456789abcdef0123456789abcdef".into(),
        edge_id: edge_id.into(),
        edge_node_id: "edge-node-01".into(),
        candidate_instance_id: "candidate-0123456789abcdef0123456789abcdef".into(),
        backup_id: "backup-0123456789abcdef0123456789abcdef".into(),
        old_ledger_epoch: "epoch-01".into(),
        new_ledger_epoch: "epoch-02".into(),
        broker_credential_generation: 2,
        device_auth_generation: 4,
        snapshot_accepted_through: 1,
        snapshot_allocation_high_water: 5,
        snapshot_epoch_start_publication_seq: None,
        edge_accepted_through: 1,
        grant_revision: 1,
        issued_at: 6,
    }
}

fn result(request: &RecoveryActivationRequest) -> RecoveryActivationResult {
    RecoveryActivationResult {
        schema_version: 1,
        recovery_id: request.recovery_id.clone(),
        edge_id: request.edge_id.clone(),
        edge_node_id: request.edge_node_id.clone(),
        candidate_instance_id: request.candidate_instance_id.clone(),
        backup_id: request.backup_id.clone(),
        old_ledger_epoch: request.old_ledger_epoch.clone(),
        new_ledger_epoch: request.new_ledger_epoch.clone(),
        broker_credential_generation: request.broker_credential_generation,
        device_auth_generation: request.device_auth_generation,
        status: "applied".into(),
        edge_accepted_through: request.edge_accepted_through,
        replayed_records: 4,
        first_new_publication_seq: 1,
        last_new_publication_seq: 5,
        applied_at: 7,
    }
}

async fn authorized_store() -> (TempDir, Storage, RecoveryActivationRequest) {
    let (directory, store, edge_id) = active_store().await;
    let request = request(&edge_id);
    store
        .prepare_edge_node_recovery(
            &RecoveryPrepare {
                recovery_id: request.recovery_id.clone(),
                edge_node_id: request.edge_node_id.clone(),
                backup_id: request.backup_id.clone(),
                old_ledger_epoch: request.old_ledger_epoch.clone(),
                new_ledger_epoch: request.new_ledger_epoch.clone(),
                broker_fence_id: "fence-0123456789abcdef0123456789abcdef".into(),
                broker_credential_generation: request.broker_credential_generation,
                backup_created_at: 1,
                broker_fenced_at: 4,
                snapshot_accepted_through: request.snapshot_accepted_through,
                snapshot_allocation_high_water: request.snapshot_allocation_high_water,
                snapshot_epoch_start_publication_seq: request.snapshot_epoch_start_publication_seq,
            },
            5,
        )
        .await
        .unwrap();
    store
        .authorize_edge_node_recovery(&request, 6)
        .await
        .unwrap();
    (directory, store, request)
}

async fn assert_epoch_start_replay_boundary(
    snapshot_high_water: i64,
    epoch_start_sequence: i64,
    expected_replayed: i64,
    expected_last_sequence: i64,
) {
    let (_directory, store, edge_id) = active_store().await;
    let mut request = request(&edge_id);
    request.snapshot_allocation_high_water = snapshot_high_water;
    request.snapshot_epoch_start_publication_seq = Some(epoch_start_sequence);
    request.edge_accepted_through = 1;
    store
        .prepare_edge_node_recovery(
            &RecoveryPrepare {
                recovery_id: request.recovery_id.clone(),
                edge_node_id: request.edge_node_id.clone(),
                backup_id: request.backup_id.clone(),
                old_ledger_epoch: request.old_ledger_epoch.clone(),
                new_ledger_epoch: request.new_ledger_epoch.clone(),
                broker_fence_id: "fence-0123456789abcdef0123456789abcdef".into(),
                broker_credential_generation: request.broker_credential_generation,
                backup_created_at: 1,
                broker_fenced_at: 4,
                snapshot_accepted_through: request.snapshot_accepted_through,
                snapshot_allocation_high_water: request.snapshot_allocation_high_water,
                snapshot_epoch_start_publication_seq: request.snapshot_epoch_start_publication_seq,
            },
            5,
        )
        .await
        .unwrap();
    store
        .authorize_edge_node_recovery(&request, 6)
        .await
        .unwrap();
    let result = RecoveryActivationResult {
        schema_version: 1,
        recovery_id: request.recovery_id.clone(),
        edge_id: request.edge_id.clone(),
        edge_node_id: request.edge_node_id.clone(),
        candidate_instance_id: request.candidate_instance_id.clone(),
        backup_id: request.backup_id.clone(),
        old_ledger_epoch: request.old_ledger_epoch.clone(),
        new_ledger_epoch: request.new_ledger_epoch.clone(),
        broker_credential_generation: request.broker_credential_generation,
        device_auth_generation: request.device_auth_generation,
        status: "applied".into(),
        edge_accepted_through: request.edge_accepted_through,
        replayed_records: expected_replayed,
        first_new_publication_seq: 1,
        last_new_publication_seq: expected_last_sequence,
        applied_at: 7,
    };
    store
        .apply_edge_node_recovery_result(&result, 8)
        .await
        .unwrap();
}

#[tokio::test]
async fn edge_excludes_an_unaccepted_snapshot_epoch_start_from_expected_replay() {
    assert_epoch_start_replay_boundary(2, 2, 0, 1).await;
    assert_epoch_start_replay_boundary(3, 2, 1, 2).await;
}

#[tokio::test]
async fn recovery_authority_rejects_a_different_edge_identity_before_enqueue() {
    let (_directory, store, edge_id) = active_store().await;
    let mut request = request(&edge_id);
    store
        .prepare_edge_node_recovery(
            &RecoveryPrepare {
                recovery_id: request.recovery_id.clone(),
                edge_node_id: request.edge_node_id.clone(),
                backup_id: request.backup_id.clone(),
                old_ledger_epoch: request.old_ledger_epoch.clone(),
                new_ledger_epoch: request.new_ledger_epoch.clone(),
                broker_fence_id: "fence-0123456789abcdef0123456789abcdef".into(),
                broker_credential_generation: request.broker_credential_generation,
                backup_created_at: 1,
                broker_fenced_at: 4,
                snapshot_accepted_through: request.snapshot_accepted_through,
                snapshot_allocation_high_water: request.snapshot_allocation_high_water,
                snapshot_epoch_start_publication_seq: request.snapshot_epoch_start_publication_seq,
            },
            5,
        )
        .await
        .unwrap();
    request.edge_id = "edge-ffffffffffffffffffffffffffffffff".into();

    assert!(matches!(
        store.authorize_edge_node_recovery(&request, 6).await,
        Err(StorageError::RecoveryConflict)
    ));
    assert!(
        store
            .pending_recovery_commands(10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn recovery_result_rejects_both_under_and_over_replay_claims() {
    for replayed_records in [3, 5] {
        let (_directory, store, request) = authorized_store().await;
        let mut result = result(&request);
        result.replayed_records = replayed_records;
        result.last_new_publication_seq = replayed_records + 1;

        assert!(matches!(
            store.apply_edge_node_recovery_result(&result, 8).await,
            Err(StorageError::RecoveryConflict)
        ));
        assert_eq!(
            store
                .recovery_case(&request.recovery_id)
                .await
                .unwrap()
                .state,
            "recovery_hold"
        );
    }
}

#[tokio::test]
async fn recovery_result_rejects_a_different_edge_identity() {
    let (_directory, store, request) = authorized_store().await;
    let mut result = result(&request);
    result.edge_id = "edge-ffffffffffffffffffffffffffffffff".into();

    assert!(matches!(
        store.apply_edge_node_recovery_result(&result, 8).await,
        Err(StorageError::RecoveryConflict)
    ));
    assert_eq!(
        store
            .recovery_case(&request.recovery_id)
            .await
            .unwrap()
            .state,
        "recovery_hold"
    );
}

#[tokio::test]
async fn mismatching_completion_ack_fails_closed_without_acknowledging_completion() {
    let (_directory, store, request) = authorized_store().await;
    let completion = store
        .apply_edge_node_recovery_result(&result(&request), 8)
        .await
        .unwrap();
    let acknowledgement = RecoveryCompletionAck {
        schema_version: 1,
        recovery_id: completion.recovery_id,
        edge_id: completion.edge_id,
        edge_node_id: completion.edge_node_id,
        candidate_instance_id: "candidate-ffffffffffffffffffffffffffffffff".into(),
        new_ledger_epoch: completion.new_ledger_epoch,
        status: "completion_stored".into(),
        acknowledged_at: 9,
    };

    assert!(matches!(
        store
            .acknowledge_edge_node_recovery_completion(&acknowledgement, 9)
            .await,
        Err(StorageError::RecoveryConflict)
    ));
    assert!(
        !store
            .recovery_completion_acknowledged(&request.recovery_id)
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .recovery_case(&request.recovery_id)
            .await
            .unwrap()
            .state,
        "recovery_hold"
    );
}

#[test]
fn recovery_evidence_fixtures_decode_as_closed_documents() {
    let inspection: BackupInspection = serde_json::from_value(serde_json::json!({
        "status": "authenticated",
        "artifact_kind": "iotkit-node-backup",
        "format_version": 1,
        "backup_id": "backup-0123456789abcdef0123456789abcdef",
        "edge_node_id": "edge-node-01",
        "ledger_epoch": "epoch-01",
        "created_at_ms": 1,
        "accepted_cursor": 1,
        "allocation_high_water": 5,
        "epoch_start_publication_seq": 1,
        "snapshot_mode": "online",
        "schema_version": 23,
        "database_length": 4096
    }))
    .unwrap();
    assert_eq!(inspection.schema_version, 23);
    let mut inspection_with_unknown = serde_json::to_value(serde_json::json!({
        "status": "authenticated",
        "artifact_kind": "iotkit-node-backup",
        "format_version": 1,
        "backup_id": "backup-0123456789abcdef0123456789abcdef",
        "edge_node_id": "edge-node-01",
        "ledger_epoch": "epoch-01",
        "created_at_ms": 1,
        "accepted_cursor": 1,
        "allocation_high_water": 5,
        "epoch_start_publication_seq": 1,
        "snapshot_mode": "online",
        "schema_version": 23,
        "database_length": 4096,
        "unexpected": true
    }))
    .unwrap();
    assert!(serde_json::from_value::<BackupInspection>(inspection_with_unknown.take()).is_err());

    let fence: BrokerFenceReceipt = serde_json::from_slice(include_bytes!(
        "../../edge-node/core/recovery/tests/fixtures/broker-fence-receipt-v1.json"
    ))
    .unwrap();
    assert_eq!(fence.fence_id, "fence-0123456789abcdef0123456789abcdef");

    let mut fence_with_unknown: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../edge-node/core/recovery/tests/fixtures/broker-fence-receipt-v1.json"
    ))
    .unwrap();
    fence_with_unknown["password"] = "must-not-be-accepted".into();
    assert!(serde_json::from_value::<BrokerFenceReceipt>(fence_with_unknown).is_err());

    let restore: iotkit_edge::application::recovery::RestoreReceipt = serde_json::from_slice(
        include_bytes!("../../edge-node/core/recovery/tests/fixtures/restore-receipt-v2.json"),
    )
    .unwrap();
    assert_eq!(restore.schema_version, 2);
    assert_eq!(restore.device_auth_generation, 3);
}

#[tokio::test]
async fn recovery_is_durable_idempotent_and_keeps_the_new_epoch_closed_until_matching_result() {
    let (_directory, store, edge_id) = active_store().await;
    let request = request(&edge_id);
    let case = store
        .prepare_edge_node_recovery(
            &RecoveryPrepare {
                recovery_id: request.recovery_id.clone(),
                edge_node_id: request.edge_node_id.clone(),
                backup_id: request.backup_id.clone(),
                old_ledger_epoch: request.old_ledger_epoch.clone(),
                new_ledger_epoch: request.new_ledger_epoch.clone(),
                broker_fence_id: "fence-0123456789abcdef0123456789abcdef".into(),
                broker_credential_generation: request.broker_credential_generation,
                backup_created_at: 1,
                broker_fenced_at: 4,
                snapshot_accepted_through: request.snapshot_accepted_through,
                snapshot_allocation_high_water: request.snapshot_allocation_high_water,
                snapshot_epoch_start_publication_seq: request.snapshot_epoch_start_publication_seq,
            },
            5,
        )
        .await
        .unwrap();
    assert_eq!(case.edge_accepted_through, 1);
    let exact_replay = store
        .prepare_edge_node_recovery(
            &RecoveryPrepare {
                recovery_id: request.recovery_id.clone(),
                edge_node_id: request.edge_node_id.clone(),
                backup_id: request.backup_id.clone(),
                old_ledger_epoch: request.old_ledger_epoch.clone(),
                new_ledger_epoch: request.new_ledger_epoch.clone(),
                broker_fence_id: "fence-0123456789abcdef0123456789abcdef".into(),
                broker_credential_generation: request.broker_credential_generation,
                backup_created_at: 1,
                broker_fenced_at: 4,
                snapshot_accepted_through: request.snapshot_accepted_through,
                snapshot_allocation_high_water: request.snapshot_allocation_high_water,
                snapshot_epoch_start_publication_seq: request.snapshot_epoch_start_publication_seq,
            },
            99,
        )
        .await
        .unwrap();
    assert_eq!(exact_replay, case);
    assert!(matches!(
        store
            .accept_active_batch(AcceptBatch {
                edge_node_id: request.edge_node_id.clone(),
                ledger_epoch: request.old_ledger_epoch.clone(),
                publication_id: "edge-node-01:epoch-01:2:2".into(),
                received_at: 6,
                records: vec![
                    RawRecord::new(
                        2,
                        br#"{"schema_version":1,"series_key":"signal-01","event_time":6,"values":{"temperature":21}}"#,
                    )
                    .unwrap(),
                ],
            })
            .await,
        Err(StorageError::EdgeNodeNotActive)
    ));

    let command = store
        .authorize_edge_node_recovery(&request, 6)
        .await
        .unwrap();
    assert_eq!(
        command.topic,
        "iotkit/v1/edge-nodes/edge-node-01/recovery/request"
    );
    assert_eq!(store.pending_recovery_commands(10).await.unwrap().len(), 1);
    assert_eq!(
        store
            .pending_recovery_commands_due(10, 6)
            .await
            .unwrap()
            .len(),
        1
    );
    store
        .mark_recovery_attempt(&request.recovery_id, "request", 6)
        .await
        .unwrap();
    assert!(
        store
            .pending_recovery_commands_due(10, 5_005)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .pending_recovery_commands_due(10, 5_006)
            .await
            .unwrap()
            .len(),
        1
    );

    let result = RecoveryActivationResult {
        schema_version: 1,
        recovery_id: request.recovery_id.clone(),
        edge_id: request.edge_id.clone(),
        edge_node_id: request.edge_node_id.clone(),
        candidate_instance_id: request.candidate_instance_id.clone(),
        backup_id: request.backup_id.clone(),
        old_ledger_epoch: request.old_ledger_epoch.clone(),
        new_ledger_epoch: request.new_ledger_epoch.clone(),
        broker_credential_generation: request.broker_credential_generation,
        device_auth_generation: request.device_auth_generation,
        status: "applied".into(),
        edge_accepted_through: request.edge_accepted_through,
        replayed_records: 4,
        first_new_publication_seq: 1,
        last_new_publication_seq: 5,
        applied_at: 7,
    };
    let completion = store
        .apply_edge_node_recovery_result(&result, 8)
        .await
        .unwrap();
    assert_eq!(completion.new_ledger_epoch, "epoch-02");
    assert_eq!(
        store
            .accepted_through("edge-node-01", "epoch-02")
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        store.pending_recovery_commands(10).await.unwrap()[0].kind,
        "completion"
    );

    assert_eq!(
        store
            .apply_edge_node_recovery_result(&result, 9)
            .await
            .unwrap(),
        completion
    );
    assert_eq!(store.pending_recovery_commands(10).await.unwrap().len(), 1);
    let acknowledgement = RecoveryCompletionAck {
        schema_version: 1,
        recovery_id: completion.recovery_id.clone(),
        edge_id: completion.edge_id.clone(),
        edge_node_id: completion.edge_node_id.clone(),
        candidate_instance_id: completion.candidate_instance_id.clone(),
        new_ledger_epoch: completion.new_ledger_epoch.clone(),
        status: "completion_stored".into(),
        acknowledged_at: 10,
    };
    store
        .acknowledge_edge_node_recovery_completion(&acknowledgement, 10)
        .await
        .unwrap();
    store
        .acknowledge_edge_node_recovery_completion(&acknowledgement, 11)
        .await
        .unwrap();
    let report = RecoveryService::new(store.clone())
        .report(&request.recovery_id)
        .await
        .unwrap();
    assert_eq!(report.new_epoch_accepted_through, Some(0));
    assert!(!report.cursor_converged);
    assert!(report.remaining_gap_review_required);

    store
        .accept_active_batch(AcceptBatch {
            edge_node_id: request.edge_node_id.clone(),
            ledger_epoch: request.new_ledger_epoch.clone(),
            publication_id: "edge-node-01:epoch-02:1:5".into(),
                received_at: 12,
            records: (1..=5)
                .map(|pub_seq| {
                    RawRecord::new(
                        pub_seq,
                        format!(
                            r#"{{"schema_version":1,"series_key":"signal-01","event_time":{},"values":{{"temperature":20}}}}"#,
                            12 + pub_seq
                        )
                        .as_bytes(),
                    )
                    .unwrap()
                })
                .collect(),
        })
        .await
        .unwrap();
    let converged = RecoveryService::new(store.clone())
        .report(&request.recovery_id)
        .await
        .unwrap();
    assert_eq!(converged.new_epoch_accepted_through, Some(5));
    assert!(converged.cursor_converged);
    assert_eq!(converged.backup_created_at, 1);
    assert_eq!(converged.broker_fenced_at, 4);
    assert_eq!(converged.recovery_window_ms, None);
    assert_eq!(converged.potential_unrecoverable_local_after_seq, Some(6));
    assert!(converged.remaining_gap_review_required);
    assert!(
        store
            .pending_recovery_commands(10)
            .await
            .unwrap()
            .is_empty()
    );

    let mut late_mismatch = result.clone();
    late_mismatch.candidate_instance_id = "candidate-ffffffffffffffffffffffffffffffff".into();
    assert!(matches!(
        store
            .apply_edge_node_recovery_result(&late_mismatch, 13)
            .await,
        Err(StorageError::RecoveryConflict)
    ));
    assert_eq!(
        store
            .recovery_case(&request.recovery_id)
            .await
            .unwrap()
            .state,
        "recovery_hold"
    );
    assert!(matches!(
        store
            .accept_active_batch(AcceptBatch {
                edge_node_id: request.edge_node_id,
                ledger_epoch: request.new_ledger_epoch,
                publication_id: "edge-node-01:epoch-02:6:6".into(),
                received_at: 14,
                records: vec![
                    RawRecord::new(
                        6,
                        br#"{"schema_version":1,"series_key":"signal-01","event_time":12,"values":{"temperature":22}}"#,
                    )
                    .unwrap(),
                ],
            })
            .await,
        Err(StorageError::EdgeNodeNotActive)
    ));
}

#[tokio::test]
async fn prepare_reconstructs_the_same_authority_from_the_same_fence_receipt() {
    let (_directory, store, _edge_id) = active_store().await;
    let inspection = BackupInspection {
        status: "authenticated".into(),
        artifact_kind: "iotkit-node-backup".into(),
        format_version: 1,
        backup_id: "backup-0123456789abcdef0123456789abcdef".into(),
        edge_node_id: "edge-node-01".into(),
        ledger_epoch: "epoch-01".into(),
        created_at_ms: 1,
        accepted_cursor: 1,
        allocation_high_water: 5,
        epoch_start_publication_seq: Some(1),
        snapshot_mode: "online".into(),
        schema_version: 24,
        database_length: 4096,
    };
    let fence = BrokerFenceReceipt {
        schema_version: 1,
        status: "fenced".into(),
        fence_id: "fence-0123456789abcdef0123456789abcdef".into(),
        edge_node_id: "edge-node-01".into(),
        credential_generation: 2,
        fenced_at: 4,
    };
    let service = RecoveryService::new(store);
    let first = service.prepare(&inspection, &fence, 5).await.unwrap();
    let replay = service.prepare(&inspection, &fence, 6).await.unwrap();

    assert_eq!(
        first.recovery_id,
        "recovery-0123456789abcdef0123456789abcdef"
    );
    assert_eq!(
        first.proposed_new_epoch,
        "epoch-0123456789abcdef0123456789abcdef"
    );
    assert_eq!(replay.recovery_id, first.recovery_id);
    assert_eq!(replay.proposed_new_epoch, first.proposed_new_epoch);
}

#[tokio::test]
async fn a_recovery_boundary_mismatch_fails_before_authority_is_queued() {
    let (_directory, store, edge_id) = active_store().await;
    let mut request = request(&edge_id);
    store
        .prepare_edge_node_recovery(
            &RecoveryPrepare {
                recovery_id: request.recovery_id.clone(),
                edge_node_id: request.edge_node_id.clone(),
                backup_id: request.backup_id.clone(),
                old_ledger_epoch: request.old_ledger_epoch.clone(),
                new_ledger_epoch: request.new_ledger_epoch.clone(),
                broker_fence_id: "fence-0123456789abcdef0123456789abcdef".into(),
                broker_credential_generation: request.broker_credential_generation,
                backup_created_at: 1,
                broker_fenced_at: 4,
                snapshot_accepted_through: request.snapshot_accepted_through,
                snapshot_allocation_high_water: request.snapshot_allocation_high_water,
                snapshot_epoch_start_publication_seq: request.snapshot_epoch_start_publication_seq,
            },
            5,
        )
        .await
        .unwrap();
    request.candidate_instance_id = "candidate-ffffffffffffffffffffffffffffffff".into();
    request.edge_accepted_through = 2;

    assert!(matches!(
        store.authorize_edge_node_recovery(&request, 6).await,
        Err(StorageError::RecoveryConflict)
    ));
    assert!(
        store
            .pending_recovery_commands(10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_mismatching_candidate_result_enters_recovery_hold_without_switching_epoch() {
    let (_directory, store, edge_id) = active_store().await;
    let request = request(&edge_id);
    store
        .prepare_edge_node_recovery(
            &RecoveryPrepare {
                recovery_id: request.recovery_id.clone(),
                edge_node_id: request.edge_node_id.clone(),
                backup_id: request.backup_id.clone(),
                old_ledger_epoch: request.old_ledger_epoch.clone(),
                new_ledger_epoch: request.new_ledger_epoch.clone(),
                broker_fence_id: "fence-0123456789abcdef0123456789abcdef".into(),
                broker_credential_generation: request.broker_credential_generation,
                backup_created_at: 1,
                broker_fenced_at: 4,
                snapshot_accepted_through: request.snapshot_accepted_through,
                snapshot_allocation_high_water: request.snapshot_allocation_high_water,
                snapshot_epoch_start_publication_seq: request.snapshot_epoch_start_publication_seq,
            },
            5,
        )
        .await
        .unwrap();
    store
        .authorize_edge_node_recovery(&request, 6)
        .await
        .unwrap();
    let mut result = RecoveryActivationResult {
        schema_version: 1,
        recovery_id: request.recovery_id.clone(),
        edge_id: request.edge_id.clone(),
        edge_node_id: request.edge_node_id.clone(),
        candidate_instance_id: request.candidate_instance_id.clone(),
        backup_id: request.backup_id.clone(),
        old_ledger_epoch: request.old_ledger_epoch.clone(),
        new_ledger_epoch: request.new_ledger_epoch.clone(),
        broker_credential_generation: request.broker_credential_generation,
        device_auth_generation: request.device_auth_generation,
        status: "applied".into(),
        edge_accepted_through: request.edge_accepted_through,
        replayed_records: 4,
        first_new_publication_seq: 1,
        last_new_publication_seq: 5,
        applied_at: 7,
    };
    result.candidate_instance_id = "candidate-ffffffffffffffffffffffffffffffff".into();

    assert!(matches!(
        store.apply_edge_node_recovery_result(&result, 8).await,
        Err(StorageError::RecoveryConflict)
    ));
    assert_eq!(
        store
            .recovery_case(&request.recovery_id)
            .await
            .unwrap()
            .state,
        "recovery_hold"
    );
    assert!(matches!(
        store
            .accept_active_batch(AcceptBatch {
                edge_node_id: request.edge_node_id,
                ledger_epoch: request.old_ledger_epoch,
                publication_id: "held:1:1".into(),
                received_at: 9,
                records: vec![
                    RawRecord::new(
                        2,
                        br#"{"schema_version":1,"series_key":"signal-01","event_time":9,"values":{"temperature":21}}"#,
                    )
                    .unwrap(),
                ],
            })
            .await,
        Err(StorageError::EdgeNodeNotActive)
    ));
    assert!(
        store
            .pending_recovery_commands(10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn another_nodes_result_cannot_put_this_recovery_case_on_hold() {
    let (_directory, store, edge_id) = active_store().await;
    let request = request(&edge_id);
    store
        .prepare_edge_node_recovery(
            &RecoveryPrepare {
                recovery_id: request.recovery_id.clone(),
                edge_node_id: request.edge_node_id.clone(),
                backup_id: request.backup_id.clone(),
                old_ledger_epoch: request.old_ledger_epoch.clone(),
                new_ledger_epoch: request.new_ledger_epoch.clone(),
                broker_fence_id: "fence-0123456789abcdef0123456789abcdef".into(),
                broker_credential_generation: request.broker_credential_generation,
                backup_created_at: 1,
                broker_fenced_at: 4,
                snapshot_accepted_through: request.snapshot_accepted_through,
                snapshot_allocation_high_water: request.snapshot_allocation_high_water,
                snapshot_epoch_start_publication_seq: request.snapshot_epoch_start_publication_seq,
            },
            5,
        )
        .await
        .unwrap();
    store
        .authorize_edge_node_recovery(&request, 6)
        .await
        .unwrap();
    let foreign = RecoveryActivationResult {
        schema_version: 1,
        recovery_id: request.recovery_id.clone(),
        edge_id: request.edge_id,
        edge_node_id: "edge-node-02".into(),
        candidate_instance_id: request.candidate_instance_id,
        backup_id: request.backup_id,
        old_ledger_epoch: request.old_ledger_epoch,
        new_ledger_epoch: request.new_ledger_epoch,
        broker_credential_generation: request.broker_credential_generation,
        device_auth_generation: request.device_auth_generation,
        status: "applied".into(),
        edge_accepted_through: request.edge_accepted_through,
        replayed_records: 4,
        first_new_publication_seq: 1,
        last_new_publication_seq: 5,
        applied_at: 7,
    };
    assert!(matches!(
        store.apply_edge_node_recovery_result(&foreign, 8).await,
        Err(StorageError::RecoveryConflict)
    ));
    assert_eq!(
        store
            .recovery_case(&request.recovery_id)
            .await
            .unwrap()
            .state,
        "authorized"
    );
    assert_eq!(store.pending_recovery_commands(10).await.unwrap().len(), 1);
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_recovery_freezes_old_admission_and_replays_exactly() {
    let dsn =
        std::env::var("IOTKIT_TEST_POSTGRES_DSN").expect("IOTKIT_TEST_POSTGRES_DSN must be set");
    let store = Storage::connect(StorageProfile::Postgres { dsn })
        .await
        .unwrap();
    let (store, edge_id) = seed_active_store(store).await;
    let request = request(&edge_id);
    store
        .prepare_edge_node_recovery(
            &RecoveryPrepare {
                recovery_id: request.recovery_id.clone(),
                edge_node_id: request.edge_node_id.clone(),
                backup_id: request.backup_id.clone(),
                old_ledger_epoch: request.old_ledger_epoch.clone(),
                new_ledger_epoch: request.new_ledger_epoch.clone(),
                broker_fence_id: "fence-0123456789abcdef0123456789abcdef".into(),
                broker_credential_generation: request.broker_credential_generation,
                backup_created_at: 1,
                broker_fenced_at: 4,
                snapshot_accepted_through: request.snapshot_accepted_through,
                snapshot_allocation_high_water: request.snapshot_allocation_high_water,
                snapshot_epoch_start_publication_seq: request.snapshot_epoch_start_publication_seq,
            },
            5,
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .accept_active_batch(AcceptBatch {
                edge_node_id: request.edge_node_id.clone(),
                ledger_epoch: request.old_ledger_epoch.clone(),
                publication_id: "edge-node-01:epoch-01:2:2".into(),
                received_at: 6,
                records: vec![
                    RawRecord::new(
                        2,
                        br#"{"schema_version":1,"series_key":"signal-01","event_time":6,"values":{"temperature":21}}"#,
                    )
                    .unwrap(),
                ],
            })
            .await,
        Err(StorageError::EdgeNodeNotActive)
    ));
    store
        .authorize_edge_node_recovery(&request, 6)
        .await
        .unwrap();
    let result = RecoveryActivationResult {
        schema_version: 1,
        recovery_id: request.recovery_id.clone(),
        edge_id: request.edge_id.clone(),
        edge_node_id: request.edge_node_id.clone(),
        candidate_instance_id: request.candidate_instance_id.clone(),
        backup_id: request.backup_id.clone(),
        old_ledger_epoch: request.old_ledger_epoch.clone(),
        new_ledger_epoch: request.new_ledger_epoch.clone(),
        broker_credential_generation: request.broker_credential_generation,
        device_auth_generation: request.device_auth_generation,
        status: "applied".into(),
        edge_accepted_through: request.edge_accepted_through,
        replayed_records: 4,
        first_new_publication_seq: 1,
        last_new_publication_seq: 5,
        applied_at: 7,
    };
    let completion = store
        .apply_edge_node_recovery_result(&result, 8)
        .await
        .unwrap();
    assert_eq!(
        store
            .apply_edge_node_recovery_result(&result, 9)
            .await
            .unwrap(),
        completion
    );
    store
        .acknowledge_edge_node_recovery_completion(
            &RecoveryCompletionAck {
                schema_version: 1,
                recovery_id: completion.recovery_id.clone(),
                edge_id: completion.edge_id.clone(),
                edge_node_id: completion.edge_node_id.clone(),
                candidate_instance_id: completion.candidate_instance_id.clone(),
                new_ledger_epoch: completion.new_ledger_epoch.clone(),
                status: "completion_stored".into(),
                acknowledged_at: 10,
            },
            10,
        )
        .await
        .unwrap();
    let mut mismatch = result;
    mismatch.candidate_instance_id = "candidate-ffffffffffffffffffffffffffffffff".into();
    assert!(matches!(
        store.apply_edge_node_recovery_result(&mismatch, 10).await,
        Err(StorageError::RecoveryConflict)
    ));
    assert_eq!(
        store
            .recovery_case(&request.recovery_id)
            .await
            .unwrap()
            .state,
        "recovery_hold"
    );
}
