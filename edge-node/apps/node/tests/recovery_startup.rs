use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection;

const SENTINEL: &str = "sentinel-sensitive-config";

fn create_fenced_candidate(path: &Path) {
    // Build this fixture through the same source -> snapshot -> encrypted
    // restore path used by `nodectl`, rather than synthesizing the candidate
    // row. This exercises publication, receipt, authority, and sidecar
    // cleanup before the node binary is launched.
    let root = path.parent().unwrap();
    let source = root.join("restore-source.db");
    let config_path = root.join("restore-backup.json");
    let passphrase_path = root.join("restore-passphrase");
    let destination = tempfile::tempdir_in("/dev/shm").unwrap();
    let backup_staging_parent = tempfile::tempdir_in("/dev/shm").unwrap();
    let backup_staging = tempfile::tempdir_in(backup_staging_parent.path()).unwrap();
    let staging_parent = tempfile::tempdir_in("/dev/shm").unwrap();
    let staging = tempfile::tempdir_in(staging_parent.path()).unwrap();
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        for directory in [
            root,
            destination.path(),
            backup_staging_parent.path(),
            backup_staging.path(),
            staging_parent.path(),
            staging.path(),
        ] {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    let conn = Connection::open(&source).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    iotkit_core_storage::run_migrations(&conn, &iotkit_core_recovery::all_edge_node_migrations())
        .unwrap();
    conn.execute(
        "INSERT INTO ledger_meta(key, value) VALUES
             ('edge_node_id', 'candidate-node'), ('epoch', 'epoch-old'), ('generation', '1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO devices(system_id, hardware_id, user_label, kind, state, created_at)
         VALUES(?1, 'fixture-device', 'Fixture device', 'individual', 'active', 1)",
        [vec![1_u8; 16]],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO series(system_id, measurement_key, channel_index, variant, created_at)
         VALUES(?1, 'temperature_c', -1, 'primary', 1)",
        [vec![1_u8; 16]],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO readings(
             series_id, received_at, device_time, time_source, time_quality,
             values_json, event_time, event_time_source
         ) VALUES(1, 1, NULL, 'edge_node', 'unsynced', '[21.5]', 1, 'received_at')",
        [],
    )
    .unwrap();
    let reading_seq = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO publication_log(epoch, kind, reading_seq, created_at)
         VALUES('epoch-old', 'measurement', ?1, 1)",
        [reading_seq],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO target_registry(
             target_id, endpoint_url, credential_token, archive_responsible,
             schema_version, cursor_epoch, cursor_pub_seq, created_at
         ) VALUES('edge', 'https://edge.test.invalid', 'source-token', 1, 1, 'epoch-old', 1, 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE edge_node_activation
         SET state='active', edge_id='edge-candidate',
             activation_id='act-0123456789abcdef0123456789abcdef',
             ledger_epoch='epoch-old', discard_through_reading_seq=0,
             cleanup_through_reading_seq=0, request_json='{}', result_json='{}', activated_at=1
         WHERE singleton=1",
        [],
    )
    .unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    drop(conn);

    let passphrase =
        iotkit_core_recovery::BackupPassphrase::new("owner-only-test-passphrase".to_string());
    std::fs::write(&passphrase_path, b"owner-only-test-passphrase").unwrap();
    iotkit_core_recovery::configure_backup(
        &config_path,
        &iotkit_core_recovery::BackupConfig {
            schema_version: 1,
            database: source.clone(),
            destination: destination.path().to_path_buf(),
            staging_directory: backup_staging.path().to_path_buf(),
            passphrase_file: passphrase_path.clone(),
            expected_mount: iotkit_core_recovery::MountIdentity {
                mount_point: destination.path().to_path_buf(),
                source: "pending".into(),
                filesystem_type: "pending".into(),
                filesystem_id: "pending".into(),
            },
            freshness_seconds: 60,
            retention_count: 1,
        },
        iotkit_core_recovery::BackupConfigReplace::Refuse,
    )
    .unwrap();
    let manifest =
        iotkit_core_recovery::create_backup(&config_path, &passphrase, 1_725_000_000_000).unwrap();
    let artifact = destination.path().join(format!(
        "{}{}",
        manifest.backup_id,
        iotkit_core_recovery::NODE_BACKUP_SUFFIX
    ));
    assert!(artifact.exists());
    let source_conn = Connection::open(&source).unwrap();
    assert_eq!(
        source_conn
            .query_row(
                "SELECT state FROM edge_node_backup_attempts WHERE backup_id=?1",
                [&manifest.backup_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "success"
    );
    drop(source_conn);
    let receipt = iotkit_core_recovery::restore_candidate(
        &iotkit_core_recovery::RestoreRequest {
            input: artifact.clone(),
            live_database: source.clone(),
            candidate_database: path.to_path_buf(),
            staging_directory: staging.path().to_path_buf(),
            handoff: iotkit_core_recovery::RecoveryHandoff {
                schema_version: 1,
                recovery_id: "recovery-candidate".into(),
                edge_id: "edge-candidate".into(),
                edge_node_id: "candidate-node".into(),
                old_ledger_epoch: "epoch-old".into(),
                expected_backup_id: Some(manifest.backup_id.clone()),
                proposed_new_epoch: "epoch-new".into(),
                credential_generation: 1,
            },
        },
        &passphrase,
    )
    .unwrap();
    assert!(matches!(
        receipt.status,
        iotkit_core_recovery::RestoreStatus::DurablyFencedCandidate
    ));
    let candidate_conn = Connection::open(path).unwrap();
    assert!(matches!(
        iotkit_core_recovery::startup_mode(&candidate_conn).unwrap(),
        iotkit_core_recovery::RecoveryStartupMode::FencedCandidate { .. }
    ));
    assert_eq!(
        iotkit_core_ops::ownership_state(&candidate_conn).unwrap(),
        iotkit_core_ops::OwnershipState::LocalRecoveryRequired
    );
    assert_eq!(
        candidate_conn
            .query_row(
                "SELECT state FROM edge_node_recovery_candidate WHERE singleton=1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "durably_fenced_candidate"
    );
    assert!(
        candidate_conn
            .query_row("SELECT count(*) FROM publication_log", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
            > 0
    );
    drop(candidate_conn);
    for sidecar in [
        path.with_extension("db-wal"),
        path.with_extension("db-shm"),
        path.with_extension("db-journal"),
    ] {
        assert!(!sidecar.exists());
    }
    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(config_path).unwrap();
    std::fs::remove_file(passphrase_path).unwrap();
}

fn database_state(path: &Path) -> (Vec<u8>, Vec<Option<Vec<u8>>>) {
    let sidecars = [
        path.with_extension("db-wal"),
        path.with_extension("db-shm"),
        path.with_extension("db-journal"),
        iotkit_core_ops::database_initialization_marker_path(path),
    ];
    (
        std::fs::read(path).unwrap(),
        sidecars
            .iter()
            .map(|sidecar| std::fs::read(sidecar).ok())
            .collect(),
    )
}

fn assert_database_state_unchanged(path: &Path, before: &(Vec<u8>, Vec<Option<Vec<u8>>>)) {
    assert_eq!(std::fs::read(path).unwrap(), before.0);
    let sidecars = [
        path.with_extension("db-wal"),
        path.with_extension("db-shm"),
        path.with_extension("db-journal"),
        iotkit_core_ops::database_initialization_marker_path(path),
    ];
    for (sidecar, expected) in sidecars.iter().zip(&before.1) {
        assert_eq!(std::fs::read(sidecar).ok(), *expected, "{sidecar:?}");
    }
}

fn add_invalid_second_candidate_row(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "ignore_check_constraints", "ON")
        .unwrap();
    conn.execute(
        "INSERT INTO edge_node_recovery_candidate(
             singleton, state, recovery_id, candidate_instance_id, backup_id,
             source_database_length, source_database_sha256, artifact_length, artifact_sha256,
             edge_id, edge_node_id, old_ledger_epoch, proposed_new_epoch,
             credential_generation, handoff_schema_version, installed_at_ms
         ) VALUES(
             2, 'durably_fenced_candidate', 'recovery-second', 'candidate-second',
             'backup-second', 1,
             '0000000000000000000000000000000000000000000000000000000000000000', 1,
             '1111111111111111111111111111111111111111111111111111111111111111',
             'edge-candidate', 'candidate-node', 'epoch-old', 'epoch-new', 1, 1, 1
         )",
        [],
    )
    .unwrap();
}

fn launch_node(config: &Path) -> std::process::Output {
    launch_node_with_env(config, &[])
}

fn launch_node_with_env(config: &Path, env: &[(&str, &str)]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_iotkit-edge-node"));
    command
        .args(["--config", config.to_str().unwrap()])
        .env_remove("IOTKIT_DB_PATH")
        .env_remove("IOTKIT_CONFIG_PATH")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("iotkit-edge-node exceeded the 10 second test timeout");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn probe_listener_connections(address: std::net::SocketAddr) -> usize {
    if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
        1
    } else {
        0
    }
}

#[test]
fn fenced_candidate_exits_before_logging_or_starting_normal_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("candidate.db");
    let config = directory.path().join("iotkit.toml");
    let bind = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = bind.local_addr().unwrap();
    drop(bind);
    create_fenced_candidate(&database);
    let before = database_state(&database);
    std::fs::write(
        &config,
        format!(
            "[edge_node]\n db_path = {:?}\n health_json_path = {:?}\n\
             [adapters.bravepi]\n enabled = false\n port = {:?}\n\
             [api]\n enabled = true\n bind = {:?}\n edge_node_name = {:?}\n\
             [exit.mqtt]\n enabled = true\n host = {:?}\n port = 1883\n\
             password_file = {:?}\n allow_insecure = true\n",
            database,
            directory.path().join(format!("{SENTINEL}-health.json")),
            format!("{SENTINEL}-source"),
            address.to_string(),
            SENTINEL,
            format!("{SENTINEL}-host"),
            directory.path().join(format!("{SENTINEL}-password")),
        ),
    )
    .unwrap();

    let output = launch_node(&config);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!output.status.success(), "stdout={stdout}\nstderr={stderr}");
    assert!(
        stderr.contains("fenced recovery candidate"),
        "stderr={stderr}"
    );
    assert!(
        !stderr.contains("MQTT exit publisher started")
            && !stdout.contains("MQTT exit publisher started")
    );
    assert!(
        !stderr.contains("control-plane API started")
            && !stdout.contains("control-plane API started")
    );
    assert!(
        !stderr.contains("input adapter instance configured")
            && !stdout.contains("input adapter instance configured")
    );
    assert!(!stderr.contains(SENTINEL) && !stdout.contains(SENTINEL));
    assert_eq!(probe_listener_connections(address), 0);
    assert_database_state_unchanged(&database, &before);
}

#[test]
fn malformed_recovery_schema_fails_closed_before_migration_or_config_logging() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("malformed.db");
    let config = directory.path().join("iotkit.toml");
    let bind = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = bind.local_addr().unwrap();
    drop(bind);
    let conn = Connection::open(&database).unwrap();
    conn.execute_batch("CREATE TABLE edge_node_recovery_candidate (singleton INTEGER)")
        .unwrap();
    drop(conn);
    let before = database_state(&database);
    std::fs::write(
        &config,
        format!(
            "[edge_node]\n db_path = {:?}\n health_json_path = {:?}\n\
             [api]\n enabled = true\n bind = {:?}\n edge_node_name = {:?}\n",
            database,
            directory.path().join(format!("{SENTINEL}-health.json")),
            address.to_string(),
            SENTINEL,
        ),
    )
    .unwrap();

    let output = launch_node(&config);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(3),
        "stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stderr.contains("Edge Node recovery startup state is invalid"),
        "stderr={stderr}"
    );
    assert!(!stderr.contains(SENTINEL) && !stdout.contains(SENTINEL));
    assert_eq!(probe_listener_connections(address), 0);
    assert_database_state_unchanged(&database, &before);
}

