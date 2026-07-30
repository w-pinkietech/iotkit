use super::*;
use iotkit_core_ops::{Actor, ActorKind, DispatchRequest, Tier, dispatch};
use serde_json::json;

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
enum TestBackupFault {
    AfterBegin,
    AfterPublication,
    AfterReadback,
    BeforeReceipt,
    AfterReceipt,
    ReplaceSourceBeforeSnapshot,
    ReconciliationParentSync,
}

#[cfg(target_os = "linux")]
struct TestBackupHook(TestBackupFault);

#[cfg(target_os = "linux")]
impl crate::backup::BackupHook for TestBackupHook {
    fn at(
        &self,
        point: crate::backup::BackupHookPoint,
        config: &BackupConfig,
    ) -> Result<(), RecoveryError> {
        use crate::backup::BackupHookPoint;
        let selected = match self.0 {
            TestBackupFault::AfterBegin => BackupHookPoint::AfterBegin,
            TestBackupFault::AfterPublication => BackupHookPoint::AfterPublication,
            TestBackupFault::AfterReadback => BackupHookPoint::AfterReadback,
            TestBackupFault::BeforeReceipt => BackupHookPoint::BeforeReceipt,
            TestBackupFault::AfterReceipt => BackupHookPoint::AfterReceipt,
            TestBackupFault::ReplaceSourceBeforeSnapshot => BackupHookPoint::BeforeSnapshot,
            TestBackupFault::ReconciliationParentSync => {
                BackupHookPoint::BeforeReconciliationParentSync
            }
        };
        if point != selected {
            return Ok(());
        }
        match self.0 {
            TestBackupFault::ReplaceSourceBeforeSnapshot => {
                std::fs::rename(
                    &config.database,
                    config.database.with_extension("held-original"),
                )
                .map_err(|_| RecoveryError::Storage)?;
                std::fs::rename(
                    config.database.with_extension("replacement"),
                    &config.database,
                )
                .map_err(|_| RecoveryError::Storage)
            }
            TestBackupFault::AfterBegin | TestBackupFault::AfterReceipt => {
                Err(RecoveryError::Storage)
            }
            TestBackupFault::AfterPublication
            | TestBackupFault::AfterReadback
            | TestBackupFault::BeforeReceipt
            | TestBackupFault::ReconciliationParentSync => {
                Err(RecoveryError::ArtifactPublicationUncertain)
            }
        }
    }
}

#[cfg(target_os = "linux")]
struct SnapshotCapacityFailureHook {
    requested: std::cell::Cell<Option<u64>>,
}

#[cfg(target_os = "linux")]
impl crate::backup::BackupHook for SnapshotCapacityFailureHook {
    fn check_destination_capacity(
        &self,
        _destination: &VerifiedBackupDestination,
        bytes: u64,
    ) -> Result<(), RecoveryError> {
        self.requested.set(Some(bytes));
        Err(RecoveryError::StorageFull)
    }
}

#[cfg(target_os = "linux")]
fn create_backup_with_fault(
    config_path: &std::path::Path,
    passphrase: &BackupPassphrase,
    now_ms: i64,
    fault: TestBackupFault,
) -> Result<NodeBackupManifest, RecoveryError> {
    crate::backup::create_backup_with_hook(config_path, passphrase, now_ms, &TestBackupHook(fault))
}

#[test]
fn operation_busy_status_debug_is_redacted() {
    assert_eq!(
        format!("{:?}", BackupReadiness::OperationBusy),
        "BackupReadiness::OperationBusy"
    );
}

#[cfg(not(target_os = "linux"))]
#[test]
fn configured_status_and_creation_fail_closed_without_filesystem_effects() {
    let root = tempfile::tempdir().unwrap();
    let config_path = root.path().join("backup.json");
    std::fs::write(&config_path, b"{}").unwrap();
    assert_eq!(
        backup_status(&config_path, 1),
        Err(RecoveryError::PlatformUnsupported)
    );
    let config = BackupConfig {
        schema_version: 1,
        database: root.path().join("edge.db"),
        destination: root.path().join("destination"),
        staging_directory: root.path().join("staging"),
        passphrase_file: root.path().join("passphrase"),
        expected_mount: MountIdentity {
            mount_point: root.path().to_path_buf(),
            source: "source".into(),
            filesystem_type: "test".into(),
            filesystem_id: "fsid".into(),
        },
        freshness_seconds: 60,
        retention_count: 1,
    };
    assert_eq!(
        create_backup(
            &config_path,
            &BackupPassphrase::new("owner-only-passphrase".into()),
            1
        ),
        Err(RecoveryError::PlatformUnsupported)
    );
    assert!(!config.database.exists());
    assert!(!config.destination.exists());
    assert!(!config.staging_directory.exists());
}

fn dispatch_private(conn: &rusqlite::Connection, op: &str, state: serde_json::Value) {
    dispatch(
        conn,
        recovery_descriptors(),
        DispatchRequest {
            op: op.into(),
            params: json!({"private_recovery_state": state}),
            dry_run: false,
            actor: Actor {
                actor_id: "backup-test".into(),
                actor_kind: ActorKind::LocalCli,
                tier_ceiling: Tier::Construction,
            },
            source: None,
            step_up_verified: true,
            clock_trust: None,
        },
    )
    .unwrap();
}

