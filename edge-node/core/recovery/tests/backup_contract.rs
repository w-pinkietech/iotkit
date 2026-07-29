#[cfg(not(target_os = "linux"))]
use std::path::Path;

use iotkit_core_recovery::{
    BEGIN_BACKUP_ATTEMPT_OP, BackupReadiness, COMPLETE_BACKUP_ATTEMPT_OP, INSTALL_CANDIDATE_OP,
    RECORD_BACKUP_PREFLIGHT_FAILURE_OP, RecoveryError, backup_status, inspect_backup,
    recovery_descriptors,
};
#[cfg(target_os = "linux")]
use iotkit_core_recovery::{
    BackupConfig, BackupConfigReplace, BackupPassphrase, MountIdentity, NODE_BACKUP_SUFFIX,
    all_edge_node_migrations, configure_backup, create_backup,
};
use tempfile::tempdir;

#[test]
fn backup_operations_are_typed_construction_operations() {
    let descriptors = recovery_descriptors();
    for name in [
        BEGIN_BACKUP_ATTEMPT_OP,
        COMPLETE_BACKUP_ATTEMPT_OP,
        RECORD_BACKUP_PREFLIGHT_FAILURE_OP,
        INSTALL_CANDIDATE_OP,
    ] {
        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.name == name)
            .expect("backup operation descriptor");
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
}

#[test]
fn status_is_not_configured_when_the_owner_config_is_absent() {
    let root = tempdir().unwrap();
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let status = backup_status(&root.path().join("backup.json"), 1_725_000_000_000).unwrap();
    assert_eq!(status, BackupReadiness::NotConfigured);
    assert_eq!(format!("{status:?}"), "BackupReadiness::NotConfigured");
}

#[test]
fn inspection_of_an_absent_artifact_is_a_closed_redacted_error() {
    let passphrase =
        iotkit_core_recovery::BackupPassphrase::new("owner-only-test-passphrase".to_string());
    #[cfg(target_os = "linux")]
    let error = {
        use std::fs::{self, OpenOptions};
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let root = tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let config_path = root.path().join("backup.json");
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
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&config_path)
            .unwrap();
        file.write_all(&serde_json::to_vec(&config).unwrap())
            .unwrap();
        drop(file);
        inspect_backup(&root.path().join("absent.iotkit-node-backup"), &passphrase).unwrap_err()
    };
    #[cfg(not(target_os = "linux"))]
    let error = inspect_backup(Path::new("absent.iotkit-node-backup"), &passphrase).unwrap_err();
    assert_eq!(
        error,
        if cfg!(target_os = "linux") {
            RecoveryError::Storage
        } else {
            RecoveryError::PlatformUnsupported
        }
    );
    assert!(!format!("{error:?}").contains("absent"));
    assert!(!error.to_string().contains("absent"));
}