#[test]
fn malformed_recovery_row_fails_closed_without_repair_or_service_start() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("malformed-row.db");
    let config = directory.path().join("iotkit.toml");
    let bind = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = bind.local_addr().unwrap();
    drop(bind);
    create_fenced_candidate(&database);
    add_invalid_second_candidate_row(&database);
    let before = database_state(&database);
    std::fs::write(
        &config,
        format!(
            "[edge_node]\n db_path = {:?}\n health_json_path = {:?}\n\
             [api]\n enabled = true\n bind = {:?}\n edge_node_name = {:?}\n",
            database,
            directory.path().join(format!("{SENTINEL}-health.json")),
            address.to_string(),
            SENTINEL,
        ),
    )
    .unwrap();

    let output = launch_node(&config);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(3),
        "stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stderr.contains("Edge Node recovery startup state is invalid"),
        "stderr={stderr}"
    );
    assert!(!stderr.contains(SENTINEL) && !stdout.contains(SENTINEL));
    assert_eq!(probe_listener_connections(address), 0);
    assert_database_state_unchanged(&database, &before);
}

#[test]
fn rotated_recovery_authority_still_fails_closed_before_normal_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("authority-rotated.db");
    let config = directory.path().join("iotkit.toml");
    let bind = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = bind.local_addr().unwrap();
    drop(bind);
    create_fenced_candidate(&database);
    let conn = Connection::open(&database).unwrap();
    conn.execute(
        "UPDATE auth_state SET recovery_required = 0, ownership_ever_established = 0 WHERE id = 1",
        [],
    )
    .unwrap();
    drop(conn);
    let before = database_state(&database);
    std::fs::write(
        &config,
        format!(
            "[edge_node]\n db_path = {:?}\n health_json_path = {:?}\n\
             [api]\n enabled = true\n bind = {:?}\n edge_node_name = {:?}\n",
            database,
            directory.path().join(format!("{SENTINEL}-health.json")),
            address.to_string(),
            SENTINEL,
        ),
    )
    .unwrap();

    let output = launch_node(&config);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(3),
        "stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stderr.contains("fenced recovery candidate"),
        "stderr={stderr}"
    );
    assert!(!stderr.contains(SENTINEL) && !stdout.contains(SENTINEL));
    assert_eq!(probe_listener_connections(address), 0);
    assert_database_state_unchanged(&database, &before);
}

