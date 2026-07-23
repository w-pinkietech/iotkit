use iotkit_edge::storage::{
    AcceptBatch, RawRecord, Storage, StorageError, StorageProfile, StoredRawRecord,
};
use serde::Deserialize;
use sqlx::{
    Executor,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::path::PathBuf;
use tempfile::TempDir;

async fn sqlite_store() -> (TempDir, PathBuf, Storage) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).expect("create workspace test temp");
    let directory = TempDir::new_in(root).expect("temp directory");
    let database = directory.path().join("edge.db");
    let store = Storage::connect(StorageProfile::Sqlite {
        path: database.clone(),
    })
    .await
    .expect("open SQLite storage");
    (directory, database, store)
}

async fn postgres_store() -> Option<Storage> {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return None,
    };
    Some(
        Storage::connect(StorageProfile::Postgres { dsn })
            .await
            .expect("open PostgreSQL storage"),
    )
}

fn record(sequence: i64, value: i64) -> RawRecord {
    RawRecord::new(
        sequence,
        format!(
            "{{\"schema_version\":1,\"series_key\":\"bravepi-mainboard:temperature:0\",\
             \"event_time\":{},\"values\":{{\"temperature\":{value}}}}}",
            1_721_800_000_000_i64 + sequence
        ),
    )
    .expect("valid raw record")
}

fn batch(
    edge_node_id: impl Into<String>,
    ledger_epoch: impl Into<String>,
    records: Vec<RawRecord>,
) -> AcceptBatch {
    let edge_node_id = edge_node_id.into();
    let ledger_epoch = ledger_epoch.into();
    let start = records.first().map_or(0, |record| record.pub_seq);
    let end = records.last().map_or(0, |record| record.pub_seq);
    AcceptBatch {
        publication_id: format!("{edge_node_id}:{ledger_epoch}:{start}:{end}"),
        edge_node_id,
        ledger_epoch,
        received_at: 1_721_800_000_999,
        records,
    }
}

fn stored(records: &[RawRecord], publication_id: &str, received_at: i64) -> Vec<StoredRawRecord> {
    records
        .iter()
        .map(|record| StoredRawRecord {
            pub_seq: record.pub_seq,
            publication_id: publication_id.into(),
            record_json: record.record_json.clone(),
            received_at,
        })
        .collect()
}

#[derive(Deserialize)]
struct RecordBatchFixture {
    edge_node_id: String,
    ledger_epoch: String,
    publication_id: String,
    records: Vec<Box<serde_json::value::RawValue>>,
}

#[tokio::test]
async fn persists_the_wire_batch_publication_id_on_every_raw_row() {
    let (_directory, _database, store) = sqlite_store().await;
    let fixture: RecordBatchFixture =
        serde_json::from_slice(include_bytes!("../../testdata/egress/v1/record-batch.json"))
            .expect("decode shared record batch fixture");
    let records = fixture
        .records
        .iter()
        .enumerate()
        .map(|(index, record)| RawRecord::new(index as i64 + 1, record.get()))
        .collect::<Result<Vec<_>, _>>()
        .expect("convert fixture records");

    store
        .accept_batch(AcceptBatch {
            edge_node_id: fixture.edge_node_id.clone(),
            ledger_epoch: fixture.ledger_epoch.clone(),
            publication_id: fixture.publication_id.clone(),
            received_at: 1_721_800_000_999,
            records,
        })
        .await
        .expect("accept fixture batch");

    let stored = store
        .raw_records(&fixture.edge_node_id, &fixture.ledger_epoch)
        .await
        .expect("read fixture records");
    assert!(!stored.is_empty());
    assert!(
        stored
            .iter()
            .all(|record| record.publication_id == fixture.publication_id)
    );
}