#[test]
fn exact_terminal_completion_replay_is_idempotent_and_audit_stays_redacted() {
    let conn = crate::tests_support::complete_database();
    let begin = json!({
        "attempt_id": "attempt-sensitive-id",
        "backup_id": "backup-sensitive-id",
        "artifact_name": "backup-sensitive-id.iotkit-node-backup",
        "edge_node_id": "edge-node-sensitive-id",
        "started_at_ms": 10
    });
    let complete = json!({
        "attempt_id": "attempt-sensitive-id",
        "outcome": "success",
        "reason_code": "ok",
        "artifact_length": 42,
        "ledger_epoch": "ledger-sensitive-epoch",
        "accepted_cursor": 4,
        "allocation_high_water": 7,
        "artifact_created_at_ms": 10,
        "completed_at_ms": 11
    });
    dispatch_private(&conn, BEGIN_BACKUP_ATTEMPT_OP, begin.clone());
    dispatch_private(&conn, BEGIN_BACKUP_ATTEMPT_OP, begin);
    dispatch_private(&conn, COMPLETE_BACKUP_ATTEMPT_OP, complete.clone());
    dispatch_private(&conn, COMPLETE_BACKUP_ATTEMPT_OP, complete);

    assert_eq!(
        conn.query_row("SELECT state FROM edge_node_backup_attempts", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "success"
    );
    let audit = conn
        .prepare("SELECT detail FROM ledger_events WHERE kind='r14_op'")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    for protected in [
        "attempt-sensitive-id",
        "backup-sensitive-id",
        "edge-node-sensitive-id",
        "ledger-sensitive-epoch",
    ] {
        assert!(!audit.contains(protected));
    }
    assert!(audit.contains("\"private_recovery_state\":\"[REDACTED]\""));
    assert!(audit.contains("\"targets\":[]"));
}

#[test]
fn exact_preflight_failure_replay_is_idempotent_and_cannot_become_success() {
    let conn = crate::tests_support::complete_database();
    let failed = json!({
        "attempt_id": "attempt-preflight",
        "backup_id": "backup-preflight",
        "artifact_name": "backup-preflight.iotkit-node-backup",
        "edge_node_id": "edge-node-test",
        "reason_code": "storage_full",
        "started_at_ms": 10,
        "completed_at_ms": 10
    });
    dispatch_private(&conn, RECORD_BACKUP_PREFLIGHT_FAILURE_OP, failed.clone());
    dispatch_private(&conn, RECORD_BACKUP_PREFLIGHT_FAILURE_OP, failed);
    let result = dispatch(
        &conn,
        recovery_descriptors(),
        DispatchRequest {
            op: COMPLETE_BACKUP_ATTEMPT_OP.into(),
            params: json!({"private_recovery_state": {
                "attempt_id": "attempt-preflight",
                "outcome": "success",
                "reason_code": "ok",
                "artifact_length": 42,
                "ledger_epoch": "epoch-test",
                "accepted_cursor": 1,
                "allocation_high_water": 2,
                "artifact_created_at_ms": 10,
                "completed_at_ms": 11
            }}),
            dry_run: false,
            actor: Actor {
                actor_id: "backup-test".into(),
                actor_kind: ActorKind::LocalCli,
                tier_ceiling: Tier::Construction,
            },
            source: None,
            step_up_verified: true,
            clock_trust: None,
        },
    );
    assert!(result.is_err());
    assert_eq!(
        conn.query_row(
            "SELECT state || ':' || reason_code FROM edge_node_backup_attempts",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "failed:storage_full"
    );
    let arbitrary = dispatch(
        &conn,
        recovery_descriptors(),
        DispatchRequest {
            op: RECORD_BACKUP_PREFLIGHT_FAILURE_OP.into(),
            params: json!({"private_recovery_state": {
                "attempt_id": "attempt-free-text",
                "backup_id": "backup-free-text",
                "artifact_name": "backup-free-text.iotkit-node-backup",
                "edge_node_id": "edge-node-test",
                "reason_code": "passphrase=must-not-persist",
                "started_at_ms": 12,
                "completed_at_ms": 12
            }}),
            dry_run: false,
            actor: Actor {
                actor_id: "backup-test".into(),
                actor_kind: ActorKind::LocalCli,
                tier_ceiling: Tier::Construction,
            },
            source: None,
            step_up_verified: true,
            clock_trust: None,
        },
    );
    assert!(arbitrary.is_err());
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM edge_node_backup_attempts",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[cfg(target_os = "linux")]
fn status_fixture(
    root: &std::path::Path,
) -> (std::path::PathBuf, BackupConfig, rusqlite::Connection) {
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    let database = root.join("edge.db");
    let conn = crate::tests_support::active_database_with_publications(&database, 1, 2);
    let destination = root.join("destination");
    let staging = root.join("staging");
    fs::create_dir(&destination).unwrap();
    fs::create_dir(&staging).unwrap();
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o700)).unwrap();
    let config = BackupConfig {
        schema_version: 1,
        database,
        destination,
        staging_directory: staging,
        passphrase_file: root.join("passphrase"),
        expected_mount: MountIdentity {
            mount_point: root.to_path_buf(),
            source: "redacted-source".into(),
            filesystem_type: "test".into(),
            filesystem_id: "redacted-fsid".into(),
        },
        freshness_seconds: 60,
        retention_count: 2,
    };
    let config_path = root.join("backup.json");
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&config_path)
        .unwrap();
    file.write_all(&serde_json::to_vec(&config).unwrap())
        .unwrap();
    drop(file);
    (config_path, config, conn)
}