#[test]
fn fenced_candidate_precedes_invalid_adapter_catalog_validation() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("invalid-adapter.db");
    let config = directory.path().join("iotkit.toml");
    let bind = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = bind.local_addr().unwrap();
    drop(bind);
    create_fenced_candidate(&database);
    let before = database_state(&database);
    std::fs::write(
        &config,
        format!(
            "[edge_node]\n db_path = {:?}\n health_json_path = {:?}\n\
             [adapters.instances.sentinel]\n type = \"unknown-adapter\"\n enabled = true\n\
             config_schema_version = 1\n source = \"{SENTINEL}-source\"\n\
             [api]\n enabled = true\n bind = {:?}\n edge_node_name = {:?}\n",
            database,
            directory.path().join(format!("{SENTINEL}-health.json")),
            address.to_string(),
            SENTINEL,
        ),
    )
    .unwrap();

    let output = launch_node(&config);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(3),
        "stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stderr.contains("fenced recovery candidate"),
        "stderr={stderr}"
    );
    assert!(!stderr.contains(SENTINEL) && !stdout.contains(SENTINEL));
    assert_eq!(probe_listener_connections(address), 0);
    assert_database_state_unchanged(&database, &before);
}

#[test]
fn fenced_candidate_precedes_unrelated_toml_and_environment_errors() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("invalid-unrelated.db");
    let config = directory.path().join("iotkit.toml");
    create_fenced_candidate(&database);
    let before = database_state(&database);
    std::fs::write(
        &config,
        format!(
            "[edge_node]\n db_path = {:?}\n\n[unknown]\n secret = {:?}\n",
            database, SENTINEL
        ),
    )
    .unwrap();

    let output = launch_node_with_env(&config, &[("IOTKIT_API_ENABLED", "not-a-bool")]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(3),
        "stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stderr.contains("fenced recovery candidate"),
        "stderr={stderr}"
    );
    assert!(!stderr.contains(SENTINEL) && !stdout.contains(SENTINEL));
    assert_database_state_unchanged(&database, &before);
}

