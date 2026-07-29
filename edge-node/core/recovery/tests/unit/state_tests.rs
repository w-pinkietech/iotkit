use super::*;
use crate::all_edge_node_migrations;
use rusqlite::Connection;
use tempfile::tempdir;

const CANDIDATE_COLUMNS: &[&str] = &[
    "singleton",
    "state",
    "recovery_id",
    "candidate_instance_id",
    "backup_id",
    "source_database_length",
    "source_database_sha256",
    "artifact_length",
    "artifact_sha256",
    "edge_id",
    "edge_node_id",
    "old_ledger_epoch",
    "proposed_new_epoch",
    "credential_generation",
    "handoff_schema_version",
    "installed_at_ms",
];

fn install_candidate(conn: &Connection) {
    conn.execute(
        "INSERT INTO edge_node_recovery_candidate(
             singleton, state, recovery_id, candidate_instance_id, backup_id,
             source_database_length, source_database_sha256, artifact_length, artifact_sha256, edge_id,
             edge_node_id, old_ledger_epoch, proposed_new_epoch, credential_generation,
             handoff_schema_version, installed_at_ms
         ) VALUES(1, 'durably_fenced_candidate', 'recovery-1', 'candidate-1',
             'backup-1', 1, '0000000000000000000000000000000000000000000000000000000000000000',
             1, '1111111111111111111111111111111111111111111111111111111111111111',
             'edge-1', 'node-1', 'epoch-old', 'epoch-new', 0, 1, 1)",
        [],
    )
    .unwrap();
}

#[test]
fn recovery_migration_defaults_to_normal_without_a_candidate_row() {
    let conn = crate::tests_support::complete_database();
    assert!(recovery_schema_is_exact(&conn).unwrap());
    assert_eq!(startup_mode(&conn).unwrap(), RecoveryStartupMode::Normal);
    crate::tests_support::assert_table_columns(
        &conn,
        "edge_node_recovery_candidate",
        CANDIDATE_COLUMNS,
    );
}

#[test]
fn complete_migration_set_is_sorted_and_ends_at_recovery_version_23() {
    let migrations = all_edge_node_migrations();
    assert_eq!(
        migrations.last().map(|migration| migration.version),
        Some(23)
    );
    assert!(
        migrations
            .windows(2)
            .all(|pair| pair[0].version < pair[1].version)
    );
}

#[test]
fn backup_attempt_schema_accepts_only_forward_terminal_transitions() {
    let conn = crate::tests_support::complete_database();
    assert!(
        conn.execute(
            "INSERT INTO edge_node_backup_attempts(
                 attempt_id, backup_id, state, reason_code, artifact_name, artifact_length,
                 edge_node_id, ledger_epoch, accepted_cursor, allocation_high_water,
                 started_at_ms, artifact_created_at_ms, completed_at_ms
             ) VALUES('attempt-direct-success', 'backup-direct-success', 'success', 'ok',
                 'artifact-direct-success', 1, 'node-1', 'epoch-1', 2, 3, 1, 1, 1)",
            [],
        )
        .is_err()
    );
    conn.execute(
        "INSERT INTO edge_node_backup_attempts(
             attempt_id, backup_id, state, artifact_name, edge_node_id, started_at_ms
         ) VALUES('attempt-1', 'backup-1', 'started', 'artifact-1', 'node-1', 1)",
        [],
    )
    .unwrap();
    assert!(
        conn.execute(
            "UPDATE edge_node_backup_attempts SET state = 'started' WHERE attempt_id = 'attempt-1'",
            [],
        )
        .is_err()
    );
    conn.execute(
        "UPDATE edge_node_backup_attempts SET state = 'success', reason_code = 'ok',
             artifact_length = 1, ledger_epoch = 'epoch-1', accepted_cursor = 2,
             allocation_high_water = 3, artifact_created_at_ms = 4, completed_at_ms = 5
         WHERE attempt_id = 'attempt-1'",
        [],
    )
    .unwrap();
    assert!(
        conn.execute(
            "UPDATE edge_node_backup_attempts SET state = 'success', reason_code = 'ok',
                 artifact_name = 'artifact-conflict', artifact_length = 1, ledger_epoch = 'epoch-1',
                 accepted_cursor = 2, allocation_high_water = 3, artifact_created_at_ms = 4,
                 completed_at_ms = 5 WHERE attempt_id = 'attempt-1'",
            [],
        )
        .is_err()
    );
    assert!(conn.execute(
        "UPDATE edge_node_backup_attempts SET completed_at_ms = 6 WHERE attempt_id = 'attempt-1'",
        [],
    ).is_err());
    assert!(
        conn.execute(
            "DELETE FROM edge_node_backup_attempts WHERE attempt_id = 'attempt-1'",
            [],
        )
        .is_err()
    );
    conn.execute(
        "INSERT INTO edge_node_backup_attempts(
             attempt_id, backup_id, state, reason_code, artifact_name, edge_node_id,
             started_at_ms, completed_at_ms
         ) VALUES('attempt-2', 'backup-2', 'failed', 'preflight_failed', 'artifact-2', 'node-1', 1, 2)",
        [],
    )
    .unwrap();
}