#[cfg(target_os = "linux")]
#[test]
fn status_reports_healthy_stale_latest_failure_and_active_operation_without_writes() {
    let root = tempfile::tempdir().unwrap();
    let (config_path, config, conn) = status_fixture(root.path());
    dispatch_private(
        &conn,
        BEGIN_BACKUP_ATTEMPT_OP,
        json!({
            "attempt_id": "attempt-one",
            "backup_id": "backup-one",
            "artifact_name": "backup-one.iotkit-node-backup",
            "edge_node_id": "edge-node-test",
            "started_at_ms": 1_000
        }),
    );
    dispatch_private(
        &conn,
        COMPLETE_BACKUP_ATTEMPT_OP,
        json!({
            "attempt_id": "attempt-one",
            "outcome": "success",
            "reason_code": "ok",
            "artifact_length": 42,
            "ledger_epoch": "epoch-test",
            "accepted_cursor": 1,
            "allocation_high_water": 2,
            "artifact_created_at_ms": 1_000,
            "completed_at_ms": 1_001
        }),
    );
    drop(conn);

    let lock_path = config_path.parent().unwrap().join(".iotkit-recovery.lock");
    assert!(!lock_path.exists());
    assert!(matches!(
        backup_status(&config_path, 60_999).unwrap(),
        BackupReadiness::Healthy { .. }
    ));
    assert!(matches!(
        backup_status(&config_path, 61_001).unwrap(),
        BackupReadiness::Stale { .. }
    ));
    assert!(!lock_path.exists(), "read-only status created a lock file");

    let conn = rusqlite::Connection::open(&config.database).unwrap();
    dispatch_private(
        &conn,
        BEGIN_BACKUP_ATTEMPT_OP,
        json!({
            "attempt_id": "attempt-interrupted",
            "backup_id": "backup-interrupted",
            "artifact_name": "backup-interrupted.iotkit-node-backup",
            "edge_node_id": "edge-node-test",
            "started_at_ms": 1_500
        }),
    );
    drop(conn);
    assert!(matches!(
        backup_status(&config_path, 1_501).unwrap(),
        BackupReadiness::Failed {
            ref reason_code,
            ..
        } if reason_code == "interrupted"
    ));
    let conn = rusqlite::Connection::open(&config.database).unwrap();
    dispatch_private(
        &conn,
        COMPLETE_BACKUP_ATTEMPT_OP,
        json!({
            "attempt_id": "attempt-interrupted",
            "outcome": "failed",
            "reason_code": "interrupted",
            "artifact_length": null,
            "ledger_epoch": null,
            "accepted_cursor": null,
            "allocation_high_water": null,
            "artifact_created_at_ms": null,
            "completed_at_ms": 1_502
        }),
    );
    dispatch_private(
        &conn,
        RECORD_BACKUP_PREFLIGHT_FAILURE_OP,
        json!({
            "attempt_id": "attempt-two",
            "backup_id": "backup-two",
            "artifact_name": "backup-two.iotkit-node-backup",
            "edge_node_id": "edge-node-test",
            "reason_code": "storage_full",
            "started_at_ms": 2_000,
            "completed_at_ms": 2_000
        }),
    );
    drop(conn);
    assert!(matches!(
        backup_status(&config_path, 2_001).unwrap(),
        BackupReadiness::Failed {
            ref reason_code,
            last_verified: Some(_),
            ..
        } if reason_code == "storage_full"
    ));

    let _guard = acquire_recovery_operation(&config_path).unwrap();
    assert_eq!(
        backup_status(&config_path, 2_002).unwrap(),
        BackupReadiness::OperationBusy
    );
}

#[cfg(target_os = "linux")]
#[test]
fn status_rejects_a_persisted_free_text_failure_reason_without_projecting_it() {
    let secret = "customer-secret-free-text-must-not-leave-storage";
    let root = tempfile::tempdir().unwrap();
    let (config_path, _config, conn) = status_fixture(root.path());
    conn.execute(
        "INSERT INTO edge_node_backup_attempts(
             attempt_id, backup_id, state, reason_code, artifact_name, edge_node_id,
             started_at_ms, completed_at_ms
         ) VALUES('attempt-corrupt', 'backup-corrupt', 'failed', ?1,
                  'backup-corrupt.iotkit-node-backup', 'edge-node-test', 10, 11)",
        [secret],
    )
    .unwrap();
    drop(conn);

    let result = backup_status(&config_path, 12);
    assert_eq!(result, Err(RecoveryError::InvalidStartupState));
    let rendered = format!("{result:?}");
    assert!(!rendered.contains(secret));
    assert_eq!(rendered, "Err(InvalidStartupState)");
}