#[test]
fn fenced_candidate_uses_canonical_path_before_unrelated_malformed_multiline() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("malformed-multiline.db");
    let config = directory.path().join("iotkit.toml");
    create_fenced_candidate(&database);
    std::fs::write(
        &config,
        format!(
            "[edge_node]\ndb_path = {:?}\n[unrelated]\nbroken = \"\"\"\ndb_path = {:?}\n",
            database, SENTINEL
        ),
    )
    .unwrap();

    let output = launch_node(&config);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(3),
        "stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stderr.contains("fenced recovery candidate"),
        "stderr={stderr}"
    );
    assert!(!stderr.contains(SENTINEL) && !stdout.contains(SENTINEL));
}

#[test]
fn fenced_candidate_path_probe_supports_inline_quoted_and_dotted_forms() {
    fn inline_document(database: &Path) -> String {
        format!("edge_node = {{ db_path = {:?} }}\n", database)
    }
    fn dotted_document(database: &Path) -> String {
        format!("edge_node.db_path = {:?}\n", database)
    }
    fn quoted_document(database: &Path) -> String {
        format!("[edge_node]\ndb_path = {:?}\n", database)
    }
    let forms = [
        ("inline", inline_document as fn(&Path) -> String),
        ("dotted", dotted_document as fn(&Path) -> String),
        ("quoted", quoted_document as fn(&Path) -> String),
    ];
    for (name, document) in forms {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join(format!("{name}-candidate.db"));
        let config = directory.path().join("iotkit.toml");
        create_fenced_candidate(&database);
        std::fs::write(&config, document(&database)).unwrap();
        let output = launch_node(&config);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            output.status.code(),
            Some(3),
            "{name}: stdout={stdout}\nstderr={stderr}"
        );
        assert!(
            stderr.contains("fenced recovery candidate"),
            "{name}: stderr={stderr}"
        );
        assert!(!stderr.contains(SENTINEL) && !stdout.contains(SENTINEL));
    }
}

