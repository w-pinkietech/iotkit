use iotkit_core_storage::{Migration, run_migrations};
use std::path::Path;

use rusqlite::{Connection, params};

pub const SNAPSHOT_SENTINEL: &str = "sentinel-http-bearer-must-not-leave-source";
pub const TEST_EDGE_NODE_ID: &str = "edge-node-test";
pub const TEST_LEDGER_EPOCH: &str = "epoch-test";

pub fn snapshot_credential() -> String {
    format!("{SNAPSHOT_SENTINEL}{}", "x".repeat(8 * 1024))
}

pub fn complete_database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn, &crate::all_edge_node_migrations()).unwrap();
    conn
}

pub fn pre_recovery_migrations() -> Vec<Migration> {
    let mut migrations = crate::all_edge_node_migrations();
    migrations.retain(|migration| migration.version < 23);
    migrations
}

pub fn assert_table_columns(conn: &Connection, table: &str, expected: &[&str]) {
    let actual: Vec<String> = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(actual, expected);
}

pub fn active_database_with_publications(path: &Path, accepted: i64, allocated: i64) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    run_migrations(&conn, &crate::all_edge_node_migrations()).unwrap();
    conn.execute(
        "INSERT INTO ledger_meta(key, value) VALUES
             ('edge_node_id', ?1), ('epoch', ?2), ('generation', '1')",
        params![TEST_EDGE_NODE_ID, TEST_LEDGER_EPOCH],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO devices(
             system_id, hardware_id, user_label, kind, state, created_at
         ) VALUES(?1, 'fixture-device', 'Fixture device', 'individual', 'active', 1)",
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
    for seq in 1..=allocated {
        conn.execute(
            "INSERT INTO readings(
                 series_id, received_at, device_time, time_source, time_quality,
                 values_json, event_time, event_time_source
             ) VALUES(1, ?1, NULL, 'edge_node', 'unsynced', '[21.5]', ?1, 'received_at')",
            [seq],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO publication_log(epoch, kind, reading_seq, created_at)
             VALUES(?1, 'measurement', ?2, ?2)",
            params![TEST_LEDGER_EPOCH, seq],
        )
        .unwrap();
    }
    conn.execute(
        "DELETE FROM publication_log WHERE pub_seq <= ?1",
        [accepted],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO target_registry(
             target_id, endpoint_url, credential_token, archive_responsible,
             schema_version, cursor_epoch, cursor_pub_seq, created_at
         ) VALUES(
             'edge', 'https://edge.test.invalid', ?1, 1, 1, ?2, ?3, 1
         )",
        params![snapshot_credential(), TEST_LEDGER_EPOCH, accepted],
    )
    .unwrap();
    conn.execute(
        "UPDATE edge_node_activation
         SET state='active', edge_id='edge-test',
             activation_id='act-0123456789abcdef0123456789abcdef',
             ledger_epoch=?1, discard_through_reading_seq=0,
             cleanup_through_reading_seq=0,
             request_json='{}', result_json='{}', activated_at=1
         WHERE singleton=1",
        [TEST_LEDGER_EPOCH],
    )
    .unwrap();
    conn
}

pub fn insert_next_publication(path: &Path, pub_seq: i64) {
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    tx.execute(
        "INSERT INTO readings(
             series_id, received_at, device_time, time_source, time_quality,
             values_json, event_time, event_time_source
         ) VALUES(1, ?1, NULL, 'edge_node', 'unsynced', '[22.0]', ?1, 'received_at')",
        [pub_seq],
    )
    .unwrap();
    let reading_seq = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO publication_log(epoch, kind, reading_seq, created_at)
         VALUES(?1, 'measurement', ?2, ?3)",
        params![TEST_LEDGER_EPOCH, reading_seq, pub_seq],
    )
    .unwrap();
    tx.commit().unwrap();
}