#[cfg(target_os = "linux")]
#[test]
fn status_uses_insertion_order_when_attempt_timestamps_tie() {
    let root = tempfile::tempdir().unwrap();
    let (config_path, _config, conn) = status_fixture(root.path());
    for (attempt_id, backup_id) in [
        ("attempt-z-old", "backup-old"),
        ("attempt-a-new", "backup-new"),
    ] {
        dispatch_private(
            &conn,
            BEGIN_BACKUP_ATTEMPT_OP,
            json!({
                "attempt_id": attempt_id,
                "backup_id": backup_id,
                "artifact_name": format!("{backup_id}.iotkit-node-backup"),
                "edge_node_id": "edge-node-test",
                "started_at_ms": 100
            }),
        );
        dispatch_private(
            &conn,
            COMPLETE_BACKUP_ATTEMPT_OP,
            json!({
                "attempt_id": attempt_id,
                "outcome": "success",
                "reason_code": "ok",
                "artifact_length": 42,
                "ledger_epoch": "epoch-test",
                "accepted_cursor": 1,
                "allocation_high_water": 2,
                "artifact_created_at_ms": 100,
                "completed_at_ms": 101
            }),
        );
    }
    dispatch_private(
        &conn,
        RECORD_BACKUP_PREFLIGHT_FAILURE_OP,
        json!({
            "attempt_id": "attempt-0-latest",
            "backup_id": "backup-failed",
            "artifact_name": "backup-failed.iotkit-node-backup",
            "edge_node_id": "edge-node-test",
            "reason_code": "storage",
            "started_at_ms": 100,
            "completed_at_ms": 101
        }),
    );
    drop(conn);

    assert!(matches!(
        backup_status(&config_path, 102).unwrap(),
        BackupReadiness::Failed {
            ref reason_code,
            last_verified: Some(ref artifact),
            ..
        } if reason_code == "storage" && artifact.backup_id == "backup-new"
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn status_observes_initial_configure_lock_before_reporting_not_configured() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let config_path = root.path().join("backup.json");
    let _configure_guard = acquire_recovery_operation(&config_path).unwrap();

    assert_eq!(
        backup_status(&config_path, 1),
        Ok(BackupReadiness::OperationBusy)
    );
    assert!(!config_path.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn competing_create_returns_busy_before_opening_the_source_database() {
    let root = tempfile::tempdir().unwrap();
    use std::fs;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let staging = root.path().join("staging");
    fs::create_dir(&staging).unwrap();
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o700)).unwrap();
    let config = BackupConfig {
        schema_version: 1,
        database: root.path().join("source-does-not-exist.db"),
        destination: root.path().join("destination"),
        staging_directory: staging,
        passphrase_file: root.path().join("passphrase"),
        expected_mount: MountIdentity {
            mount_point: root.path().to_path_buf(),
            source: "source".into(),
            filesystem_type: "test".into(),
            filesystem_id: "fsid".into(),
        },
        freshness_seconds: 60,
        retention_count: 1,
    };
    let config_path = root.path().join("backup.json");
    let mut config_file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&config_path)
        .unwrap();
    use std::io::Write as _;
    config_file
        .write_all(&serde_json::to_vec(&config).unwrap())
        .unwrap();
    drop(config_file);
    let _guard = acquire_recovery_operation(&config_path).unwrap();
    assert_eq!(
        create_backup(
            &config_path,
            &BackupPassphrase::new("owner-only-passphrase".into()),
            1
        ),
        Err(RecoveryError::OperationBusy)
    );
    assert!(!config.database.exists());
}

#[cfg(target_os = "linux")]
fn held_staging(path: &std::path::Path) -> VerifiedStagingDirectory {
    VerifiedStagingDirectory {
        directory: DirectoryCapability::open(path).unwrap(),
    }
}

#[cfg(target_os = "linux")]
#[test]
fn restart_cleanup_removes_only_exact_private_single_link_plaintext_stages() {
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let exact = root
        .path()
        .join(".iotkit-backup-stage-0123456789abcdef0123456789abcdef.sqlite");
    let near = root
        .path()
        .join(".iotkit-backup-stage-0123456789abcdef.sqlite");
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&exact)
        .unwrap();
    file.write_all(b"plaintext-sensitive").unwrap();
    drop(file);
    for suffix in ["-journal", "-wal", "-shm"] {
        let sidecar = exact.with_file_name(format!(
            "{}{}",
            exact.file_name().unwrap().to_string_lossy(),
            suffix
        ));
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(sidecar)
            .unwrap();
    }
    fs::write(&near, b"unrelated").unwrap();

    assert_eq!(
        crate::backup::cleanup_prior_plaintext(&held_staging(root.path())),
        Err(RecoveryError::CleanupRequired)
    );
    assert!(!exact.exists());
    for suffix in ["-journal", "-wal", "-shm"] {
        assert!(
            !exact
                .with_file_name(format!(
                    "{}{}",
                    exact.file_name().unwrap().to_string_lossy(),
                    suffix
                ))
                .exists()
        );
    }
    assert_eq!(fs::read(&near).unwrap(), b"unrelated");
}

#[cfg(target_os = "linux")]
#[test]
fn restart_cleanup_preserves_unsafe_plaintext_stage_classifications() {
    use std::fs::{self, OpenOptions};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};

    for classification in ["broad_mode", "hardlink", "symlink"] {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let exact = root
            .path()
            .join(".iotkit-backup-stage-fedcba9876543210fedcba9876543210.sqlite");
        match classification {
            "broad_mode" => {
                OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o644)
                    .open(&exact)
                    .unwrap();
            }
            "hardlink" => {
                let owned = root.path().join("owned");
                OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o600)
                    .open(&owned)
                    .unwrap();
                fs::hard_link(&owned, &exact).unwrap();
            }
            "symlink" => {
                let unrelated = root.path().join("unrelated");
                fs::write(&unrelated, b"keep").unwrap();
                symlink(&unrelated, &exact).unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(
            crate::backup::cleanup_prior_plaintext(&held_staging(root.path())),
            Err(RecoveryError::CleanupRequired),
            "{classification}"
        );
        assert!(
            fs::symlink_metadata(&exact).is_ok(),
            "{classification} was removed"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn restart_cleanup_converges_after_a_child_process_abort_with_sqlite_sidecars() {
    use std::{os::unix::process::ExitStatusExt, process::Command};

    let fixture = create_fixture();
    let stage = fixture
        .staging
        .path()
        .join(".iotkit-backup-stage-0123456789abcdef0123456789abcdef.sqlite");
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "backup_tests::snapshot_stage_abort_child",
            "--nocapture",
        ])
        .env(
            "IOTKIT_RECOVERY_TEST_CRASH_SOURCE",
            &fixture.config.database,
        )
        .env("IOTKIT_RECOVERY_TEST_CRASH_STAGE", &stage)
        .status()
        .unwrap();
    assert_eq!(
        child.signal(),
        Some(libc::SIGABRT),
        "crash child did not abort at the sidecar hook"
    );

    assert!(stage.exists(), "crash must leave the SQLite stage behind");
    assert!(
        stage
            .with_file_name(format!(
                "{}-journal",
                stage.file_name().unwrap().to_string_lossy()
            ))
            .exists(),
        "crash must leave the SQLite journal sidecar behind"
    );
    let staging = held_staging(fixture.staging.path());
    assert_eq!(crate::backup::cleanup_prior_plaintext(&staging), Ok(()));
    assert_eq!(
        std::fs::read_dir(fixture.staging.path()).unwrap().count(),
        0
    );
}

