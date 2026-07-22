use super::*;

fn migrated_conn() -> rusqlite::Connection {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    all.sort_by_key(|m| m.version);

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    iotkit_core_storage::run_migrations(&conn, &all).unwrap();
    conn
}

fn epoch_start_count(conn: &rusqlite::Connection) -> i64 {
    conn.query_row(
            "SELECT count(*) FROM publication_log WHERE kind = 'annotation' AND subtype = 'epoch_start'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn epoch_start_row(conn: &rusqlite::Connection) -> Option<(i64, String, String)> {
    conn.query_row(
        "SELECT pub_seq, epoch, annotation_json
             FROM publication_log
             WHERE kind = 'annotation' AND subtype = 'epoch_start'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()
    .unwrap()
}

#[test]
fn first_boot_without_renew_does_not_enqueue() {
    let conn = migrated_conn();
    let _epoch = iotkit_core_ledger::ledger_epoch(&conn).unwrap();

    maybe_enqueue_epoch_start(&conn).unwrap();

    assert_eq!(epoch_start_count(&conn), 0);
}

#[test]
fn after_renew_enqueues_epoch_start_once_with_prior_epoch() {
    let conn = migrated_conn();
    let prior_epoch = iotkit_core_ledger::ledger_epoch(&conn).unwrap();
    let current_epoch = iotkit_core_ledger::renew_epoch(&conn).unwrap();

    maybe_enqueue_epoch_start(&conn).unwrap();
    maybe_enqueue_epoch_start(&conn).unwrap();

    assert_eq!(epoch_start_count(&conn), 1);
    let (pub_seq, row_epoch, annotation_json) = epoch_start_row(&conn).unwrap();
    assert_eq!(pub_seq, 1);
    assert_eq!(row_epoch, current_epoch);
    let payload: serde_json::Value = serde_json::from_str(&annotation_json).unwrap();
    assert_eq!(payload, serde_json::json!({ "prior_epoch": prior_epoch }));
}

#[test]
fn fresh_renew_null_old_epoch_does_not_enqueue() {
    let conn = migrated_conn();
    let _epoch = iotkit_core_ledger::renew_epoch(&conn).unwrap();

    maybe_enqueue_epoch_start(&conn).unwrap();

    assert_eq!(epoch_start_count(&conn), 0);
}

#[test]
fn discovery_only_skips_epoch_start_publication() {
    let conn = migrated_conn();
    let _prior_epoch = iotkit_core_ledger::ledger_epoch(&conn).unwrap();
    let _current_epoch = iotkit_core_ledger::renew_epoch(&conn).unwrap();
    iotkit_core_publish::activation::install_edge_target(
        &conn,
        &iotkit_core_publish::store::TargetRow {
            target_id: "edge".into(),
            endpoint_url: "mqtts://broker.example.test:8883".into(),
            credential_token: String::new(),
            archive_responsible: true,
            schema_version: 1,
            cursor_epoch: None,
            cursor_pub_seq: 0,
        },
        1,
    )
    .unwrap();

    maybe_enqueue_epoch_start(&conn).unwrap();

    assert_eq!(epoch_start_count(&conn), 0);
}