#[test]
fn valid_candidate_is_reported_as_fenced_without_exposing_node_identity() {
    let conn = crate::tests_support::complete_database();
    install_candidate(&conn);
    assert_eq!(
        startup_mode(&conn).unwrap(),
        RecoveryStartupMode::FencedCandidate {
            recovery_id: "recovery-1".into(),
            candidate_instance_id: "candidate-1".into(),
            backup_id: Some("backup-1".into()),
            edge_id: "edge-1".into(),
            old_ledger_epoch: "epoch-old".into(),
            proposed_new_epoch: "epoch-new".into(),
            credential_generation: 0,
        }
    );
}

#[test]
fn candidate_provenance_requires_all_fields_or_all_null() {
    let conn = crate::tests_support::complete_database();
    assert!(conn
        .execute(
            "INSERT INTO edge_node_recovery_candidate(
                 singleton, state, recovery_id, candidate_instance_id, backup_id,
                 source_database_length, source_database_sha256, edge_id, edge_node_id,
                 old_ledger_epoch, proposed_new_epoch, credential_generation,
                 handoff_schema_version, installed_at_ms
             ) VALUES(1, 'durably_fenced_candidate', 'recovery-mixed', 'candidate-mixed',
                 'backup-mixed', 1, '0000000000000000000000000000000000000000000000000000000000000000',
                 'edge-1', 'node-1', 'epoch-old', 'epoch-new', 0, 1, 1)",
            [],
        )
        .is_err());

    let conn = crate::tests_support::complete_database();
    conn.execute(
        "INSERT INTO edge_node_recovery_candidate(
             singleton, state, recovery_id, candidate_instance_id, backup_id,
             source_database_length, source_database_sha256, artifact_length,
             artifact_sha256, edge_id, edge_node_id, old_ledger_epoch,
             proposed_new_epoch, credential_generation, handoff_schema_version,
             installed_at_ms
         ) VALUES(1, 'durably_fenced_candidate', 'recovery-empty', 'candidate-empty',
             NULL, NULL, NULL, NULL, NULL, 'edge-1', 'node-1', 'epoch-old',
             'epoch-new', 0, 1, 1)",
        [],
    )
    .unwrap();
    assert!(matches!(
        startup_mode(&conn).unwrap(),
        RecoveryStartupMode::FencedCandidate {
            backup_id: None,
            ..
        }
    ));
}

#[test]
fn missing_new_and_pre_recovery_paths_are_normal_without_creation_or_migration() {
    let directory = tempdir().unwrap();
    let missing = directory.path().join("missing.db");
    assert_eq!(
        probe_startup_path(&missing).unwrap(),
        RecoveryStartupMode::Normal
    );
    assert!(!missing.exists());

    let fresh = directory.path().join("fresh.db");
    Connection::open(&fresh).unwrap();
    assert_eq!(
        probe_startup_path(&fresh).unwrap(),
        RecoveryStartupMode::Normal
    );
    let fresh_conn = Connection::open(&fresh).unwrap();
    assert_eq!(
        fresh_conn
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name = 'edge_node_recovery_candidate'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0,
    );

    let old = directory.path().join("old.db");
    let old_conn = Connection::open(&old).unwrap();
    iotkit_core_storage::run_migrations(
        &old_conn,
        &crate::tests_support::pre_recovery_migrations(),
    )
    .unwrap();
    drop(old_conn);
    assert_eq!(
        probe_startup_path(&old).unwrap(),
        RecoveryStartupMode::Normal
    );
    let old_conn = Connection::open(&old).unwrap();
    assert_eq!(
        old_conn
            .query_row(
                "SELECT count(*) FROM _schema_version WHERE version = 23",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0,
    );
}

#[test]
fn present_malformed_recovery_schema_or_row_fails_closed_without_repair() {
    let directory = tempdir().unwrap();
    let malformed = directory.path().join("malformed.db");
    let conn = Connection::open(&malformed).unwrap();
    conn.execute_batch("CREATE TABLE edge_node_recovery_candidate (singleton INTEGER)")
        .unwrap();
    drop(conn);
    assert_eq!(
        probe_startup_path(&malformed),
        Err(RecoveryError::InvalidStartupState)
    );
    let conn = Connection::open(&malformed).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'edge_node_backup_attempts'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0,
    );

    let conn = crate::tests_support::complete_database();
    conn.pragma_update(None, "ignore_check_constraints", "ON")
        .unwrap();
    conn.execute(
        "INSERT INTO edge_node_recovery_candidate(
             singleton, state, recovery_id, candidate_instance_id, backup_id, edge_id,
             edge_node_id, old_ledger_epoch, proposed_new_epoch, credential_generation,
             handoff_schema_version, installed_at_ms
         ) VALUES(1, 'normal', 'recovery-1', 'candidate-1', NULL, 'edge-1', 'node-1',
             'epoch-old', 'epoch-new', 0, 1, 1)",
        [],
    )
    .unwrap();
    assert_eq!(startup_mode(&conn), Err(RecoveryError::InvalidStartupState));

    let conn = crate::tests_support::complete_database();
    conn.execute(
        "INSERT INTO edge_node_backup_attempts(
             attempt_id, backup_id, state, artifact_name, edge_node_id, started_at_ms
         ) VALUES('attempt-invalid', 'backup-invalid', 'started', 'artifact-invalid', 'node-1', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE edge_node_backup_attempts
         SET state = 'success', reason_code = 'ok', artifact_length = 1, ledger_epoch = 'epoch-1',
             accepted_cursor = 0, allocation_high_water = -1, artifact_created_at_ms = 1,
             completed_at_ms = 1
         WHERE attempt_id = 'attempt-invalid'",
        [],
    )
    .unwrap();
    assert_eq!(startup_mode(&conn), Err(RecoveryError::InvalidStartupState));

    let conn = crate::tests_support::complete_database();
    conn.execute_batch("DROP TRIGGER edge_node_backup_attempts_forward_only")
        .unwrap();
    assert_eq!(startup_mode(&conn), Err(RecoveryError::InvalidStartupState));
}