#[cfg(target_os = "linux")]
#[test]
fn snapshot_stage_abort_child() {
    let (Some(source), Some(stage)) = (
        std::env::var_os("IOTKIT_RECOVERY_TEST_CRASH_SOURCE"),
        std::env::var_os("IOTKIT_RECOVERY_TEST_CRASH_STAGE"),
    ) else {
        return;
    };
    struct AbortAfterCredentialUpdate {
        stage: std::path::PathBuf,
    }
    impl crate::snapshot::SnapshotHook for AbortAfterCredentialUpdate {
        fn at(&self, point: crate::snapshot::SnapshotHookPoint) -> Result<(), RecoveryError> {
            if point == crate::snapshot::SnapshotHookPoint::AfterCredentialUpdate {
                let conn = rusqlite::Connection::open(&self.stage).unwrap();
                conn.execute_batch(
                    "BEGIN IMMEDIATE;
                     UPDATE ledger_meta SET value='journal-crash-marker'
                     WHERE key='edge_node_id';",
                )
                .unwrap();
                let journal = self.stage.with_file_name(format!(
                    "{}-journal",
                    self.stage.file_name().unwrap().to_string_lossy()
                ));
                assert!(journal.exists(), "SQLite transaction must create journal");
                std::fs::File::open(journal).unwrap().sync_all().unwrap();
                std::process::abort();
            }
            Ok(())
        }
    }
    let source = std::path::PathBuf::from(source);
    let stage = std::path::PathBuf::from(stage);
    let hook = AbortAfterCredentialUpdate {
        stage: stage.clone(),
    };
    let _ = crate::snapshot::create_consistent_snapshot_with_hook(
        &source,
        &stage,
        "crash-child",
        1,
        &hook,
    );
}

#[cfg(target_os = "linux")]
struct CreateFixture {
    _control: tempfile::TempDir,
    destination: tempfile::TempDir,
    _database_root: tempfile::TempDir,
    _staging_parent: tempfile::TempDir,
    staging: tempfile::TempDir,
    config_path: std::path::PathBuf,
    config: BackupConfig,
    passphrase: BackupPassphrase,
}

#[cfg(target_os = "linux")]
fn create_fixture() -> CreateFixture {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let control = tempfile::TempDir::new().unwrap();
    let destination = tempfile::TempDir::new_in("/dev/shm").unwrap();
    let database_root = tempfile::TempDir::new().unwrap();
    let staging_parent = tempfile::TempDir::new_in("/dev/shm").unwrap();
    let staging = tempfile::TempDir::new_in(staging_parent.path()).unwrap();
    for directory in [
        control.path(),
        destination.path(),
        database_root.path(),
        staging.path(),
    ] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let database = database_root.path().join("edge.db");
    drop(crate::tests_support::active_database_with_publications(
        &database, 1, 3,
    ));
    let config_path = control.path().join("backup.json");
    configure_backup(
        &config_path,
        &BackupConfig {
            schema_version: 1,
            database,
            destination: destination.path().to_path_buf(),
            staging_directory: staging.path().to_path_buf(),
            passphrase_file: control.path().join("passphrase"),
            expected_mount: MountIdentity {
                mount_point: destination.path().to_path_buf(),
                source: "derived".into(),
                filesystem_type: "derived".into(),
                filesystem_id: "derived".into(),
            },
            freshness_seconds: 60,
            retention_count: 1,
        },
        BackupConfigReplace::Refuse,
    )
    .unwrap();
    let config = load_owner_only_config(&config_path).unwrap();
    CreateFixture {
        _control: control,
        destination,
        _database_root: database_root,
        _staging_parent: staging_parent,
        staging,
        config_path,
        config,
        passphrase: BackupPassphrase::new("owner-only-reconcile-passphrase".into()),
    }
}

#[cfg(target_os = "linux")]
#[test]
fn snapshot_capacity_failure_is_recorded_before_begin_and_uses_manifest_length() {
    let fixture = create_fixture();
    let expected_snapshot_path = fixture.staging.path().join("expected-snapshot.sqlite");
    let expected_snapshot = create_consistent_snapshot(
        &fixture.config.database,
        &expected_snapshot_path,
        "expected-snapshot",
        9,
    )
    .unwrap();
    let expected_snapshot_length = expected_snapshot.manifest.database_length;
    std::fs::remove_file(&expected_snapshot_path).unwrap();
    let hook = SnapshotCapacityFailureHook {
        requested: std::cell::Cell::new(None),
    };

    assert_eq!(
        crate::backup::create_backup_with_hook(&fixture.config_path, &fixture.passphrase, 9, &hook,),
        Err(RecoveryError::StorageFull)
    );
    let requested = hook.requested.get().expect("snapshot capacity check");
    assert_eq!(requested, expected_snapshot_length);

    let conn = rusqlite::Connection::open(&fixture.config.database).unwrap();
    let receipt: (String, String) = conn
        .query_row(
            "SELECT state, reason_code FROM edge_node_backup_attempts",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(receipt, ("failed".into(), "storage_full".into()));
    assert!(!fixture.staging.path().read_dir().unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".iotkit-backup-stage-")
    }));
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM edge_node_backup_attempts WHERE state='started'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
    );
}

#[cfg(target_os = "linux")]
#[test]
fn config_adjacent_configure_lock_blocks_create_and_inspect_before_effects() {
    let fixture = create_fixture();
    let _configure_guard = acquire_recovery_operation(&fixture.config_path).unwrap();

    assert_eq!(
        create_backup(&fixture.config_path, &fixture.passphrase, 9),
        Err(RecoveryError::OperationBusy)
    );
    assert_eq!(
        inspect_backup(
            std::path::Path::new("must-not-be-opened.iotkit-node-backup"),
            &fixture.passphrase,
        ),
        Err(RecoveryError::Storage)
    );
    let conn = rusqlite::Connection::open(&fixture.config.database).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM edge_node_backup_attempts",
            [],
            |row| { row.get::<_, i64>(0) }
        )
        .unwrap(),
        0
    );
    assert_eq!(
        std::fs::read_dir(fixture.destination.path())
            .unwrap()
            .count(),
        0
    );
}