#[cfg(target_os = "linux")]
#[test]
fn encrypted_backup_round_trips_custody_state_and_redacts_receipt_audit() {
    use std::fs::{self, OpenOptions};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    use iotkit_core_storage::run_migrations;
    use rusqlite::{Connection, params};
    use tempfile::TempDir;

    let control = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let database_root = TempDir::new_in("/dev/shm").unwrap();
    let staging = TempDir::new_in("/dev/shm").unwrap();
    for directory in [
        control.path(),
        destination.path(),
        database_root.path(),
        staging.path(),
    ] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let database = database_root.path().join("edge.db");
    let conn = Connection::open(&database).unwrap();
    run_migrations(&conn, &all_edge_node_migrations()).unwrap();
    conn.execute(
        "INSERT INTO ledger_meta(key, value) VALUES
             ('edge_node_id', ?1), ('epoch', ?2), ('generation', '1')",
        params!["edge-node-sensitive-id", "ledger-sensitive-epoch"],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO devices(
             system_id, hardware_id, user_label, kind, state, created_at
         ) VALUES(?1, 'hardware-secret', 'Fixture', 'individual', 'active', 1)",
        [vec![1_u8; 16]],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO series(
             system_id, measurement_key, channel_index, variant, created_at
         ) VALUES(?1, 'temperature_c', -1, 'primary', 1)",
        [vec![1_u8; 16]],
    )
    .unwrap();
    for sequence in 1..=3_i64 {
        conn.execute(
            "INSERT INTO readings(
                 series_id, received_at, device_time, time_source, time_quality,
                 values_json, event_time, event_time_source, quarantined
             ) VALUES(1, ?1, NULL, 'edge_node', 'unsynced', '[21.5]', ?1,
                      'received_at', ?2)",
            params![sequence, i64::from(sequence == 3)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO publication_log(epoch, kind, reading_seq, created_at)
             VALUES('ledger-sensitive-epoch', 'measurement', ?1, ?1)",
            [sequence],
        )
        .unwrap();
    }
    conn.execute("DELETE FROM publication_log WHERE pub_seq=1", [])
        .unwrap();
    conn.execute(
        "INSERT INTO target_registry(
             target_id, endpoint_url, credential_token, archive_responsible,
             schema_version, cursor_epoch, cursor_pub_seq, created_at
         ) VALUES('edge', 'https://invalid', ?1, 1, 1,
                  'ledger-sensitive-epoch', 1, 1)",
        ["device-token-hash-secret"],
    )
    .unwrap();
    conn.execute(
        "UPDATE edge_node_activation
         SET state='active', edge_id='edge-sensitive-id',
             activation_id='activation-sensitive-id',
             ledger_epoch='ledger-sensitive-epoch',
             discard_through_reading_seq=0, cleanup_through_reading_seq=0,
             request_json='{}', result_json='{}', activated_at=1
         WHERE singleton=1",
        [],
    )
    .unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(conn);

    let passphrase_path = control.path().join("passphrase");
    let mut passphrase_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&passphrase_path)
        .unwrap();
    use std::io::Write as _;
    passphrase_file
        .write_all(b"backup-passphrase-sensitive")
        .unwrap();
    drop(passphrase_file);

    let config_path = control.path().join("backup.json");
    configure_backup(
        &config_path,
        &BackupConfig {
            schema_version: 1,
            database: database.clone(),
            destination: destination.path().to_path_buf(),
            staging_directory: staging.path().to_path_buf(),
            passphrase_file: passphrase_path,
            expected_mount: MountIdentity {
                mount_point: destination.path().to_path_buf(),
                source: "ignored-during-configure".into(),
                filesystem_type: "ignored".into(),
                filesystem_id: "ignored".into(),
            },
            freshness_seconds: 60,
            retention_count: 2,
        },
        BackupConfigReplace::Refuse,
    )
    .unwrap();
    let passphrase = BackupPassphrase::new("backup-passphrase-sensitive".into());
    let manifest = create_backup(&config_path, &passphrase, 1_725_000_000_000).unwrap();
    let artifact = destination
        .path()
        .join(format!("{}{}", manifest.backup_id, NODE_BACKUP_SUFFIX));
    assert_eq!(inspect_backup(&artifact, &passphrase).unwrap(), manifest);
    assert_eq!(manifest.accepted_cursor, 1);
    assert_eq!(manifest.allocation_high_water, 3);
    assert_eq!(manifest.counts.quarantine_rows, 1);

    let receipt = Connection::open(&database).unwrap();
    let state: String = receipt
        .query_row(
            "SELECT state FROM edge_node_backup_attempts WHERE backup_id=?1",
            [manifest.backup_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "success");
    let audit = receipt
        .prepare("SELECT detail FROM ledger_events WHERE kind='r14_op'")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    let database_text = database.to_string_lossy().into_owned();
    for secret in [
        "edge-node-sensitive-id",
        "ledger-sensitive-epoch",
        "edge-sensitive-id",
        "activation-sensitive-id",
        "device-token-hash-secret",
        "backup-passphrase-sensitive",
        manifest.backup_id.as_str(),
        database_text.as_str(),
    ] {
        assert!(!audit.contains(secret), "audit leaked a protected value");
    }
    assert!(audit.contains("\"private_recovery_state\":\"[REDACTED]\""));
    assert!(audit.contains("\"targets\":[]"));
}