#[test]
fn probe_rejects_weakened_v23_trigger_even_when_old_fragments_remain_in_comments() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("weakened.db");
    let conn = Connection::open(&database).unwrap();
    iotkit_core_storage::run_migrations(&conn, &all_edge_node_migrations()).unwrap();
    conn.pragma_update(None, "writable_schema", "ON").unwrap();
    conn.execute(
        "UPDATE sqlite_schema
         SET sql = 'CREATE TRIGGER edge_node_backup_attempts_forward_only
                    BEFORE UPDATE ON edge_node_backup_attempts
                    BEGIN SELECT 1; END
                    /* WHEN NOT (OLD.state = ''started'' AND NEW.state IN (''success'', ''failed''))
                       BEGIN SELECT RAISE(ABORT, ''backup attempt transition is not allowed''); END */'
         WHERE type = 'trigger' AND name = 'edge_node_backup_attempts_forward_only'",
        [],
    )
    .unwrap();
    conn.pragma_update(None, "writable_schema", "OFF").unwrap();
    drop(conn);

    assert_eq!(
        probe_startup_path(&database),
        Err(RecoveryError::InvalidStartupState)
    );
}

#[test]
fn probe_rejects_weakened_v23_table_even_when_old_fragments_remain_in_comments() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("weakened-table.db");
    let conn = Connection::open(&database).unwrap();
    iotkit_core_storage::run_migrations(&conn, &all_edge_node_migrations()).unwrap();
    conn.pragma_update(None, "writable_schema", "ON").unwrap();
    conn.execute(
        "UPDATE sqlite_schema
         SET sql = 'CREATE TABLE edge_node_recovery_candidate(
             singleton INTEGER PRIMARY KEY CHECK(singleton = 1), state TEXT NOT NULL,
             recovery_id TEXT NOT NULL, candidate_instance_id TEXT NOT NULL UNIQUE, backup_id TEXT,
             edge_id TEXT NOT NULL, edge_node_id TEXT NOT NULL, old_ledger_epoch TEXT NOT NULL,
             proposed_new_epoch TEXT NOT NULL, credential_generation INTEGER NOT NULL,
             handoff_schema_version INTEGER NOT NULL, installed_at_ms INTEGER NOT NULL
         ) /* state TEXT NOT NULL CHECK(state = ''durably_fenced_candidate'')
              candidate_instance_id TEXT NOT NULL UNIQUE
              credential_generation INTEGER NOT NULL CHECK(credential_generation >= 0)
              handoff_schema_version INTEGER NOT NULL CHECK(handoff_schema_version = 1) */'
         WHERE type = 'table' AND name = 'edge_node_recovery_candidate'",
        [],
    )
    .unwrap();
    conn.pragma_update(None, "writable_schema", "OFF").unwrap();
    drop(conn);

    assert_eq!(
        probe_startup_path(&database),
        Err(RecoveryError::InvalidStartupState)
    );
}