#[cfg(target_os = "linux")]
struct ConfigureDuringCreate<'a> {
    config_path: &'a std::path::Path,
    observed: std::cell::RefCell<Option<Result<(), RecoveryError>>>,
}

#[cfg(target_os = "linux")]
struct ConfigureReplacementDuringSelectedCreate<'a> {
    config_path: &'a std::path::Path,
    replacement: &'a BackupConfig,
    observed: std::cell::RefCell<Option<Result<(), RecoveryError>>>,
}

#[cfg(target_os = "linux")]
impl crate::backup::BackupHook for ConfigureReplacementDuringSelectedCreate<'_> {
    fn at(
        &self,
        point: crate::backup::BackupHookPoint,
        _config: &BackupConfig,
    ) -> Result<(), RecoveryError> {
        if point == crate::backup::BackupHookPoint::BeforeSnapshot {
            self.observed.replace(Some(configure_backup(
                self.config_path,
                self.replacement,
                BackupConfigReplace::ReplaceExisting,
            )));
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl crate::backup::BackupHook for ConfigureDuringCreate<'_> {
    fn at(
        &self,
        point: crate::backup::BackupHookPoint,
        config: &BackupConfig,
    ) -> Result<(), RecoveryError> {
        if point == crate::backup::BackupHookPoint::BeforeSnapshot {
            self.observed.replace(Some(configure_backup(
                self.config_path,
                config,
                BackupConfigReplace::ReplaceExisting,
            )));
            return Err(RecoveryError::Storage);
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[test]
fn active_create_blocks_configure_before_config_replacement() {
    let fixture = create_fixture();
    let config_before = std::fs::read(&fixture.config_path).unwrap();
    let hook = ConfigureDuringCreate {
        config_path: &fixture.config_path,
        observed: std::cell::RefCell::new(None),
    };

    assert_eq!(
        crate::backup::create_backup_with_hook(&fixture.config_path, &fixture.passphrase, 9, &hook,),
        Err(RecoveryError::Storage)
    );
    assert_eq!(
        hook.observed.into_inner(),
        Some(Err(RecoveryError::OperationBusy))
    );
    assert_eq!(std::fs::read(&fixture.config_path).unwrap(), config_before);
    let conn = rusqlite::Connection::open(&fixture.config.database).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM edge_node_backup_attempts",
            [],
            |row| { row.get::<_, i64>(0) }
        )
        .unwrap(),
        0
    );
}

#[cfg(target_os = "linux")]
#[test]
fn create_from_files_holds_config_and_passphrase_selection_until_artifact_completion() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = create_fixture();
    let passphrase_a = "owner-only-selection-passphrase-a";
    let passphrase_b = "owner-only-selection-passphrase-b";
    std::fs::write(&fixture.config.passphrase_file, passphrase_a).unwrap();
    std::fs::set_permissions(
        &fixture.config.passphrase_file,
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let passphrase_b_path = fixture
        .config
        .passphrase_file
        .parent()
        .unwrap()
        .join("passphrase-b");
    std::fs::write(&passphrase_b_path, passphrase_b).unwrap();
    std::fs::set_permissions(&passphrase_b_path, std::fs::Permissions::from_mode(0o600)).unwrap();

    let mut replacement = fixture.config.clone();
    replacement.passphrase_file = passphrase_b_path;
    let config_before = std::fs::read(&fixture.config_path).unwrap();
    let hook = ConfigureReplacementDuringSelectedCreate {
        config_path: &fixture.config_path,
        replacement: &replacement,
        observed: std::cell::RefCell::new(None),
    };
    let manifest =
        crate::backup::create_backup_from_files_with_hook(&fixture.config_path, 9, &hook).unwrap();

    assert_eq!(
        hook.observed.into_inner(),
        Some(Err(RecoveryError::OperationBusy))
    );
    assert_eq!(std::fs::read(&fixture.config_path).unwrap(), config_before);
    let artifact = fixture
        .destination
        .path()
        .join(format!("{}{}", manifest.backup_id, NODE_BACKUP_SUFFIX));
    assert_eq!(
        inspect_backup(&artifact, &BackupPassphrase::new(passphrase_a.into())).unwrap(),
        manifest
    );
    assert!(inspect_backup(&artifact, &BackupPassphrase::new(passphrase_b.into())).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn next_create_cleans_named_crash_plaintext_before_reconciling_the_exact_artifact() {
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;

    let fixture = create_fixture();
    let guard = acquire_recovery_operation(&fixture.config_path).unwrap();
    let reconciliation_destination =
        crate::destination::verify_destination_for_reconciliation(&guard, &fixture.config).unwrap();
    drop(reconciliation_destination);
    assert!(matches!(
        verify_destination(&guard, &fixture.config, u64::MAX),
        Err(RecoveryError::CapacityOverflow)
    ));
    let length = fs::metadata(&fixture.config.database).unwrap().len();
    let destination = verify_destination(&guard, &fixture.config, length).unwrap();
    let snapshot_path = fixture.staging.path().join("manual-snapshot.db");
    let snapshot = create_consistent_snapshot(
        &fixture.config.database,
        &snapshot_path,
        "backup-reconcile",
        10,
    )
    .unwrap();
    let artifact_name = format!("backup-reconcile{NODE_BACKUP_SUFFIX}");
    encrypt_container(
        &snapshot.path,
        &snapshot.manifest,
        &fixture.passphrase,
        destination.capability(),
        &artifact_name,
    )
    .unwrap();
    fs::remove_file(&snapshot_path).unwrap();
    drop(destination);
    drop(guard);

    let conn = rusqlite::Connection::open(&fixture.config.database).unwrap();
    dispatch_private(
        &conn,
        BEGIN_BACKUP_ATTEMPT_OP,
        json!({
            "attempt_id": "attempt-reconcile",
            "backup_id": "backup-reconcile",
            "artifact_name": artifact_name,
            "edge_node_id": "edge-node-test",
            "started_at_ms": 10
        }),
    );
    drop(conn);
    let crash_stage = fixture
        .staging
        .path()
        .join(".iotkit-backup-stage-0123456789abcdef0123456789abcdef.sqlite");
    let mut crash_plaintext = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&crash_stage)
        .unwrap();
    crash_plaintext
        .write_all(b"named-plaintext-left-by-a-dead-process")
        .unwrap();
    drop(crash_plaintext);
    let unsafe_near_stage = fixture
        .staging
        .path()
        .join(".iotkit-backup-stage-secret.sqlite");
    fs::write(&unsafe_near_stage, b"not-safe-to-classify").unwrap();
    let original_permissions = fs::metadata(fixture.destination.path())
        .unwrap()
        .permissions();
    let mut read_only_permissions = original_permissions.clone();
    use std::os::unix::fs::PermissionsExt;
    read_only_permissions.set_mode(0o500);
    fs::set_permissions(fixture.destination.path(), read_only_permissions).unwrap();
    assert_eq!(
        create_backup(&fixture.config_path, &fixture.passphrase, 11),
        Err(RecoveryError::CleanupRequired)
    );
    assert!(!crash_stage.exists());
    assert!(unsafe_near_stage.exists());
    let conn = rusqlite::Connection::open(&fixture.config.database).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT state FROM edge_node_backup_attempts WHERE attempt_id='attempt-reconcile'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "started"
    );
    drop(conn);
    fs::remove_file(&unsafe_near_stage).unwrap();
    let reconciled = create_backup(&fixture.config_path, &fixture.passphrase, 11).unwrap();
    fs::set_permissions(fixture.destination.path(), original_permissions).unwrap();
    assert_eq!(reconciled, snapshot.manifest);

    let exact_path = fixture.destination.path().join(&artifact_name);
    let unrelated = fixture
        .destination
        .path()
        .join(format!("unreferenced{NODE_BACKUP_SUFFIX}"));
    fs::copy(&exact_path, &unrelated).unwrap();
    let mismatched_exact = fixture
        .destination
        .path()
        .join(format!("backup-missing{NODE_BACKUP_SUFFIX}"));
    fs::copy(&exact_path, &mismatched_exact).unwrap();
    let conn = rusqlite::Connection::open(&fixture.config.database).unwrap();
    dispatch_private(
        &conn,
        BEGIN_BACKUP_ATTEMPT_OP,
        json!({
            "attempt_id": "attempt-missing",
            "backup_id": "backup-missing",
            "artifact_name": format!("backup-missing{NODE_BACKUP_SUFFIX}"),
            "edge_node_id": "edge-node-test",
            "started_at_ms": 12
        }),
    );
    drop(conn);
    let created = create_backup(&fixture.config_path, &fixture.passphrase, 13).unwrap();
    assert_ne!(created.backup_id, "backup-reconcile");
    assert!(unrelated.exists());
    assert!(mismatched_exact.exists());
    let remaining = fs::read_dir(fixture.destination.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(!exact_path.exists(), "remaining artifacts: {remaining:?}");
    let conn = rusqlite::Connection::open(&fixture.config.database).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT state || ':' || reason_code
             FROM edge_node_backup_attempts WHERE attempt_id='attempt-missing'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "failed:interrupted"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn crash_after_begin_leaves_started_then_next_create_cleans_and_closes_it() {
    let fixture = create_fixture();
    assert_eq!(
        create_backup_with_fault(
            &fixture.config_path,
            &fixture.passphrase,
            20,
            TestBackupFault::AfterBegin,
        ),
        Err(RecoveryError::Storage)
    );
    let conn = rusqlite::Connection::open(&fixture.config.database).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT state FROM edge_node_backup_attempts ORDER BY started_at_ms DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "started"
    );
    drop(conn);
    assert!(
        std::fs::read_dir(fixture.staging.path())
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".iotkit-backup-stage-"))
    );

    create_backup(&fixture.config_path, &fixture.passphrase, 21).unwrap();
    let conn = rusqlite::Connection::open(&fixture.config.database).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT state || ':' || reason_code
             FROM edge_node_backup_attempts WHERE started_at_ms=20",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "failed:interrupted"
    );
    assert!(
        !std::fs::read_dir(fixture.staging.path())
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".iotkit-backup-stage-"))
    );
}

