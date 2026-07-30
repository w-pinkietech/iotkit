use super::*;
use crate::RecoveryHandoff;

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
enum RestoreFault {
    Decrypted,
    Copied,
    FenceCommitted,
    Checkpointed,
    CandidateFileSynced,
    RenameSucceeded,
    ParentSynced,
    PublishedReadbackVerified,
}

#[cfg(target_os = "linux")]
fn restore_with_fault(
    request: &RestoreRequest,
    passphrase: &BackupPassphrase,
    fault: RestoreFault,
) -> Result<RestoreReceipt, RecoveryError> {
    let hook = |phase: crate::restore::RestorePhase, published: bool| {
        let should_fail = matches!(
            (fault, phase),
            (
                RestoreFault::Decrypted,
                crate::restore::RestorePhase::Decrypted
            ) | (RestoreFault::Copied, crate::restore::RestorePhase::Copied)
                | (
                    RestoreFault::FenceCommitted,
                    crate::restore::RestorePhase::FenceCommitted
                )
                | (
                    RestoreFault::Checkpointed,
                    crate::restore::RestorePhase::Checkpointed
                )
                | (
                    RestoreFault::CandidateFileSynced,
                    crate::restore::RestorePhase::CandidateFileSynced
                )
                | (
                    RestoreFault::RenameSucceeded,
                    crate::restore::RestorePhase::RenameSucceeded
                )
                | (
                    RestoreFault::ParentSynced,
                    crate::restore::RestorePhase::ParentSynced
                )
                | (
                    RestoreFault::PublishedReadbackVerified,
                    crate::restore::RestorePhase::PublishedReadbackVerified
                )
        );
        if should_fail {
            Err(if published {
                RecoveryError::CandidatePublicationUncertain
            } else {
                RecoveryError::Storage
            })
        } else {
            Ok(())
        }
    };
    crate::restore::restore_candidate_inner(request, passphrase, Some(&hook))
}

#[test]
fn checked_in_restore_contracts_are_canonical_and_closed() {
    let handoff_bytes = include_bytes!("../fixtures/recovery-handoff-v1.json");
    let receipt_bytes = include_bytes!("../fixtures/restore-receipt-v2.json");
    let handoff: RecoveryHandoff = serde_json::from_slice(handoff_bytes).unwrap();
    let receipt: RestoreReceipt = serde_json::from_slice(receipt_bytes).unwrap();
    assert_eq!(
        serde_json::to_vec(&handoff).unwrap(),
        handoff_bytes.strip_suffix(b"\n").unwrap_or(handoff_bytes)
    );
    assert_eq!(
        serde_json::to_vec(&receipt).unwrap(),
        receipt_bytes.strip_suffix(b"\n").unwrap_or(receipt_bytes)
    );

    let handoff_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/recovery-handoff-v1.schema.json"
    ))
    .unwrap();
    let receipt_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/restore-receipt-v2.schema.json"
    ))
    .unwrap();
    let handoff_validator = jsonschema::validator_for(&handoff_schema).unwrap();
    let receipt_validator = jsonschema::validator_for(&receipt_schema).unwrap();
    assert!(handoff_validator.is_valid(&serde_json::to_value(&handoff).unwrap()));
    assert!(receipt_validator.is_valid(&serde_json::to_value(&receipt).unwrap()));

    let unknown = br#"{"schema_version":1,"recovery_id":"recovery-fixture","edge_id":"edge-fixture","edge_node_id":"node-fixture","old_ledger_epoch":"epoch-old-fixture","expected_backup_id":"backup-fixture","proposed_new_epoch":"epoch-new-fixture","credential_generation":2,"unexpected":true}"#;
    assert!(serde_json::from_slice::<RecoveryHandoff>(unknown).is_err());
}