#[tokio::test]
async fn accepts_a_contiguous_batch_and_advances_the_cursor_atomically() {
    let (_directory, _database, store) = sqlite_store().await;

    let result = store
        .accept_batch(batch(
            "edge-node-01",
            "epoch-01",
            vec![record(1, 20), record(2, 21)],
        ))
        .await
        .expect("accept batch");

    assert_eq!(result.accepted_through, 2);
    assert_eq!(
        store
            .accepted_through("edge-node-01", "epoch-01")
            .await
            .expect("read cursor"),
        2
    );
    assert_eq!(
        store
            .raw_records("edge-node-01", "epoch-01")
            .await
            .expect("read records"),
        stored(
            &[record(1, 20), record(2, 21)],
            "edge-node-01:epoch-01:1:2",
            1_721_800_000_999,
        )
    );
}

#[tokio::test]
async fn accepts_an_exact_replay_without_duplicating_records() {
    let (_directory, _database, store) = sqlite_store().await;
    let batch = batch(
        "edge-node-01",
        "epoch-01",
        vec![record(1, 20), record(2, 21)],
    );

    store
        .accept_batch(batch.clone())
        .await
        .expect("first batch");
    let cursor_before_replay = store
        .accepted_cursor("edge-node-01", "epoch-01")
        .await
        .expect("cursor before replay");
    let mut replay_batch = batch;
    replay_batch.received_at = 1;
    let replay = store
        .accept_batch(replay_batch)
        .await
        .expect("exact replay");

    assert_eq!(replay.accepted_through, 2);
    assert_eq!(
        store
            .raw_records("edge-node-01", "epoch-01")
            .await
            .expect("read records")
            .len(),
        2
    );
    assert_eq!(
        store
            .accepted_cursor("edge-node-01", "epoch-01")
            .await
            .expect("cursor after replay"),
        cursor_before_replay
    );
}

#[tokio::test]
async fn rejects_gaps_and_conflicts_without_changing_records_or_cursor() {
    let (_directory, _database, store) = sqlite_store().await;
    store
        .accept_batch(batch("edge-node-01", "epoch-01", vec![record(1, 20)]))
        .await
        .expect("first batch");

    let gap = store
        .accept_batch(batch("edge-node-01", "epoch-01", vec![record(3, 22)]))
        .await
        .expect_err("gap must fail");
    assert!(matches!(
        gap,
        StorageError::SequenceGap {
            expected: 2,
            actual: 3
        }
    ));

    let conflict = store
        .accept_batch(batch("edge-node-01", "epoch-01", vec![record(1, 999)]))
        .await
        .expect_err("conflict must fail");
    assert!(matches!(
        conflict,
        StorageError::RecordConflict { sequence: 1 }
    ));

    assert_eq!(
        store
            .accepted_through("edge-node-01", "epoch-01")
            .await
            .expect("read cursor"),
        1
    );
    assert_eq!(
        store
            .raw_records("edge-node-01", "epoch-01")
            .await
            .expect("read records"),
        stored(
            &[record(1, 20)],
            "edge-node-01:epoch-01:1:1",
            1_721_800_000_999,
        )
    );
}

#[tokio::test]
async fn refuses_an_empty_batch() {
    let (_directory, _database, store) = sqlite_store().await;
    let error = store
        .accept_batch(batch("edge-node-01", "epoch-01", vec![]))
        .await
        .expect_err("empty batch must fail");
    assert!(matches!(error, StorageError::InvalidRecord(_)));
}

