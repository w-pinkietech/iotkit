use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use rusqlite::Connection;

const SENTINEL: &str = "sentinel-sensitive-config";

fn create_fenced_candidate(path: &Path) {
    let conn = Connection::open(path).unwrap();
    iotkit_core_storage::run_migrations(&conn, &iotkit_core_recovery::all_edge_node_migrations())
        .unwrap();
    conn.execute(
        "INSERT INTO ledger_meta(key, value) VALUES
             ('edge_node_id', 'candidate-node'), ('epoch', 'epoch-old'), ('generation', '1')",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE auth_state SET recovery_required = 1, ownership_ever_established = 1 WHERE id = 1",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO edge_node_recovery_candidate(
             singleton, state, recovery_id, candidate_instance_id, backup_id,
             source_database_length, source_database_sha256, artifact_length, artifact_sha256,
             edge_id, edge_node_id, old_ledger_epoch, proposed_new_epoch,
             credential_generation, handoff_schema_version, installed_at_ms
         ) VALUES(
             1, 'durably_fenced_candidate', 'recovery-candidate', 'candidate-instance',
             'backup-candidate', 1,
             '0000000000000000000000000000000000000000000000000000000000000000', 1,
             '1111111111111111111111111111111111111111111111111111111111111111',
             'edge-candidate', 'candidate-node', 'epoch-old', 'epoch-new', 1, 1, 1
         )",
        [],
    )
    .unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    drop(conn);
    let marker = iotkit_core_ops::database_initialization_marker_path(path);
    std::fs::write(marker, b"iotkit-database-initialized-v1\n").unwrap();
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
    Command::new(env!("CARGO_BIN_EXE_iotkit-edge-node"))
        .args(["--config", config.to_str().unwrap()])
        .env_remove("IOTKIT_DB_PATH")
        .env_remove("IOTKIT_CONFIG_PATH")
        .output()
        .unwrap()
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
    let before = std::fs::read(&database).unwrap();
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
    assert_eq!(std::fs::read(&database).unwrap(), before);
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
    let before = std::fs::read(&database).unwrap();
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
    assert_eq!(std::fs::read(&database).unwrap(), before);
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
    let before = std::fs::read(&database).unwrap();
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
    assert_eq!(std::fs::read(&database).unwrap(), before);
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
    let before = std::fs::read(&database).unwrap();
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
    assert_eq!(std::fs::read(&database).unwrap(), before);
}