#[test]
fn restore_wire_schemas_and_rust_deserializers_share_closed_validation_boundaries() {
    let handoff_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/recovery-handoff-v1.schema.json"
    ))
    .unwrap();
    let receipt_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/restore-receipt-v2.schema.json"
    ))
    .unwrap();
    let handoff_validator = jsonschema::validator_for(&handoff_schema).unwrap();
    let receipt_validator = jsonschema::validator_for(&receipt_schema).unwrap();
    let valid_handoff = serde_json::json!({
        "schema_version": 1,
        "recovery_id": "recovery-fixture",
        "edge_id": "edge-fixture",
        "edge_node_id": "node-fixture",
        "old_ledger_epoch": "epoch-old-fixture",
        "expected_backup_id": "backup-fixture",
        "proposed_new_epoch": "epoch-new-fixture",
        "credential_generation": 9223372036854775807_i64
    });
    let valid_receipt = serde_json::json!({
        "schema_version": 2,
        "status": "durably_fenced_candidate",
        "recovery_id": "recovery-fixture",
        "candidate_instance_id": "candidate-fixture",
        "backup_id": "backup-fixture",
        "edge_id": "edge-fixture",
        "edge_node_id": "node-fixture",
        "old_ledger_epoch": "epoch-old-fixture",
        "proposed_new_epoch": "epoch-new-fixture",
        "credential_generation": 9223372036854775807_i64,
        "device_auth_generation": 9223372036854775807_i64
    });
    assert!(handoff_validator.is_valid(&valid_handoff));
    assert!(receipt_validator.is_valid(&valid_receipt));
    assert!(serde_json::from_value::<RecoveryHandoff>(valid_handoff.clone()).is_ok());
    assert!(serde_json::from_value::<RestoreReceipt>(valid_receipt.clone()).is_ok());

    let invalid_handoffs = [
        ("unicode id", serde_json::json!({"recovery_id": "復旧"})),
        ("schema overflow", serde_json::json!({"schema_version": 2})),
        (
            "credential overflow",
            serde_json::json!({"credential_generation": 9223372036854775808_u128}),
        ),
        (
            "missing backup",
            serde_json::json!({"expected_backup_id": null}),
        ),
        (
            "equal epochs",
            serde_json::json!({
                "old_ledger_epoch": "same-epoch",
                "proposed_new_epoch": "same-epoch"
            }),
        ),
    ];
    for (name, patch) in invalid_handoffs {
        let mut value = valid_handoff.clone();
        if let (Some(object), Some(patch)) = (value.as_object_mut(), patch.as_object()) {
            object.extend(patch.clone());
        }
        let schema_valid = handoff_validator.is_valid(&value)
            && value["old_ledger_epoch"] != value["proposed_new_epoch"];
        assert!(!schema_valid, "schema accepted {name}");
        assert!(
            serde_json::from_value::<RecoveryHandoff>(value).is_err(),
            "Rust accepted {name}"
        );
    }

    let invalid_receipts = [
        ("unicode id", serde_json::json!({"edge_id": "復旧"})),
        ("bad status", serde_json::json!({"status": "normal"})),
        ("bad version", serde_json::json!({"schema_version": 1})),
        (
            "credential overflow",
            serde_json::json!({"credential_generation": 9223372036854775808_u128}),
        ),
        (
            "device generation overflow",
            serde_json::json!({"device_auth_generation": 9223372036854775808_u128}),
        ),
        (
            "equal epochs",
            serde_json::json!({
                "old_ledger_epoch": "same-epoch",
                "proposed_new_epoch": "same-epoch"
            }),
        ),
    ];
    for (name, patch) in invalid_receipts {
        let mut value = valid_receipt.clone();
        if let (Some(object), Some(patch)) = (value.as_object_mut(), patch.as_object()) {
            object.extend(patch.clone());
        }
        let schema_valid = receipt_validator.is_valid(&value)
            && value["old_ledger_epoch"] != value["proposed_new_epoch"];
        assert!(!schema_valid, "schema accepted {name}");
        assert!(
            serde_json::from_value::<RestoreReceipt>(value).is_err(),
            "Rust accepted {name}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn handoff_rejects_negative_generation_equal_epochs_and_missing_backup() {
    let mut handoff: RecoveryHandoff =
        serde_json::from_slice(include_bytes!("../fixtures/recovery-handoff-v1.json")).unwrap();
    let manifest = NodeBackupManifest {
        artifact_kind: "iotkit-node-backup".into(),
        format_version: 1,
        backup_id: "backup-fixture".into(),
        edge_node_id: "node-fixture".into(),
        ledger_epoch: "epoch-old-fixture".into(),
        created_at_ms: 1,
        accepted_cursor: 0,
        allocation_high_water: 0,
        epoch_start_publication_seq: None,
        snapshot_mode: SnapshotMode::Online,
        shutdown_seal_id: None,
        schema_version: 23,
        database_length: 1,
        database_sha256: "0".repeat(64),
        counts: BackupCounts::default(),
    };
    handoff.credential_generation = -1;
    assert!(matches!(
        crate::restore::validate_handoff(&handoff, &manifest),
        Err(RecoveryError::HandoffMismatch)
    ));
    handoff.credential_generation = 2;
    handoff.proposed_new_epoch = handoff.old_ledger_epoch.clone();
    assert!(matches!(
        crate::restore::validate_handoff(&handoff, &manifest),
        Err(RecoveryError::HandoffMismatch)
    ));
    handoff.proposed_new_epoch = "epoch-new-fixture".into();
    handoff.expected_backup_id = None;
    assert!(matches!(
        crate::restore::validate_handoff(&handoff, &manifest),
        Err(RecoveryError::HandoffMismatch)
    ));
}

fn request(live: &str, candidate: &str) -> RestoreRequest {
    RestoreRequest {
        input: "missing.iotkit-node-backup".into(),
        live_database: live.into(),
        candidate_database: candidate.into(),
        staging_directory: "staging".into(),
        handoff: RecoveryHandoff {
            schema_version: 1,
            recovery_id: "recovery-test".into(),
            edge_id: "edge-test".into(),
            edge_node_id: "node-test".into(),
            old_ledger_epoch: "epoch-old".into(),
            expected_backup_id: Some("backup-test".into()),
            proposed_new_epoch: "epoch-new".into(),
            credential_generation: 1,
        },
    }
}

#[test]
fn restore_api_is_present_and_fails_closed_for_equal_paths() {
    let passphrase = BackupPassphrase::new("owner-only-test-passphrase".into());
    let error = restore_candidate(&request("live.db", "live.db"), &passphrase).unwrap_err();
    assert!(matches!(
        error,
        RecoveryError::InvalidConfiguration
            | RecoveryError::PlatformUnsupported
            | RecoveryError::Storage
    ));
}

#[test]
fn install_candidate_is_private_state_and_target_free() {
    let descriptor = crate::recovery_descriptors()
        .iter()
        .find(|descriptor| descriptor.name == INSTALL_CANDIDATE_OP)
        .expect("install candidate operation");
    assert!(descriptor.changes_state);
    assert_eq!(
        (descriptor.targets)(&serde_json::json!({})),
        Vec::<String>::new()
    );
    assert_eq!(
        (descriptor.params_schema)(),
        serde_json::json!({"required": ["private_recovery_state"]})
    );
}

#[cfg(target_os = "linux")]
#[test]
fn restore_publishes_an_already_fenced_wal_independent_candidate() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let live = root.path().join("live.db");
    let candidate = root.path().join("candidate.db");
    let snapshot = root.path().join("snapshot.db");
    let artifact = root.path().join("backup.iotkit-node-backup");
    let staging = tempfile::tempdir_in("/dev/shm").unwrap();
    fs::set_permissions(staging.path(), fs::Permissions::from_mode(0o700)).unwrap();

    let source = tests_support::active_database_with_publications(&live, 0, 1);
    drop(source);
    let backup_id = "backup-restore-fixture";
    let snapshot_artifact =
        create_consistent_snapshot(&live, &snapshot, backup_id, 1_725_000_000_000).unwrap();
    let passphrase = BackupPassphrase::new("owner-only-test-passphrase".into());
    let output = DirectoryCapability::open(root.path()).unwrap();
    encrypt_container(
        &snapshot_artifact.path,
        &snapshot_artifact.manifest,
        &passphrase,
        &output,
        "backup.iotkit-node-backup",
    )
    .unwrap();

    let request = RestoreRequest {
        input: artifact,
        live_database: live.clone(),
        candidate_database: candidate.clone(),
        staging_directory: staging.path().to_path_buf(),
        handoff: RecoveryHandoff {
            schema_version: 1,
            recovery_id: "recovery-fixture".into(),
            edge_id: "edge-test".into(),
            edge_node_id: tests_support::TEST_EDGE_NODE_ID.into(),
            old_ledger_epoch: tests_support::TEST_LEDGER_EPOCH.into(),
            expected_backup_id: Some(backup_id.into()),
            proposed_new_epoch: "epoch-restored".into(),
            credential_generation: 2,
        },
    };
    let restored = restore_candidate(&request, &passphrase).unwrap();
    let replayed = restore_candidate(&request, &passphrase).unwrap();
    assert_eq!(replayed, restored);
    assert_eq!(restored.status, RestoreStatus::DurablyFencedCandidate);
    assert!(!candidate.with_extension("db-wal").exists());
    assert!(!candidate.with_extension("db-shm").exists());
    let conn = rusqlite::Connection::open_with_flags(
        &candidate,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert!(matches!(
        startup_mode(&conn).unwrap(),
        RecoveryStartupMode::FencedCandidate { .. }
    ));
    assert_eq!(
        iotkit_core_ops::ownership_state(&conn).unwrap(),
        iotkit_core_ops::OwnershipState::LocalRecoveryRequired
    );
    let audit: String = conn
        .query_row(
            "SELECT detail FROM ledger_events WHERE kind='r14_op' ORDER BY event_id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    for protected in [
        "recovery-fixture",
        "backup-restore-fixture",
        "node-test",
        "epoch-test",
        "epoch-restored",
    ] {
        assert!(
            !audit.contains(protected),
            "restore audit leaked {protected}: {audit}"
        );
    }
    assert!(audit.contains("private_recovery_state"));
    assert!(audit.contains("[REDACTED]"));
    assert!(audit.contains("\"targets\":[]"));
    assert!(!audit.contains(&snapshot_artifact.manifest.database_sha256));
    assert!(!audit.contains(&snapshot_artifact.manifest.database_length.to_string()));
}

#[cfg(target_os = "linux")]
#[test]
fn restore_fault_matrix_never_publishes_an_unfenced_name() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    for fault in [
        RestoreFault::Decrypted,
        RestoreFault::Copied,
        RestoreFault::FenceCommitted,
        RestoreFault::Checkpointed,
        RestoreFault::CandidateFileSynced,
        RestoreFault::RenameSucceeded,
        RestoreFault::ParentSynced,
        RestoreFault::PublishedReadbackVerified,
    ] {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let live = root.path().join("live.db");
        let candidate = root.path().join("candidate.db");
        let snapshot = root.path().join("snapshot.db");
        let artifact = root.path().join("backup.iotkit-node-backup");
        let staging = tempfile::tempdir_in("/dev/shm").unwrap();
        fs::set_permissions(staging.path(), fs::Permissions::from_mode(0o700)).unwrap();

        let source = tests_support::active_database_with_publications(&live, 0, 1);
        drop(source);
        let backup_id = "backup-restore-fault";
        let snapshot_artifact =
            create_consistent_snapshot(&live, &snapshot, backup_id, 1_725_000_000_000).unwrap();
        let passphrase = BackupPassphrase::new("owner-only-test-passphrase".into());
        let output = DirectoryCapability::open(root.path()).unwrap();
        encrypt_container(
            &snapshot_artifact.path,
            &snapshot_artifact.manifest,
            &passphrase,
            &output,
            "backup.iotkit-node-backup",
        )
        .unwrap();
        let request = RestoreRequest {
            input: artifact,
            live_database: live,
            candidate_database: candidate.clone(),
            staging_directory: staging.path().to_path_buf(),
            handoff: RecoveryHandoff {
                schema_version: 1,
                recovery_id: "recovery-fault".into(),
                edge_id: "edge-test".into(),
                edge_node_id: tests_support::TEST_EDGE_NODE_ID.into(),
                old_ledger_epoch: tests_support::TEST_LEDGER_EPOCH.into(),
                expected_backup_id: Some(backup_id.into()),
                proposed_new_epoch: "epoch-restored".into(),
                credential_generation: 2,
            },
        };
        let result = restore_with_fault(&request, &passphrase, fault);
        assert!(matches!(
            result,
            Err(RecoveryError::Storage | RecoveryError::CandidatePublicationUncertain)
        ));
        if candidate.exists() {
            let conn = rusqlite::Connection::open_with_flags(
                &candidate,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            assert!(matches!(
                startup_mode(&conn).unwrap(),
                RecoveryStartupMode::FencedCandidate { .. }
            ));
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn restore_process_abort_matrix_retries_to_one_fenced_candidate_without_live_change() {
    use std::{os::unix::process::ExitStatusExt, process::Command};

    for phase in [
        "decrypted",
        "copied",
        "fence_committed",
        "checkpointed",
        "candidate_file_synced",
        "rename_succeeded",
        "parent_synced",
        "published_readback_verified",
    ] {
        let fixture = restore_fixture();
        let live_before = std::fs::read(&fixture.request.live_database).unwrap();
        let root = fixture.request.input.parent().unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "restore_tests::restore_abort_child",
                "--nocapture",
            ])
            .env("IOTKIT_RECOVERY_TEST_RESTORE_ROOT", root)
            .env(
                "IOTKIT_RECOVERY_TEST_RESTORE_STAGING",
                &fixture.request.staging_directory,
            )
            .env("IOTKIT_RECOVERY_TEST_RESTORE_PHASE", phase)
            .status()
            .unwrap();
        assert_eq!(
            child.signal(),
            Some(libc::SIGABRT),
            "{phase} child did not abort at its requested hook"
        );

        let published = fixture.request.candidate_database.exists();
        if [
            "rename_succeeded",
            "parent_synced",
            "published_readback_verified",
        ]
        .contains(&phase)
        {
            assert!(published, "{phase} must retain its published candidate");
            let conn = rusqlite::Connection::open_with_flags(
                &fixture.request.candidate_database,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            assert!(matches!(
                startup_mode(&conn).unwrap(),
                RecoveryStartupMode::FencedCandidate { .. }
            ));
        } else {
            assert!(!published, "{phase} published before its rename boundary");
        }

        let receipt = restore_candidate(&fixture.request, &restore_passphrase())
            .unwrap_or_else(|error| panic!("{phase} retry failed: {error:?}"));
        assert_eq!(receipt.status, RestoreStatus::DurablyFencedCandidate);
        assert!(fixture.request.candidate_database.exists());
        assert!(
            !fixture
                .request
                .candidate_database
                .with_extension("db-wal")
                .exists()
        );
        assert!(
            !fixture
                .request
                .candidate_database
                .with_extension("db-shm")
                .exists()
        );
        assert_eq!(
            std::fs::read(&fixture.request.live_database).unwrap(),
            live_before,
            "{phase} changed the live database"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn restore_abort_child() {
    let (Some(root), Some(staging), Some(phase)) = (
        std::env::var_os("IOTKIT_RECOVERY_TEST_RESTORE_ROOT"),
        std::env::var_os("IOTKIT_RECOVERY_TEST_RESTORE_STAGING"),
        std::env::var_os("IOTKIT_RECOVERY_TEST_RESTORE_PHASE"),
    ) else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let request = RestoreRequest {
        input: root.join("backup.iotkit-node-backup"),
        live_database: root.join("live.db"),
        candidate_database: root.join("candidate.db"),
        staging_directory: std::path::PathBuf::from(staging),
        handoff: RecoveryHandoff {
            schema_version: 1,
            recovery_id: "recovery-negative".into(),
            edge_id: "edge-test".into(),
            edge_node_id: tests_support::TEST_EDGE_NODE_ID.into(),
            old_ledger_epoch: tests_support::TEST_LEDGER_EPOCH.into(),
            expected_backup_id: Some("backup-restore-negative".into()),
            proposed_new_epoch: "epoch-restored".into(),
            credential_generation: 2,
        },
    };
    let phase = phase.to_string_lossy().to_string();
    let hook = |current: crate::restore::RestorePhase, _published: bool| {
        let current = match current {
            crate::restore::RestorePhase::Decrypted => "decrypted",
            crate::restore::RestorePhase::Copied => "copied",
            crate::restore::RestorePhase::FenceCommitted => "fence_committed",
            crate::restore::RestorePhase::Checkpointed => "checkpointed",
            crate::restore::RestorePhase::CandidateFileSynced => "candidate_file_synced",
            crate::restore::RestorePhase::RenameSucceeded => "rename_succeeded",
            crate::restore::RestorePhase::ParentSynced => "parent_synced",
            crate::restore::RestorePhase::PublishedReadbackVerified => {
                "published_readback_verified"
            }
        };
        if current == phase {
            std::process::abort();
        }
        Ok(())
    };
    let _ = crate::restore::restore_candidate_inner(&request, &restore_passphrase(), Some(&hook));
}

#[cfg(target_os = "linux")]
struct RestoreFixture {
    _root: tempfile::TempDir,
    _staging: tempfile::TempDir,
    request: RestoreRequest,
}

#[cfg(target_os = "linux")]
fn restore_fixture() -> RestoreFixture {
    restore_fixture_with_schema(24)
}

#[cfg(target_os = "linux")]
fn restore_fixture_with_schema(schema_version: u32) -> RestoreFixture {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let live = root.path().join("live.db");
    let candidate = root.path().join("candidate.db");
    let snapshot = root.path().join("snapshot.db");
    let artifact = root.path().join("backup.iotkit-node-backup");
    let staging = tempfile::tempdir_in("/dev/shm").unwrap();
    fs::set_permissions(staging.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let staging_path = staging.path().to_path_buf();

    let source = tests_support::active_database_with_publications(&live, 0, 1);
    if schema_version == 23 {
        source
            .execute_batch(
                "DROP TRIGGER edge_node_recovery_activation_immutable_delete;
                 DROP TRIGGER edge_node_recovery_activation_insert_state;
                 DROP TRIGGER edge_node_recovery_activation_forward_only;
                 DROP TABLE edge_node_recovery_activation;
                 DELETE FROM _schema_version WHERE version=24;",
            )
            .unwrap();
    } else {
        assert_eq!(schema_version, 24);
    }
    drop(source);
    let backup_id = "backup-restore-negative";
    let snapshot_artifact =
        create_consistent_snapshot(&live, &snapshot, backup_id, 1_725_000_000_000).unwrap();
    let passphrase = BackupPassphrase::new("owner-only-test-passphrase".into());
    let output = DirectoryCapability::open(root.path()).unwrap();
    encrypt_container(
        &snapshot_artifact.path,
        &snapshot_artifact.manifest,
        &passphrase,
        &output,
        "backup.iotkit-node-backup",
    )
    .unwrap();

    RestoreFixture {
        _root: root,
        _staging: staging,
        request: RestoreRequest {
            input: artifact,
            live_database: live,
            candidate_database: candidate,
            staging_directory: staging_path,
            handoff: RecoveryHandoff {
                schema_version: 1,
                recovery_id: "recovery-negative".into(),
                edge_id: "edge-test".into(),
                edge_node_id: tests_support::TEST_EDGE_NODE_ID.into(),
                old_ledger_epoch: tests_support::TEST_LEDGER_EPOCH.into(),
                expected_backup_id: Some(backup_id.into()),
                proposed_new_epoch: "epoch-restored".into(),
                credential_generation: 2,
            },
        },
    }
}

#[cfg(target_os = "linux")]
#[test]
fn v23_encrypted_backup_restores_and_migrates_the_fenced_candidate_to_v24() {
    let fixture = restore_fixture_with_schema(23);
    let receipt = restore_candidate(&fixture.request, &restore_passphrase()).unwrap();
    assert_eq!(receipt.schema_version, 2);

    let candidate = rusqlite::Connection::open(&fixture.request.candidate_database).unwrap();
    assert_eq!(
        candidate
            .query_row(
                "SELECT count(*) FROM _schema_version WHERE version=24",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert!(matches!(
        startup_mode(&candidate).unwrap(),
        RecoveryStartupMode::FencedCandidate { .. }
    ));
}

#[cfg(target_os = "linux")]
fn restore_passphrase() -> BackupPassphrase {
    BackupPassphrase::new("owner-only-test-passphrase".into())
}

#[cfg(target_os = "linux")]
fn clone_restore_request(request: &RestoreRequest) -> RestoreRequest {
    RestoreRequest {
        input: request.input.clone(),
        live_database: request.live_database.clone(),
        candidate_database: request.candidate_database.clone(),
        staging_directory: request.staging_directory.clone(),
        handoff: request.handoff.clone(),
    }
}

#[cfg(target_os = "linux")]
#[test]
fn restore_rejects_identity_conflicts_existing_targets_and_corrupt_artifacts() {
    use std::fs;
    use std::os::unix::fs::symlink;

    for (field, expected) in [
        ("node", RecoveryError::HandoffMismatch),
        ("epoch", RecoveryError::HandoffMismatch),
        ("edge", RecoveryError::HandoffMismatch),
        ("backup", RecoveryError::HandoffMismatch),
    ] {
        let mut fixture = restore_fixture();
        match field {
            "node" => fixture.request.handoff.edge_node_id = "wrong-node".into(),
            "epoch" => fixture.request.handoff.old_ledger_epoch = "wrong-epoch".into(),
            "edge" => fixture.request.handoff.edge_id = "wrong-edge".into(),
            "backup" => fixture.request.handoff.expected_backup_id = Some("wrong-backup".into()),
            _ => unreachable!(),
        }
        let result = restore_candidate(&fixture.request, &restore_passphrase());
        assert_eq!(result.unwrap_err(), expected, "identity field {field}");
        assert!(!fixture.request.candidate_database.exists());
    }

    let mut fixture = restore_fixture();
    let same = fixture.request.live_database.clone();
    fixture.request.candidate_database = same;
    assert_eq!(
        restore_candidate(&fixture.request, &restore_passphrase()),
        Err(RecoveryError::InvalidConfiguration)
    );
    let mut absent = restore_fixture();
    absent.request.live_database = absent
        .request
        .candidate_database
        .parent()
        .unwrap()
        .join("absent.db");
    absent.request.candidate_database = absent.request.live_database.clone();
    assert_eq!(
        restore_candidate(&absent.request, &restore_passphrase()),
        Err(RecoveryError::InvalidConfiguration)
    );

    let symlink_fixture = restore_fixture();
    symlink(
        &symlink_fixture.request.live_database,
        &symlink_fixture.request.candidate_database,
    )
    .unwrap();
    assert_eq!(
        restore_candidate(&symlink_fixture.request, &restore_passphrase()),
        Err(RecoveryError::CandidateConflict)
    );

    let hardlink_fixture = restore_fixture();
    fs::hard_link(
        &hardlink_fixture.request.live_database,
        &hardlink_fixture.request.candidate_database,
    )
    .unwrap();
    assert_eq!(
        restore_candidate(&hardlink_fixture.request, &restore_passphrase()),
        Err(RecoveryError::CandidateConflict)
    );

    let corrupt_fixture = restore_fixture();
    let mut bytes = fs::read(&corrupt_fixture.request.input).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    fs::write(&corrupt_fixture.request.input, bytes).unwrap();
    assert!(matches!(
        restore_candidate(&corrupt_fixture.request, &restore_passphrase()),
        Err(RecoveryError::AuthenticationFailed | RecoveryError::ContainerInvalid)
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn restore_rejects_active_and_mismatched_fenced_candidates_and_serializes_concurrency() {
    use std::{fs, sync::Arc, sync::Barrier, thread};

    let active = restore_fixture();
    fs::copy(
        &active.request.live_database,
        &active.request.candidate_database,
    )
    .unwrap();
    assert!(matches!(
        restore_candidate(&active.request, &restore_passphrase()),
        Err(RecoveryError::CandidateConflict | RecoveryError::CandidateFenceInvalid)
    ));

    let replay = restore_fixture();
    let receipt = restore_candidate(&replay.request, &restore_passphrase()).unwrap();
    for mismatch in [
        "recovery",
        "proposed_epoch",
        "expected_backup",
        "edge",
        "node",
        "old_epoch",
        "schema_version",
    ] {
        let mut request = clone_restore_request(&replay.request);
        match mismatch {
            "recovery" => request.handoff.recovery_id = "different-recovery".into(),
            "proposed_epoch" => request.handoff.proposed_new_epoch = "different-epoch".into(),
            "expected_backup" => request.handoff.expected_backup_id = None,
            "edge" => request.handoff.edge_id = "different-edge".into(),
            "node" => request.handoff.edge_node_id = "different-node".into(),
            "old_epoch" => request.handoff.old_ledger_epoch = "different-old-epoch".into(),
            "schema_version" => request.handoff.schema_version = 2,
            _ => unreachable!(),
        }
        assert_eq!(
            restore_candidate(&request, &restore_passphrase()),
            Err(RecoveryError::CandidateConflict),
            "non-exact replay handoff {mismatch}"
        );
    }
    assert!(receipt.candidate_instance_id.starts_with("candidate-"));

    let concurrent = restore_fixture();
    let barrier = Arc::new(Barrier::new(2));
    let left_request = clone_restore_request(&concurrent.request);
    let right_request = clone_restore_request(&concurrent.request);
    let left_barrier = Arc::clone(&barrier);
    let right_barrier = Arc::clone(&barrier);
    let left = thread::spawn(move || {
        left_barrier.wait();
        restore_candidate(&left_request, &restore_passphrase())
    });
    let right = thread::spawn(move || {
        right_barrier.wait();
        restore_candidate(&right_request, &restore_passphrase())
    });
    let results = [left.join().unwrap(), right.join().unwrap()];
    assert!(results.iter().any(Result::is_ok));
    assert!(
        results.iter().all(|result| {
            result.is_ok() || matches!(result, Err(RecoveryError::OperationBusy))
        })
    );
}

#[cfg(target_os = "linux")]
#[test]
fn replay_revalidates_configured_live_identity_and_published_authority() {
    use std::fs;

    for mismatch in ["node", "epoch", "edge"] {
        let fixture = restore_fixture();
        let passphrase = restore_passphrase();
        restore_candidate(&fixture.request, &passphrase).unwrap();
        let conn = rusqlite::Connection::open(&fixture.request.live_database).unwrap();
        match mismatch {
            "node" => conn
                .execute(
                    "UPDATE ledger_meta SET value='different-live-node'
                     WHERE key='edge_node_id'",
                    [],
                )
                .unwrap(),
            "epoch" => conn
                .execute(
                    "UPDATE ledger_meta SET value='different-live-epoch'
                     WHERE key='epoch'",
                    [],
                )
                .unwrap(),
            "edge" => conn
                .execute(
                    "UPDATE edge_node_activation SET edge_id='different-live-edge'
                     WHERE singleton=1",
                    [],
                )
                .unwrap(),
            _ => unreachable!(),
        };
        drop(conn);
        assert_eq!(
            restore_candidate(&fixture.request, &passphrase),
            Err(RecoveryError::HandoffMismatch),
            "configured live identity mismatch {mismatch}"
        );
    }

    let authority = restore_fixture();
    let passphrase = restore_passphrase();
    restore_candidate(&authority.request, &passphrase).unwrap();
    let conn = rusqlite::Connection::open(&authority.request.candidate_database).unwrap();
    conn.execute(
        "UPDATE auth_state SET recovery_required=0, ownership_ever_established=0 WHERE id=1",
        [],
    )
    .unwrap();
    drop(conn);
    assert_eq!(
        restore_candidate(&authority.request, &passphrase),
        Err(RecoveryError::CandidateConflict)
    );

    let sidecar = restore_fixture();
    restore_candidate(&sidecar.request, &passphrase).unwrap();
    fs::write(
        sidecar.request.candidate_database.with_extension("db-wal"),
        b"sidecar",
    )
    .unwrap();
    assert_eq!(
        restore_candidate(&sidecar.request, &passphrase),
        Err(RecoveryError::CandidateConflict)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn replay_rejects_same_ids_with_different_authenticated_database_content() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let fixture = restore_fixture();
    let passphrase = restore_passphrase();
    restore_candidate(&fixture.request, &passphrase).unwrap();
    let candidate_conn = rusqlite::Connection::open_with_flags(
        &fixture.request.candidate_database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let before: (i64, String) = candidate_conn
        .query_row(
            "SELECT source_database_length, source_database_sha256
             FROM edge_node_recovery_candidate WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    drop(candidate_conn);

    let second_root = tempfile::tempdir().unwrap();
    fs::set_permissions(second_root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let second_live = second_root.path().join("second-live.db");
    let second_snapshot = second_root.path().join("second-snapshot.db");
    let source = tests_support::active_database_with_publications(&second_live, 0, 2);
    drop(source);
    let second_artifact = create_consistent_snapshot(
        &second_live,
        &second_snapshot,
        fixture
            .request
            .handoff
            .expected_backup_id
            .as_deref()
            .unwrap(),
        1_725_000_000_000,
    )
    .unwrap();
    let output = DirectoryCapability::open(second_root.path()).unwrap();
    encrypt_container(
        &second_artifact.path,
        &second_artifact.manifest,
        &passphrase,
        &output,
        "second-backup.iotkit-node-backup",
    )
    .unwrap();
    fs::copy(
        second_root.path().join("second-backup.iotkit-node-backup"),
        &fixture.request.input,
    )
    .unwrap();

    assert_eq!(
        restore_candidate(&fixture.request, &passphrase),
        Err(RecoveryError::CandidateConflict)
    );
    let candidate_conn = rusqlite::Connection::open_with_flags(
        &fixture.request.candidate_database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let after: (i64, String) = candidate_conn
        .query_row(
            "SELECT source_database_length, source_database_sha256
             FROM edge_node_recovery_candidate WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(after, before);
}

#[cfg(target_os = "linux")]
#[test]
fn replay_rejects_same_snapshot_manifest_with_different_encrypted_bytes() {
    use std::fs;

    let fixture = restore_fixture();
    let passphrase = restore_passphrase();
    restore_candidate(&fixture.request, &passphrase).unwrap();
    let before = fs::read(&fixture.request.candidate_database).unwrap();

    let manifest = authenticate_container(&fixture.request.input, &passphrase).unwrap();
    let second_name = "second-encrypted.iotkit-node-backup";
    let parent = fixture.request.input.parent().unwrap();
    let output = DirectoryCapability::open(parent).unwrap();
    encrypt_container(
        &parent.join("snapshot.db"),
        &manifest,
        &passphrase,
        &output,
        second_name,
    )
    .unwrap();
    fs::copy(parent.join(second_name), &fixture.request.input).unwrap();

    assert_eq!(
        restore_candidate(&fixture.request, &passphrase),
        Err(RecoveryError::CandidateConflict)
    );
    assert_eq!(
        fs::read(&fixture.request.candidate_database).unwrap(),
        before
    );
}

#[cfg(target_os = "linux")]
#[test]
fn restore_rejects_stale_candidate_sidecars_before_publish_and_after_rename_is_uncertain() {
    use std::{fs, path::PathBuf};

    for suffix in ["-wal", "-shm", "-journal"] {
        let fixture = restore_fixture();
        let sidecar = PathBuf::from(format!(
            "{}{}",
            fixture.request.candidate_database.display(),
            suffix
        ));
        fs::write(&sidecar, b"stale").unwrap();
        assert_eq!(
            restore_candidate(&fixture.request, &restore_passphrase()),
            Err(RecoveryError::CandidateConflict),
            "stale candidate sidecar {suffix}"
        );
        assert!(!fixture.request.candidate_database.exists());
    }

    for suffix in ["-wal", "-shm", "-journal"] {
        let fixture = restore_fixture();
        let sidecar = PathBuf::from(format!(
            "{}{}",
            fixture.request.candidate_database.display(),
            suffix
        ));
        let hook = |phase: crate::restore::RestorePhase, _published: bool| {
            if phase == crate::restore::RestorePhase::RenameSucceeded {
                fs::write(&sidecar, b"stale-after-rename").unwrap();
            }
            Ok(())
        };
        assert_eq!(
            crate::restore::restore_candidate_inner(
                &fixture.request,
                &restore_passphrase(),
                Some(&hook),
            ),
            Err(RecoveryError::CandidatePublicationUncertain),
            "post-rename candidate sidecar {suffix}"
        );
        assert!(fixture.request.candidate_database.exists());
        fs::remove_file(&sidecar).unwrap();
        assert!(restore_candidate(&fixture.request, &restore_passphrase()).is_ok());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn restore_keeps_unpublished_temp_bound_to_the_held_candidate_parent() {
    use std::{fs, os::unix::fs::PermissionsExt};

    let fixture = restore_fixture();
    let parent = fixture
        .request
        .candidate_database
        .parent()
        .unwrap()
        .to_path_buf();
    let moved_parent = parent.with_file_name(format!(
        "{}-moved",
        parent.file_name().unwrap().to_string_lossy()
    ));
    let replacement_parent = parent.clone();
    let hook = |phase: crate::restore::RestorePhase, _published: bool| {
        if phase == crate::restore::RestorePhase::Copied {
            fs::rename(&replacement_parent, &moved_parent).unwrap();
            fs::create_dir(&replacement_parent).unwrap();
            fs::set_permissions(&replacement_parent, fs::Permissions::from_mode(0o700)).unwrap();
        }
        Ok(())
    };

    let result = crate::restore::restore_candidate_inner(
        &fixture.request,
        &restore_passphrase(),
        Some(&hook),
    );
    assert!(result.is_ok(), "held parent restore failed: {result:?}");
    assert!(moved_parent.join("candidate.db").exists());
    assert!(!replacement_parent.join("candidate.db").exists());
    fs::remove_dir_all(moved_parent).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn replay_rejects_direct_candidate_provenance_row_mismatch_without_replacement() {
    use std::fs;

    for mismatch in [
        "source_database_length",
        "source_database_sha256",
        "artifact_length",
        "artifact_sha256",
    ] {
        let fixture = restore_fixture();
        let passphrase = restore_passphrase();
        restore_candidate(&fixture.request, &passphrase).unwrap();
        let conn = rusqlite::Connection::open(&fixture.request.candidate_database).unwrap();
        let before: (i64, String, i64, String) = conn
            .query_row(
                "SELECT source_database_length, source_database_sha256,
                        artifact_length, artifact_sha256
                 FROM edge_node_recovery_candidate WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        let immutable_trigger: String = conn
            .query_row(
                "SELECT sql FROM sqlite_schema
                 WHERE type='trigger' AND name='edge_node_recovery_candidate_immutable'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute_batch("DROP TRIGGER edge_node_recovery_candidate_immutable")
            .unwrap();
        let update = match mismatch {
            "source_database_length" => {
                "UPDATE edge_node_recovery_candidate
                 SET source_database_length = source_database_length + 1
                 WHERE singleton=1"
            }
            "source_database_sha256" => {
                "UPDATE edge_node_recovery_candidate
                 SET source_database_sha256 = 'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'
                 WHERE singleton=1"
            }
            "artifact_length" => {
                "UPDATE edge_node_recovery_candidate
                 SET artifact_length = artifact_length + 1
                 WHERE singleton=1"
            }
            "artifact_sha256" => {
                "UPDATE edge_node_recovery_candidate
                 SET artifact_sha256 = 'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'
                 WHERE singleton=1"
            }
            _ => unreachable!(),
        };
        conn.execute(update, []).unwrap();
        conn.execute_batch(&immutable_trigger).unwrap();
        drop(conn);

        assert_eq!(
            restore_candidate(&fixture.request, &passphrase),
            Err(RecoveryError::CandidateConflict),
            "direct provenance mismatch {mismatch}"
        );
        let conn = rusqlite::Connection::open_with_flags(
            &fixture.request.candidate_database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let after: (i64, String, i64, String) = conn
            .query_row(
                "SELECT source_database_length, source_database_sha256,
                        artifact_length, artifact_sha256
                 FROM edge_node_recovery_candidate WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_ne!(after, before);
        assert!(fs::metadata(&fixture.request.candidate_database).is_ok());
    }
}