#[tokio::test]
async fn preserves_go_compatible_compacted_raw_bytes_for_replay_identity() {
    let (_directory, _database, store) = sqlite_store().await;
    let spaced = RawRecord::new(
        1,
        br#"{ "z": 1.0, "message": "space stays here", "a": [ 1, 2 ] }"#,
    )
    .expect("valid spaced JSON");
    let compact = RawRecord::new(1, br#"{"z":1.0,"message":"space stays here","a":[1,2]}"#)
        .expect("valid compact JSON");
    assert_eq!(spaced.record_json, compact.record_json);

    let batch = |record| batch("edge-node-01", "epoch-01", vec![record]);
    store
        .accept_batch(batch(spaced))
        .await
        .expect("first batch");
    store
        .accept_batch(batch(compact))
        .await
        .expect("whitespace-only replay");

    let numeric_change = RawRecord::new(1, br#"{"z":1,"message":"space stays here","a":[1,2]}"#)
        .expect("valid changed JSON");
    assert!(matches!(
        store
            .accept_batch(batch(numeric_change))
            .await
            .expect_err("numeric representation changes the raw record"),
        StorageError::RecordConflict { sequence: 1 }
    ));
}

#[tokio::test]
async fn rolls_back_raw_rows_when_cursor_persistence_fails() {
    let (_directory, database, store) = sqlite_store().await;
    let fault_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(database)
                .create_if_missing(false),
        )
        .await
        .expect("open fault connection");
    fault_pool
        .execute(
            "CREATE TRIGGER fail_cursor BEFORE INSERT ON accepted_cursors \
             BEGIN SELECT RAISE(ABORT, 'injected cursor failure'); END",
        )
        .await
        .expect("install fault");

    store
        .accept_batch(batch("edge-node-01", "epoch-01", vec![record(1, 20)]))
        .await
        .expect_err("cursor persistence fault must fail the transaction");

    assert!(
        store
            .raw_records("edge-node-01", "epoch-01")
            .await
            .expect("read records")
            .is_empty()
    );
    assert_eq!(
        store
            .accepted_through("edge-node-01", "epoch-01")
            .await
            .expect("read cursor"),
        0
    );
}

#[tokio::test]
async fn prevents_two_sqlite_process_owners_and_handles_special_paths() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).expect("create workspace test temp");
    let directory = TempDir::new_in(root).expect("temp directory");
    let database = directory.path().join("edge #1.db");
    let profile = StorageProfile::Sqlite {
        path: database.clone(),
    };
    let first = Storage::connect(profile.clone())
        .await
        .expect("open special SQLite path");
    let error = match Storage::connect(profile).await {
        Ok(_) => panic!("second SQLite owner must fail"),
        Err(error) => error,
    };
    assert!(matches!(error, StorageError::AlreadyInUse));
    drop(first);
    Storage::connect(StorageProfile::Sqlite { path: database })
        .await
        .expect("guard is released when owner closes");
}

#[tokio::test]
async fn refuses_a_go_era_schema_without_mutating_it() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).expect("create workspace test temp");
    let directory = TempDir::new_in(root).expect("temp directory");
    let database = directory.path().join("legacy.db");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&database)
                .create_if_missing(true),
        )
        .await
        .expect("open legacy database");
    pool.execute("CREATE TABLE raw_records(legacy INTEGER NOT NULL)")
        .await
        .expect("create legacy marker");
    pool.close().await;

    let error = match Storage::connect(StorageProfile::Sqlite {
        path: database.clone(),
    })
    .await
    {
        Ok(_) => panic!("legacy schema must fail"),
        Err(error) => error,
    };
    assert!(matches!(error, StorageError::UnsupportedLegacySchema));

    let inspection = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(database))
        .await
        .expect("reopen legacy database");
    let migration_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(&inspection)
    .await
    .expect("inspect migration table");
    assert_eq!(migration_table_count, 0);
}