#[cfg(target_os = "linux")]
#[test]
fn post_publication_faults_keep_started_until_exact_next_create_reconciliation() {
    for fault in [
        TestBackupFault::AfterPublication,
        TestBackupFault::AfterReadback,
        TestBackupFault::BeforeReceipt,
    ] {
        let fixture = create_fixture();
        assert_eq!(
            create_backup_with_fault(&fixture.config_path, &fixture.passphrase, 30, fault),
            Err(RecoveryError::ArtifactPublicationUncertain)
        );
        let conn = rusqlite::Connection::open(&fixture.config.database).unwrap();
        let backup_id: String = conn
            .query_row(
                "SELECT backup_id FROM edge_node_backup_attempts WHERE state='started'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(conn);
        let reconciled = create_backup(&fixture.config_path, &fixture.passphrase, 31).unwrap();
        assert_eq!(reconciled.backup_id, backup_id);
        assert!(
            !std::fs::read_dir(fixture.staging.path())
                .unwrap()
                .any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".iotkit-backup-stage-"))
        );
    }
}

#[cfg(target_os = "linux")]
struct AbortBackupHook(&'static str);

#[cfg(target_os = "linux")]
impl crate::backup::BackupHook for AbortBackupHook {
    fn at(
        &self,
        point: crate::backup::BackupHookPoint,
        _config: &BackupConfig,
    ) -> Result<(), RecoveryError> {
        let phase = match point {
            crate::backup::BackupHookPoint::AfterSnapshot => "after_snapshot",
            crate::backup::BackupHookPoint::AfterBegin => "after_begin",
            crate::backup::BackupHookPoint::AfterPublication => "after_publication",
            crate::backup::BackupHookPoint::BeforeReceipt => "before_receipt",
            crate::backup::BackupHookPoint::AfterReceipt => "after_receipt",
            _ => return Ok(()),
        };
        if phase == self.0 {
            std::process::abort();
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[test]
fn backup_process_abort_windows_reconcile_without_started_or_plaintext_stage() {
    use std::{
        os::unix::{fs::PermissionsExt, process::ExitStatusExt},
        process::Command,
    };

    for phase in [
        "after_snapshot",
        "after_begin",
        "after_publication",
        "before_receipt",
        "after_receipt",
    ] {
        let fixture = create_fixture();
        std::fs::write(
            &fixture.config.passphrase_file,
            b"owner-only-reconcile-passphrase",
        )
        .unwrap();
        std::fs::set_permissions(
            &fixture.config.passphrase_file,
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "backup_tests::backup_abort_child", "--nocapture"])
            .env("IOTKIT_RECOVERY_TEST_BACKUP_CONFIG", &fixture.config_path)
            .env("IOTKIT_RECOVERY_TEST_BACKUP_PHASE", phase)
            .status()
            .unwrap();
        assert_eq!(
            child.signal(),
            Some(libc::SIGABRT),
            "{phase} child did not abort at its requested hook"
        );

        let reconciled = create_backup(&fixture.config_path, &fixture.passphrase, 101)
            .unwrap_or_else(|error| panic!("{phase} retry failed: {error:?}"));
        assert!(!reconciled.backup_id.is_empty());
        assert!(
            !std::fs::read_dir(fixture.staging.path())
                .unwrap()
                .any(|entry| {
                    entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".iotkit-backup-stage-")
                })
        );
        let conn = rusqlite::Connection::open(&fixture.config.database).unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM edge_node_backup_attempts WHERE state='started'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
        assert!(
            conn.query_row(
                "SELECT count(*) FROM edge_node_backup_attempts WHERE state='success'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
                > 0,
            "{phase} has no durable success receipt"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn backup_abort_child() {
    let (Some(config), Some(phase)) = (
        std::env::var_os("IOTKIT_RECOVERY_TEST_BACKUP_CONFIG"),
        std::env::var_os("IOTKIT_RECOVERY_TEST_BACKUP_PHASE"),
    ) else {
        return;
    };
    let config_path = std::path::PathBuf::from(config);
    let phase = phase.to_string_lossy().to_string();
    let passphrase = crate::config::load_owner_only_passphrase(
        &crate::config::load_owner_only_config(&config_path)
            .unwrap()
            .passphrase_file,
    )
    .unwrap();
    let _ = crate::backup::create_backup_with_hook(
        &config_path,
        &passphrase,
        100,
        &AbortBackupHook(Box::leak(phase.into_boxed_str())),
    );
}

#[cfg(target_os = "linux")]
#[test]
fn reconciliation_parent_sync_failure_keeps_started_until_durability_is_proven() {
    let fixture = create_fixture();
    assert_eq!(
        create_backup_with_fault(
            &fixture.config_path,
            &fixture.passphrase,
            35,
            TestBackupFault::AfterPublication,
        ),
        Err(RecoveryError::ArtifactPublicationUncertain)
    );
    assert_eq!(
        create_backup_with_fault(
            &fixture.config_path,
            &fixture.passphrase,
            36,
            TestBackupFault::ReconciliationParentSync,
        ),
        Err(RecoveryError::ArtifactPublicationUncertain)
    );
    let conn = rusqlite::Connection::open(&fixture.config.database).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT state FROM edge_node_backup_attempts ORDER BY started_at_ms DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "started"
    );
    drop(conn);

    create_backup(&fixture.config_path, &fixture.passphrase, 37).unwrap();
    let conn = rusqlite::Connection::open(&fixture.config.database).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT state FROM edge_node_backup_attempts ORDER BY started_at_ms DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "success"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn receipt_commit_precedes_retention_and_success_reporting() {
    let fixture = create_fixture();
    assert_eq!(
        create_backup_with_fault(
            &fixture.config_path,
            &fixture.passphrase,
            40,
            TestBackupFault::AfterReceipt,
        ),
        Err(RecoveryError::Storage)
    );
    let conn = rusqlite::Connection::open(&fixture.config.database).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT state || ':' || reason_code FROM edge_node_backup_attempts",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "success:ok"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn source_path_replacement_cannot_complete_a_receipt_for_another_node() {
    let fixture = create_fixture();
    let replacement = fixture.config.database.with_extension("replacement");
    let replacement_conn =
        crate::tests_support::active_database_with_publications(&replacement, 1, 3);
    replacement_conn
        .execute(
            "UPDATE ledger_meta SET value='other-node' WHERE key='edge_node_id'",
            [],
        )
        .unwrap();
    replacement_conn
        .execute(
            "UPDATE ledger_meta SET value='other-epoch' WHERE key='epoch'",
            [],
        )
        .unwrap();
    replacement_conn
        .execute(
            "UPDATE edge_node_activation SET ledger_epoch='other-epoch' WHERE singleton=1",
            [],
        )
        .unwrap();
    replacement_conn
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(replacement_conn);

    assert_eq!(
        create_backup_with_fault(
            &fixture.config_path,
            &fixture.passphrase,
            50,
            TestBackupFault::ReplaceSourceBeforeSnapshot,
        ),
        Err(RecoveryError::InvalidSnapshot)
    );
    let original =
        rusqlite::Connection::open(fixture.config.database.with_extension("held-original"))
            .unwrap();
    assert_eq!(
        original
            .query_row(
                "SELECT count(*) FROM edge_node_backup_attempts",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
