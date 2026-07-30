use super::*;

#[test]
fn recovery_debug_output_redacts_every_identifier_and_path() {
    let handoff = RecoveryHandoff {
        schema_version: 1,
        recovery_id: "recovery-secret".into(),
        edge_id: "edge-secret".into(),
        edge_node_id: "node-secret".into(),
        old_ledger_epoch: "epoch-old-secret".into(),
        expected_backup_id: Some("backup-secret".into()),
        proposed_new_epoch: "epoch-new-secret".into(),
        credential_generation: 7,
    };
    let manifest = NodeBackupManifest {
        artifact_kind: "iotkit-node-backup".into(),
        format_version: 1,
        backup_id: "backup-secret".into(),
        edge_node_id: "node-secret".into(),
        ledger_epoch: "epoch-old-secret".into(),
        created_at_ms: 99,
        accepted_cursor: 11,
        allocation_high_water: 12,
        snapshot_mode: SnapshotMode::Online,
        shutdown_seal_id: Some("seal-secret".into()),
        schema_version: 1,
        database_length: 13,
        database_sha256: "digest-secret".into(),
        counts: BackupCounts::default(),
    };
    let config = BackupConfig {
        schema_version: 1,
        database: "C:/secret/database.db".into(),
        destination: "C:/secret/destination".into(),
        staging_directory: "C:/secret/staging".into(),
        passphrase_file: "C:/secret/passphrase".into(),
        expected_mount: MountIdentity {
            mount_point: "C:/secret/mount".into(),
            source: "source-secret".into(),
            filesystem_type: "fs-secret".into(),
            filesystem_id: "fs-id-secret".into(),
        },
        freshness_seconds: 60,
        retention_count: 2,
    };

    let passphrase = BackupPassphrase::new("passphrase-secret".into());
    assert!(!passphrase.is_empty());
    let output = format!("{handoff:?} {manifest:?} {config:?} {passphrase:?}");
    for secret in [
        "recovery-secret",
        "edge-secret",
        "node-secret",
        "epoch-old-secret",
        "backup-secret",
        "epoch-new-secret",
        "seal-secret",
        "digest-secret",
        "C:/secret",
        "source-secret",
        "fs-secret",
        "fs-id-secret",
        "7",
        "11",
        "12",
        "passphrase-secret",
    ] {
        assert!(!output.contains(secret), "debug leaked {secret}: {output}");
    }
    assert!(output.contains("iotkit-node-backup"));
    assert!(output.contains("Online"));
}

#[test]
fn remaining_recovery_debug_implementations_expose_only_safe_labels() {
    let artifact = BackupStatusArtifact {
        backup_id: "backup-secret".into(),
        edge_node_id: "node-secret".into(),
        ledger_epoch: "epoch-secret".into(),
        created_at_ms: 1,
        artifact_length: 2,
        accepted_cursor: 3,
        allocation_high_water: 4,
    };
    let startup = RecoveryStartupMode::FencedCandidate {
        recovery_id: "recovery-secret".into(),
        candidate_instance_id: "candidate-secret".into(),
        backup_id: Some("backup-secret".into()),
        edge_id: "edge-secret".into(),
        old_ledger_epoch: "epoch-secret".into(),
        proposed_new_epoch: "new-epoch-secret".into(),
        credential_generation: 5,
    };
    let readiness = BackupReadiness::Failed {
        reason_code: "reason-secret".into(),
        observed_at_ms: 6,
        last_verified: Some(artifact.clone()),
    };
    let receipt = RestoreReceipt {
        schema_version: 1,
        status: RestoreStatus::DurablyFencedCandidate,
        recovery_id: "recovery-secret".into(),
        candidate_instance_id: "candidate-secret".into(),
        backup_id: "backup-secret".into(),
        edge_id: "edge-secret".into(),
        edge_node_id: "node-secret".into(),
        old_ledger_epoch: "epoch-secret".into(),
        proposed_new_epoch: "new-epoch-secret".into(),
        credential_generation: 5,
    };
    let mount = MountIdentity {
        mount_point: "C:/secret/mount".into(),
        source: "source-secret".into(),
        filesystem_type: "filesystem-secret".into(),
        filesystem_id: "filesystem-id-secret".into(),
    };

    let output = format!("{artifact:?} {startup:?} {readiness:?} {receipt:?} {mount:?}");
    for secret in [
        "backup-secret",
        "node-secret",
        "epoch-secret",
        "recovery-secret",
        "candidate-secret",
        "edge-secret",
        "new-epoch-secret",
        "reason-secret",
        "C:/secret",
        "source-secret",
        "filesystem-secret",
        "filesystem-id-secret",
        "5",
    ] {
        assert!(!output.contains(secret), "debug leaked {secret}: {output}");
    }
    assert!(output.contains("FencedCandidate"));
    assert!(output.contains("Failed"));
    assert!(output.contains("DurablyFencedCandidate"));
}

#[test]
fn restore_request_debug_redacts_every_path_and_handoff_field() {
    let request = RestoreRequest {
        input: "C:/secret/input.iotkit-node-backup".into(),
        live_database: "C:/secret/live.db".into(),
        candidate_database: "C:/secret/candidate.db".into(),
        staging_directory: "C:/secret/staging".into(),
        handoff: RecoveryHandoff {
            schema_version: 1,
            recovery_id: "recovery-secret".into(),
            edge_id: "edge-secret".into(),
            edge_node_id: "node-secret".into(),
            old_ledger_epoch: "old-epoch-secret".into(),
            expected_backup_id: Some("backup-secret".into()),
            proposed_new_epoch: "new-epoch-secret".into(),
            credential_generation: 9,
        },
    };

    let output = format!("{request:?}");
    for secret in [
        "C:/secret",
        "recovery-secret",
        "edge-secret",
        "node-secret",
        "old-epoch-secret",
        "backup-secret",
        "new-epoch-secret",
        "9",
    ] {
        assert!(!output.contains(secret), "debug leaked {secret}: {output}");
    }
    assert_eq!(output, "RestoreRequest");
}