#[test]
fn postgres_profile_debug_output_redacts_credentials() {
    let profile = StorageProfile::Postgres {
        dsn: "postgres://iotkit:secret@example.test/iotkit".into(),
    };
    let debug = format!("{profile:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("secret"));
}

#[tokio::test]
async fn postgres_obeys_the_same_raw_custody_contract_when_configured() {
    let Some(store) = postgres_store().await else {
        return;
    };
    let dsn = std::env::var("IOTKIT_TEST_POSTGRES_DSN").expect("configured PostgreSQL DSN");
    let second = match Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() }).await {
        Ok(_) => panic!("second PostgreSQL owner must fail"),
        Err(error) => error,
    };
    assert!(matches!(second, StorageError::AlreadyInUse));

    let accepted_batch = batch(
        format!("edge-node-{}", uuid::Uuid::new_v4()),
        "epoch-01",
        vec![record(1, 20), record(2, 21)],
    );

    let accepted = store
        .accept_batch(accepted_batch.clone())
        .await
        .expect("accept PostgreSQL batch");
    let cursor_before_replay = store
        .accepted_cursor(&accepted_batch.edge_node_id, &accepted_batch.ledger_epoch)
        .await
        .expect("PostgreSQL cursor before replay");
    let mut replay_batch = accepted_batch.clone();
    replay_batch.received_at = 1;
    let replay = store
        .accept_batch(replay_batch)
        .await
        .expect("replay PostgreSQL batch");

    assert_eq!(accepted.accepted_through, 2);
    assert_eq!(replay.accepted_through, 2);
    assert_eq!(
        store
            .accepted_cursor(&accepted_batch.edge_node_id, &accepted_batch.ledger_epoch,)
            .await
            .expect("PostgreSQL cursor after replay"),
        cursor_before_replay
    );
    assert_eq!(
        store
            .raw_records(&accepted_batch.edge_node_id, &accepted_batch.ledger_epoch,)
            .await
            .expect("read PostgreSQL records"),
        stored(
            &accepted_batch.records,
            &accepted_batch.publication_id,
            accepted_batch.received_at,
        )
    );

    let gap = store
        .accept_batch(batch(
            accepted_batch.edge_node_id.clone(),
            accepted_batch.ledger_epoch.clone(),
            vec![record(4, 22)],
        ))
        .await
        .expect_err("PostgreSQL gap must fail");
    assert!(matches!(
        gap,
        StorageError::SequenceGap {
            expected: 3,
            actual: 4
        }
    ));
    let conflict = store
        .accept_batch(batch(
            accepted_batch.edge_node_id.clone(),
            accepted_batch.ledger_epoch.clone(),
            vec![record(1, 999)],
        ))
        .await
        .expect_err("PostgreSQL conflict must fail");
    assert!(matches!(
        conflict,
        StorageError::RecordConflict { sequence: 1 }
    ));

    let concurrent_edge = format!("edge-node-{}", uuid::Uuid::new_v4());
    let first = batch(concurrent_edge.clone(), "epoch-01", vec![record(1, 20)]);
    let extended = batch(
        concurrent_edge.clone(),
        "epoch-01",
        vec![record(1, 20), record(2, 21)],
    );
    let (short_result, extended_result) =
        tokio::join!(store.accept_batch(first), store.accept_batch(extended));
    short_result.expect("concurrent short batch");
    extended_result.expect("concurrent extended batch");
    assert_eq!(
        store
            .accepted_through(&concurrent_edge, "epoch-01")
            .await
            .expect("read concurrent cursor"),
        2
    );

    let fault_pool = sqlx::PgPool::connect(&dsn)
        .await
        .expect("open PostgreSQL fault connection");
    fault_pool
        .execute(
            "CREATE OR REPLACE FUNCTION iotkit_test_fail_cursor() RETURNS trigger \
             LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected cursor failure'; END $$",
        )
        .await
        .expect("create PostgreSQL fault function");
    fault_pool
        .execute(
            "CREATE TRIGGER iotkit_test_fail_cursor BEFORE UPDATE ON accepted_cursors \
             FOR EACH ROW EXECUTE FUNCTION iotkit_test_fail_cursor()",
        )
        .await
        .expect("install PostgreSQL fault trigger");
    let fault_edge = format!("edge-node-{}", uuid::Uuid::new_v4());
    store
        .accept_batch(batch(fault_edge.clone(), "epoch-01", vec![record(1, 20)]))
        .await
        .expect_err("PostgreSQL cursor persistence fault must fail");
    fault_pool
        .execute("DROP TRIGGER iotkit_test_fail_cursor ON accepted_cursors")
        .await
        .expect("remove PostgreSQL fault trigger");
    fault_pool
        .execute("DROP FUNCTION iotkit_test_fail_cursor()")
        .await
        .expect("remove PostgreSQL fault function");
    assert!(
        store
            .raw_records(&fault_edge, "epoch-01")
            .await
            .expect("read PostgreSQL rollback records")
            .is_empty()
    );
    assert_eq!(
        store
            .accepted_through(&fault_edge, "epoch-01")
            .await
            .expect("read PostgreSQL rollback cursor"),
        0
    );
}
