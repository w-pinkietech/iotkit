use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

use super::*;
use crate::{
    BackupCounts, RecoveryError, SnapshotMode,
    tests_support::{
        SNAPSHOT_SENTINEL, TEST_LEDGER_EPOCH, active_database_with_publications,
        insert_next_publication,
    },
};

fn make_source(root: &Path) -> (PathBuf, Connection) {
    let path = root.join("source.db");
    let conn = active_database_with_publications(&path, 3, 5);
    (path, conn)
}

fn clone_valid_snapshot(root: &Path) -> PathBuf {
    let (source, source_conn) = make_source(root);
    drop(source_conn);
    let snapshot = root.join("snapshot.db");
    create_consistent_snapshot(&source, &snapshot, "node-backup-test", 1_725_000_000_000).unwrap();
    snapshot
}

fn expect_invalid(path: &Path) {
    assert_eq!(validate_snapshot(path), Err(RecoveryError::InvalidSnapshot));
}

#[test]
fn online_snapshot_is_self_consistent_while_source_advances() {
    let temp = tempdir().unwrap();
    let (source, source_conn) = make_source(temp.path());
    let snapshot = temp.path().join("snapshot.db");
    let writer_source = source.clone();
    let writer_snapshot = snapshot.clone();
    let writer = thread::spawn(move || {
        for _ in 0..2_000 {
            if fs::metadata(&writer_snapshot).is_ok_and(|metadata| metadata.len() > 0) {
                insert_next_publication(&writer_source, 6);
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("snapshot backup did not begin");
    });

    let artifact =
        create_consistent_snapshot(&source, &snapshot, "node-backup-test", 1_725_000_000_000)
            .unwrap();
    writer.join().unwrap();

    assert_eq!(artifact.manifest.accepted_cursor, 3);
    assert_eq!(artifact.manifest.allocation_high_water, 5);
    assert_eq!(artifact.manifest.snapshot_mode, SnapshotMode::Online);
    assert!(artifact.manifest.shutdown_seal_id.is_none());
    assert_eq!(artifact.path, snapshot);
    assert!(!snapshot.with_extension("db-wal").exists());
    assert!(
        source_conn
            .query_row("SELECT credential_token FROM target_registry", [], |row| {
                row.get::<_, String>(0)
            },)
            .unwrap()
            .starts_with(SNAPSHOT_SENTINEL)
    );
    assert_eq!(
        source_conn
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name='publication_log'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        6
    );
    let bytes = fs::read(&snapshot).unwrap();
    assert!(
        !bytes
            .windows(SNAPSHOT_SENTINEL.len())
            .any(|window| window == SNAPSHOT_SENTINEL.as_bytes())
    );
    let snapshot_conn =
        Connection::open_with_flags(&snapshot, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        snapshot_conn
            .query_row("SELECT credential_token FROM target_registry", [], |row| {
                row.get::<_, String>(0)
            },)
            .unwrap(),
        ""
    );
    let audit: String = snapshot_conn
        .query_row(
            "SELECT detail FROM ledger_events
             WHERE kind='r14_op'
             ORDER BY event_id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(audit.contains("recovery.snapshot.remove_deployment_credentials"));
    assert!(audit.contains("\"credential_token\":\"[REDACTED]\""));
    assert!(audit.contains("\"targets\":[]"));
    assert!(!audit.contains(SNAPSHOT_SENTINEL));
    assert_eq!(
        artifact.manifest.counts,
        BackupCounts {
            devices: 1,
            series: 1,
            readings: 5,
            publication_rows: 2,
            ingest_dedup_rows: 0,
            staged_readings: 0,
            quarantine_rows: 0,
            device_principals: 0,
            device_credentials: 0,
            activation_rows: 1,
            ledger_events: 1,
            audit_events: 1,
        }
    );
    let debug = format!("{artifact:?}");
    assert!(!debug.contains(SNAPSHOT_SENTINEL));
    assert!(!debug.contains("node-backup-test"));
    assert!(!debug.contains(snapshot.to_string_lossy().as_ref()));
}

#[test]
fn snapshot_creation_refuses_to_overwrite_an_existing_path() {
    let temp = tempdir().unwrap();
    let (source, source_conn) = make_source(temp.path());
    let snapshot = temp.path().join("snapshot.db");
    fs::write(&snapshot, b"keep-existing").unwrap();

    assert_eq!(
        create_consistent_snapshot(&source, &snapshot, "node-backup-test", 1),
        Err(RecoveryError::Storage)
    );
    assert_eq!(fs::read(&snapshot).unwrap(), b"keep-existing");
    assert!(
        source_conn
            .query_row("SELECT credential_token FROM target_registry", [], |row| {
                row.get::<_, String>(0)
            },)
            .unwrap()
            .starts_with(SNAPSHOT_SENTINEL)
    );
}

#[test]
fn validation_rejects_activation_epoch_different_from_ledger_epoch() {
    let temp = tempdir().unwrap();
    let snapshot = clone_valid_snapshot(temp.path());
    let conn = Connection::open(&snapshot).unwrap();
    conn.execute(
        "UPDATE edge_node_activation SET ledger_epoch='different-epoch' WHERE singleton=1",
        [],
    )
    .unwrap();
    drop(conn);
    expect_invalid(&snapshot);
}

#[test]
fn validation_rejects_cursor_past_allocation_high_water() {
    let temp = tempdir().unwrap();
    let snapshot = clone_valid_snapshot(temp.path());
    let conn = Connection::open(&snapshot).unwrap();
    conn.execute(
        "UPDATE target_registry SET cursor_pub_seq=6 WHERE target_id='edge'",
        [],
    )
    .unwrap();
    drop(conn);
    expect_invalid(&snapshot);
}

#[test]
fn validation_rejects_gap_in_unacknowledged_publication_range() {
    let temp = tempdir().unwrap();
    let snapshot = clone_valid_snapshot(temp.path());
    let conn = Connection::open(&snapshot).unwrap();
    conn.execute("DELETE FROM publication_log WHERE pub_seq=4", [])
        .unwrap();
    drop(conn);
    expect_invalid(&snapshot);
}

#[test]
fn validation_rejects_measurement_without_its_reading() {
    let temp = tempdir().unwrap();
    let snapshot = clone_valid_snapshot(temp.path());
    let conn = Connection::open(&snapshot).unwrap();
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    conn.execute("DELETE FROM readings WHERE seq=4", [])
        .unwrap();
    drop(conn);
    expect_invalid(&snapshot);
}

#[test]
fn validation_rejects_any_nonempty_legacy_target_token() {
    let temp = tempdir().unwrap();
    let snapshot = clone_valid_snapshot(temp.path());
    let conn = Connection::open(&snapshot).unwrap();
    conn.execute(
        "UPDATE target_registry SET credential_token='unexpected-secret'",
        [],
    )
    .unwrap();
    drop(conn);
    expect_invalid(&snapshot);
    let diagnostic = format!("{:?}", validate_snapshot(&snapshot));
    assert!(!diagnostic.contains("unexpected-secret"));
    assert!(!diagnostic.contains(snapshot.to_string_lossy().as_ref()));
}

#[test]
fn validation_rejects_unknown_family_invalid_subtype_and_malformed_payload_json() {
    for (name, mutation) in [
        (
            "family",
            "UPDATE publication_log SET kind='unknown' WHERE pub_seq=4",
        ),
        (
            "subtype",
            "UPDATE publication_log
             SET kind='annotation', subtype='unknown', reading_seq=NULL,
                 annotation_json='{\"prior_epoch\":\"prior\"}'
             WHERE pub_seq=4",
        ),
        (
            "json",
            "UPDATE publication_log
             SET kind='commissioning_smoke', subtype=NULL, reading_seq=NULL,
                 annotation_json='{\"test_id\":'
             WHERE pub_seq=4",
        ),
    ] {
        let temp = tempdir().unwrap();
        let snapshot = clone_valid_snapshot(temp.path());
        let conn = Connection::open(&snapshot).unwrap();
        conn.execute_batch(mutation).unwrap();
        drop(conn);
        assert_eq!(
            validate_snapshot(&snapshot),
            Err(RecoveryError::InvalidSnapshot),
            "{name}"
        );
    }
}

#[test]
fn validation_accepts_every_closed_v1_record_family() {
    let temp = tempdir().unwrap();
    let snapshot = clone_valid_snapshot(temp.path());
    let conn = Connection::open(&snapshot).unwrap();
    conn.execute_batch(
        "UPDATE publication_log
         SET kind='annotation', subtype='epoch_start', reading_seq=NULL,
             annotation_json='{\"prior_epoch\":\"prior-epoch\"}'
         WHERE pub_seq=4;
         UPDATE publication_log
         SET kind='commissioning_smoke', subtype=NULL, reading_seq=NULL,
             annotation_json='{\"test_id\":\"smoke-0123456789abcdef0123456789abcdef\"}'
         WHERE pub_seq=5;",
    )
    .unwrap();
    drop(conn);

    let facts = validate_snapshot(&snapshot).unwrap();
    assert_eq!(facts.accepted_cursor, 3);
    assert_eq!(facts.allocation_high_water, 5);
}

#[test]
fn validation_rejects_wrong_target_schema_and_allocator_sequence() {
    for (name, mutation) in [
        (
            "target schema",
            "UPDATE target_registry SET schema_version=2",
        ),
        (
            "allocator",
            "UPDATE sqlite_sequence SET seq=6 WHERE name='publication_log'",
        ),
    ] {
        let temp = tempdir().unwrap();
        let snapshot = clone_valid_snapshot(temp.path());
        let conn = Connection::open(&snapshot).unwrap();
        conn.execute_batch(mutation).unwrap();
        drop(conn);
        assert_eq!(
            validate_snapshot(&snapshot),
            Err(RecoveryError::InvalidSnapshot),
            "{name}"
        );
    }
}

#[test]
fn validation_rejects_noncanonical_migration_rows_and_schema_objects() {
    for (name, mutation) in [
        (
            "migration label",
            "UPDATE _schema_version SET label='not-canonical' WHERE version=23",
        ),
        (
            "schema object",
            "CREATE TABLE unexpected_backup_state(value TEXT)",
        ),
    ] {
        let temp = tempdir().unwrap();
        let snapshot = clone_valid_snapshot(temp.path());
        let conn = Connection::open(&snapshot).unwrap();
        conn.execute_batch(mutation).unwrap();
        drop(conn);
        assert_eq!(
            validate_snapshot(&snapshot),
            Err(RecoveryError::InvalidSnapshot),
            "{name}"
        );
    }
}

#[test]
fn validation_rejects_a_snapshot_that_depends_on_wal_state() {
    let temp = tempdir().unwrap();
    let snapshot = clone_valid_snapshot(temp.path());
    let conn = Connection::open(&snapshot).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.execute(
        "UPDATE ledger_meta
         SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT)
         WHERE key='generation'",
        [],
    )
    .unwrap();
    assert!(snapshot.with_extension("db-wal").exists());

    assert_eq!(
        validate_snapshot(&snapshot),
        Err(RecoveryError::InvalidSnapshot)
    );
}

#[test]
fn validation_rejects_foreign_key_failure_and_failed_quick_check() {
    let temp = tempdir().unwrap();
    let snapshot = clone_valid_snapshot(temp.path());
    let conn = Connection::open(&snapshot).unwrap();
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    conn.execute("DELETE FROM series WHERE series_id=1", [])
        .unwrap();
    drop(conn);
    expect_invalid(&snapshot);

    let temp = tempdir().unwrap();
    let snapshot = clone_valid_snapshot(temp.path());
    let conn = Connection::open(&snapshot).unwrap();
    conn.pragma_update(None, "ignore_check_constraints", "ON")
        .unwrap();
    conn.execute(
        "UPDATE edge_node_activation SET state='invalid-check-state' WHERE singleton=1",
        [],
    )
    .unwrap();
    drop(conn);
    expect_invalid(&snapshot);
}

#[test]
fn online_manifest_uses_only_sanitized_snapshot_facts() {
    let temp = tempdir().unwrap();
    let (source, source_conn) = make_source(temp.path());
    let snapshot = temp.path().join("snapshot.db");
    let artifact =
        create_consistent_snapshot(&source, &snapshot, "node-backup-test", 1_725_000_000_000)
            .unwrap();

    assert_eq!(artifact.manifest.artifact_kind, "iotkit-node-backup");
    assert_eq!(artifact.manifest.backup_id, "node-backup-test");
    assert_eq!(artifact.manifest.created_at_ms, 1_725_000_000_000);
    assert_eq!(artifact.manifest.schema_version, 23);
    assert_eq!(artifact.manifest.snapshot_mode, SnapshotMode::Online);
    assert!(artifact.manifest.shutdown_seal_id.is_none());
    assert_eq!(
        artifact.manifest.database_length,
        fs::metadata(&snapshot).unwrap().len()
    );
    let expected_digest = format!("{:x}", Sha256::digest(fs::read(&snapshot).unwrap()));
    assert_eq!(artifact.manifest.database_sha256, expected_digest);
    assert_eq!(artifact.manifest.counts.audit_events, 1);
    assert_eq!(
        source_conn
            .query_row(
                "SELECT count(*) FROM ledger_events WHERE kind='r14_op'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert!(
        source_conn
            .query_row("SELECT credential_token FROM target_registry", [], |row| {
                row.get::<_, String>(0)
            },)
            .unwrap()
            .starts_with(SNAPSHOT_SENTINEL)
    );
    assert_eq!(artifact.manifest.ledger_epoch, TEST_LEDGER_EPOCH);
}