#[test]
fn malformed_recovery_precedes_unrelated_toml_and_environment_errors() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("malformed-unrelated.db");
    let config = directory.path().join("iotkit.toml");
    let conn = Connection::open(&database).unwrap();
    conn.execute_batch("CREATE TABLE edge_node_recovery_candidate (singleton INTEGER)")
        .unwrap();
    drop(conn);
    let before = database_state(&database);
    std::fs::write(
        &config,
        format!(
            "[edge_node]\n db_path = {:?}\n\n[unknown]\n secret = {:?}\n",
            database, SENTINEL
        ),
    )
    .unwrap();

    let output = launch_node_with_env(&config, &[("IOTKIT_API_ENABLED", "not-a-bool")]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(3),
        "stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stderr.contains("Edge Node recovery startup state is invalid"),
        "stderr={stderr}"
    );
    assert!(!stderr.contains(SENTINEL) && !stdout.contains(SENTINEL));
    assert_database_state_unchanged(&database, &before);
}

#[test]
fn candidate_delete_attempt_cannot_clear_startup_fence() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("delete-fenced.db");
    let config = directory.path().join("iotkit.toml");
    create_fenced_candidate(&database);
    let before = database_state(&database);
    let conn = Connection::open(&database).unwrap();
    assert!(
        conn.execute(
            "DELETE FROM edge_node_recovery_candidate WHERE singleton = 1",
            [],
        )
        .is_err()
    );
    drop(conn);
    std::fs::write(&config, format!("[edge_node]\n db_path = {:?}\n", database)).unwrap();

    let output = launch_node(&config);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(3), "stderr={stderr}");
    assert!(
        stderr.contains("fenced recovery candidate"),
        "stderr={stderr}"
    );
    assert_database_state_unchanged(&database, &before);
}
