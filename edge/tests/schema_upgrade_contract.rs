use std::{borrow::Cow, path::PathBuf};

use iotkit_edge::storage::{
    AcceptBatch, RawRecord, Storage, StorageError, StorageProfile, migrate_sqlite_to_postgres,
};
use sqlx::{
    PgPool, SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tempfile::TempDir;

fn migrations(profile: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("migrations")
        .join(profile)
}

async fn migrator_through_v6(profile: &str) -> Migrator {
    let full = Migrator::new(migrations(profile))
        .await
        .expect("load migrations");
    Migrator {
        migrations: Cow::Owned(
            full.iter()
                .filter(|migration| migration.version <= 6)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    }
}

async fn migrator_through_v8(profile: &str) -> Migrator {
    let full = Migrator::new(migrations(profile))
        .await
        .expect("load migrations");
    Migrator {
        migrations: Cow::Owned(
            full.iter()
                .filter(|migration| migration.version <= 8)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    }
}

async fn migrator_through_v10(profile: &str) -> Migrator {
    let full = Migrator::new(migrations(profile))
        .await
        .expect("load migrations");
    Migrator {
        migrations: Cow::Owned(
            full.iter()
                .filter(|migration| migration.version <= 10)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    }
}

async fn migrator_through_v11(profile: &str) -> Migrator {
    let full = Migrator::new(migrations(profile))
        .await
        .expect("load migrations");
    Migrator {
        migrations: Cow::Owned(
            full.iter()
                .filter(|migration| migration.version <= 11)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    }
}

fn v10_backfill_records() -> Vec<(i64, Vec<u8>)> {
    [
        (
            "measurement",
            serde_json::Value::String("series-good".into()),
        ),
        ("status", serde_json::Value::String("series-status".into())),
        ("measurement", serde_json::Value::Null),
        ("measurement", serde_json::json!(42)),
        ("measurement", serde_json::Value::String(String::new())),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (family, series_key))| {
        let sequence = i64::try_from(index + 1).unwrap();
        (
            sequence,
            serde_json::to_vec(&serde_json::json!({
                "family": family,
                "schema_version": 1,
                "epoch": "epoch",
                "pub_seq": sequence,
                "series_key": series_key,
                "values": [sequence],
                "event_time": sequence,
                "event_time_source": "received_at",
                "time_source": "edge_node",
                "time_quality": "unsynced",
                "received_at": sequence,
                "device_time": null
            }))
            .unwrap(),
        )
    })
    .collect()
}

fn measurement_record(ledger_epoch: &str, pub_seq: i64, series_key: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "family": "measurement",
        "schema_version": 1,
        "epoch": ledger_epoch,
        "pub_seq": pub_seq,
        "series_key": series_key,
        "values": [pub_seq],
        "event_time": pub_seq,
        "event_time_source": "received_at",
        "time_source": "edge_node",
        "time_quality": "unsynced",
        "received_at": pub_seq,
        "device_time": null
    }))
    .unwrap()
}

fn incompressible_series_key(byte_len: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut series_key = String::with_capacity(byte_len);
    for _ in 0..byte_len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        series_key.push(ALPHABET[(state % ALPHABET.len() as u64) as usize] as char);
    }
    series_key
}

#[tokio::test]
async fn sqlite_startup_upgrades_a_v6_database_without_losing_identity() {
    let directory = TempDir::new_in(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .expect("temporary directory");
    let path = directory.path().join("upgrade.db");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .expect("create v6 database");
    migrator_through_v6("sqlite")
        .await
        .run(&pool)
        .await
        .expect("apply migrations through v6");
    sqlx::query("INSERT INTO edge_meta(singleton,edge_id,created_at) VALUES(1,?,1)")
        .bind("edge-upgrade-sqlite")
        .execute(&pool)
        .await
        .expect("seed identity");
    pool.close().await;

    let storage = Storage::connect(StorageProfile::Sqlite { path: path.clone() })
        .await
        .expect("start current Rust Edge on v6 database");
    assert_eq!(storage.edge_id().await.unwrap(), "edge-upgrade-sqlite");
    drop(storage);

    let inspection = SqlitePool::connect(&format!("sqlite:{}", path.display()))
        .await
        .expect("inspect upgraded database");
    let version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE")
            .fetch_one(&inspection)
            .await
            .expect("read schema version");
    let column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('output_routes') \
         WHERE name='start_after_observation_row_id'",
    )
    .fetch_one(&inspection)
    .await
    .expect("inspect output route schema");
    let history_index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' \
         AND name='ix_semantic_observation_rule_observed_at_row'",
    )
    .fetch_one(&inspection)
    .await
    .expect("inspect semantic history index");
    assert_eq!(version, 12);
    assert_eq!(column_count, 1);
    assert_eq!(history_index_count, 1);
}

#[tokio::test]
async fn sqlite_startup_upgrades_v10_and_backfills_only_valid_measurement_series_keys() {
    let directory = TempDir::new_in(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .expect("temporary directory");
    let path = directory.path().join("upgrade-v10.db");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .expect("create v10 database");
    migrator_through_v10("sqlite")
        .await
        .run(&pool)
        .await
        .expect("apply migrations through v10");
    sqlx::query(
        "INSERT INTO edge_meta(singleton,edge_id,created_at) VALUES(1,'edge-upgrade-v10',1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for (sequence, record_json) in v10_backfill_records() {
        sqlx::query(
            "INSERT INTO raw_records(edge_node_id,ledger_epoch,pub_seq,publication_id,\
             record_json,record_sha256,received_at) VALUES('node','epoch',?,?,?,?,?)",
        )
        .bind(sequence)
        .bind(format!("publication-{sequence}"))
        .bind(record_json)
        .bind(vec![sequence as u8; 32])
        .bind(sequence)
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;

    let storage = Storage::connect(StorageProfile::Sqlite { path: path.clone() })
        .await
        .expect("upgrade v10 database");
    assert_eq!(storage.edge_id().await.unwrap(), "edge-upgrade-v10");
    drop(storage);

    let inspection = SqlitePool::connect(&format!("sqlite:{}", path.display()))
        .await
        .expect("inspect upgraded database");
    let version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE")
            .fetch_one(&inspection)
            .await
            .unwrap();
    let series_keys: Vec<(i64, Option<String>)> =
        sqlx::query_as("SELECT pub_seq,series_key FROM raw_records ORDER BY pub_seq")
            .fetch_all(&inspection)
            .await
            .unwrap();
    let column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('raw_records') WHERE name='series_key'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    let index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' \
         AND name='ix_raw_records_preview_signal_received'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    assert_eq!(version, 12);
    assert_eq!(
        series_keys,
        vec![
            (1, Some("series-good".into())),
            (2, None),
            (3, None),
            (4, None),
            (5, None),
        ]
    );
    assert_eq!(column_count, 1);
    assert_eq!(index_count, 1);
}

#[tokio::test]
async fn sqlite_v11_migration_failure_rolls_back_the_new_column() {
    let directory = TempDir::new_in(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .expect("temporary directory");
    let path = directory.path().join("upgrade-v10-failure.db");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .expect("create v10 database");
    migrator_through_v10("sqlite")
        .await
        .run(&pool)
        .await
        .expect("apply migrations through v10");
    sqlx::query("CREATE INDEX ix_raw_records_preview_signal_received ON raw_records(received_at)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    assert!(
        Storage::connect(StorageProfile::Sqlite { path: path.clone() })
            .await
            .is_err()
    );

    let inspection = SqlitePool::connect(&format!("sqlite:{}", path.display()))
        .await
        .expect("inspect rolled-back database");
    let column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('raw_records') WHERE name='series_key'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    let version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE")
            .fetch_one(&inspection)
            .await
            .unwrap();
    assert_eq!(column_count, 0);
    assert_eq!(version, 10);
}

#[tokio::test]
async fn sqlite_v12_status_migration_from_v11_adds_an_empty_latest_only_store() {
    let directory = TempDir::new_in(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .expect("temporary directory");
    let path = directory.path().join("upgrade-v11-status.db");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .expect("create v11 database");
    migrator_through_v11("sqlite")
        .await
        .run(&pool)
        .await
        .expect("apply migrations through v11");
    sqlx::query(
        "INSERT INTO edge_meta(singleton,edge_id,created_at) VALUES(1,'edge-upgrade-v11',1)",
    )
    .execute(&pool)
    .await
    .expect("seed identity");
    pool.close().await;

    let storage = Storage::connect(StorageProfile::Sqlite { path: path.clone() })
        .await
        .expect("upgrade v11 database");
    assert_eq!(storage.edge_id().await.unwrap(), "edge-upgrade-v11");
    drop(storage);

    let inspection = SqlitePool::connect(&format!("sqlite:{}", path.display()))
        .await
        .expect("inspect upgraded database");
    let version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE")
            .fetch_one(&inspection)
            .await
            .unwrap();
    let status_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='edge_node_status'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    let status_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM edge_node_status")
        .fetch_one(&inspection)
        .await
        .unwrap();
    let status_fk_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_foreign_key_list('edge_node_status') \
         WHERE \"table\"='edge_node_activations' AND \"from\"='edge_node_id' \
         AND on_delete='CASCADE'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    assert_eq!(version, 12);
    assert_eq!(status_table_count, 1);
    assert_eq!(status_rows, 0, "v12 does not invent health history");
    assert_eq!(status_fk_count, 1);
    let pending_since_column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('edge_node_status') WHERE name='pending_since_at'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    let recovery_index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' \
         AND name='ix_semantic_observation_recovery'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    let causal_index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name IN (\
         'ix_raw_records_diagnostic_epoch_signal_received',\
         'ix_semantic_observation_diagnostic_latest',\
         'ix_output_outbox_diagnostic_route_published',\
         'ix_output_outbox_diagnostic_route_pending')",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    assert_eq!(pending_since_column_count, 1);
    assert_eq!(recovery_index_count, 1);
    assert_eq!(causal_index_count, 4);
}

#[tokio::test]
async fn sqlite_v12_status_migration_failure_rolls_back_the_new_table() {
    let directory = TempDir::new_in(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .expect("temporary directory");
    let path = directory.path().join("upgrade-v11-status-failure.db");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .expect("create v11 database");
    migrator_through_v11("sqlite")
        .await
        .run(&pool)
        .await
        .expect("apply migrations through v11");
    // The index name is database-global. Its deliberate collision makes the
    // second statement of v12 fail after CREATE TABLE, exercising DDL rollback.
    sqlx::query("CREATE INDEX ix_edge_node_status_live ON raw_records(received_at)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    assert!(
        Storage::connect(StorageProfile::Sqlite { path: path.clone() })
            .await
            .is_err()
    );

    let inspection = SqlitePool::connect(&format!("sqlite:{}", path.display()))
        .await
        .expect("inspect rolled-back database");
    let version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE")
            .fetch_one(&inspection)
            .await
            .unwrap();
    let status_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='edge_node_status'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    assert_eq!(version, 11);
    assert_eq!(status_table_count, 0);
}

#[tokio::test]
async fn sqlite_to_postgres_requires_current_startup_upgrade_before_copy() {
    let directory = TempDir::new_in(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .expect("temporary directory");
    let path = directory.path().join("source-v10.db");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .expect("create v10 database");
    migrator_through_v10("sqlite")
        .await
        .run(&pool)
        .await
        .expect("apply migrations through v10");
    pool.close().await;

    let error = migrate_sqlite_to_postgres(&path, "postgres://must-not-be-opened")
        .await
        .expect_err("v10 source must be upgraded before offline profile migration");
    assert!(matches!(&error, StorageError::ProfileMigration(_)));
    assert!(error.to_string().contains("start current IoTKit Edge"));
}

#[tokio::test]
async fn sqlite_startup_upgrades_v8_with_noncontiguous_receipts_and_snapshots_each_pending_pair() {
    let directory = TempDir::new_in(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    )
    .expect("temporary directory");
    let path = directory.path().join("upgrade-v8.db");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .expect("create v8 database");
    migrator_through_v8("sqlite")
        .await
        .run(&pool)
        .await
        .expect("apply migrations through v8");
    sqlx::query(
        "INSERT INTO edge_meta(singleton,edge_id,created_at) VALUES(1,'edge-upgrade-v8',1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO accepted_cursors(edge_node_id,ledger_epoch,accepted_through,updated_at) \
         VALUES('node','epoch',4,4)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO semantic_signals(signal_ref,edge_node_id,series_key,calibration_revision,\
         scale,calibration_offset,created_at) VALUES('signal','node','series',2,2,0,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO semantic_rules(rule_id,signal_ref,display_name,kind,series_id,revision,\
         spec_json,active,created_at,retired_at) \
         VALUES('rule','signal','Retired numeric','numeric','series-v2',2,'{}',0,1,5)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for (revision, series_id) in [(1_i64, "series-v1"), (2, "series-v2")] {
        sqlx::query(
            "INSERT INTO semantic_rule_revisions(rule_id,revision,series_id,spec_json,created_at) \
             VALUES('rule',?,?,'{}',?)",
        )
        .bind(revision)
        .bind(series_id)
        .bind(revision)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO semantic_calibration_revisions(\
             signal_ref,revision,scale,calibration_offset,created_at) VALUES('signal',?,?,0,?)",
        )
        .bind(revision)
        .bind(revision as f64)
        .bind(revision)
        .execute(&pool)
        .await
        .unwrap();
    }
    for (revision, boundary) in [(1_i64, 0_i64), (2, 2)] {
        sqlx::query(
            "INSERT INTO semantic_rule_starts(rule_id,revision,ledger_epoch,start_after_pub_seq) \
             VALUES('rule',?,'epoch',?)",
        )
        .bind(revision)
        .bind(boundary)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO semantic_calibration_starts(\
             signal_ref,revision,ledger_epoch,start_after_pub_seq) VALUES('signal',?,'epoch',?)",
        )
        .bind(revision)
        .bind(boundary)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO semantic_rule_ends(rule_id,ledger_epoch,end_at_pub_seq) \
         VALUES('rule','epoch',4)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO semantic_rule_runtime(rule_id,initialized,detector_active,counter,pending,\
         pending_active,pending_since,applied_revision,applied_calibration_revision,\
         applied_ledger_epoch,applied_series_id,next_sequence) \
         VALUES('rule',1,0,7,0,0,0,2,2,'epoch','series-v2',8)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for sequence in 1_i64..=4 {
        let record = serde_json::json!({
            "family":"measurement","schema_version":1,"epoch":"epoch",
            "pub_seq":sequence,"series_key":"series","values":[sequence as f64],
            "event_time":sequence,"event_time_source":"received_at",
            "time_source":"edge_node","time_quality":"unsynced",
            "received_at":sequence,"device_time":null
        });
        sqlx::query(
            "INSERT INTO raw_records(edge_node_id,ledger_epoch,pub_seq,publication_id,\
             record_json,record_sha256,received_at) VALUES('node','epoch',?,?,?,?,?)",
        )
        .bind(sequence)
        .bind(format!("publication-{sequence}"))
        .bind(serde_json::to_vec(&record).unwrap())
        .bind(vec![sequence as u8; 32])
        .bind(sequence)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO semantic_observations(observation_id,rule_id,revision,\
         calibration_revision,series_id,sequence,kind,value_json,reading,signal_ref,\
         edge_node_id,ledger_epoch,source_pub_seq,observed_at,created_at) \
         VALUES('observation-1','rule',1,1,'series-v1',1,'numeric','1',NULL,'signal',\
         'node','epoch',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for (sequence, observation_id) in [(1_i64, Some("observation-1")), (3, None)] {
        sqlx::query(
            "INSERT INTO semantic_projection_receipts(rule_id,ledger_epoch,pub_seq,revision,\
             calibration_revision,observation_id) VALUES('rule','epoch',?,?,?,?)",
        )
        .bind(sequence)
        .bind(if sequence == 1 { 1 } else { 2 })
        .bind(if sequence == 1 { 1 } else { 2 })
        .bind(observation_id)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO semantic_projection_failures(rule_id,ledger_epoch,pub_seq,error_code,\
         attempts,last_failed_at) VALUES('rule','epoch',3,'invalid_observation',2,3)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let storage = Storage::connect(StorageProfile::Sqlite { path: path.clone() })
        .await
        .expect("upgrade v8 database");
    assert_eq!(storage.edge_id().await.unwrap(), "edge-upgrade-v8");
    drop(storage);

    let inspection = SqlitePool::connect(&format!("sqlite:{}", path.display()))
        .await
        .expect("inspect upgraded database");
    let version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE")
            .fetch_one(&inspection)
            .await
            .unwrap();
    let history_index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' \
         AND name='ix_semantic_observation_rule_observed_at_row'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    assert_eq!(version, 12);
    assert_eq!(history_index_count, 1);
    let queue: Vec<(i64, i64, i64)> = sqlx::query_as(
        "SELECT pub_seq,revision,calibration_revision FROM semantic_projection_queue \
         WHERE rule_id='rule' ORDER BY pub_seq",
    )
    .fetch_all(&inspection)
    .await
    .unwrap();
    assert_eq!(queue, vec![(2, 1, 1), (4, 2, 2)]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM semantic_projection_receipts")
            .fetch_one(&inspection)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM semantic_observations")
            .fetch_one(&inspection)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT attempts FROM semantic_projection_failures")
            .fetch_one(&inspection)
            .await
            .unwrap(),
        2
    );
    let runtime: (i64, i64, String, String, i64) = sqlx::query_as(
        "SELECT counter,applied_revision,applied_ledger_epoch,applied_series_id,next_sequence \
         FROM semantic_rule_runtime WHERE rule_id='rule'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    assert_eq!(runtime, (7, 2, "epoch".into(), "series-v2".into(), 8));
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_startup_upgrades_a_v6_database_without_losing_identity() {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let pool = PgPool::connect(&dsn).await.expect("create v6 database");
    migrator_through_v6("postgres")
        .await
        .run(&pool)
        .await
        .expect("apply migrations through v6");
    sqlx::query("INSERT INTO edge_meta(singleton,edge_id,created_at) VALUES(1,$1,1)")
        .bind("edge-upgrade-postgres")
        .execute(&pool)
        .await
        .expect("seed identity");
    pool.close().await;

    let storage = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .expect("start current Rust Edge on v6 database");
    assert_eq!(storage.edge_id().await.unwrap(), "edge-upgrade-postgres");
    drop(storage);

    let inspection = PgPool::connect(&dsn)
        .await
        .expect("inspect upgraded database");
    let version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE")
            .fetch_one(&inspection)
            .await
            .expect("read schema version");
    let column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.columns \
         WHERE table_schema='public' AND table_name='output_routes' \
         AND column_name='start_after_observation_row_id'",
    )
    .fetch_one(&inspection)
    .await
    .expect("inspect output route schema");
    let history_index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_indexes WHERE schemaname='public' \
         AND tablename='semantic_observations' \
         AND indexname='ix_semantic_observation_rule_observed_at_row'",
    )
    .fetch_one(&inspection)
    .await
    .expect("inspect semantic history index");
    assert_eq!(version, 12);
    assert_eq!(column_count, 1);
    assert_eq!(history_index_count, 1);
    inspection.close().await;
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_startup_upgrades_v10_and_backfills_only_valid_measurement_series_keys() {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let pool = PgPool::connect(&dsn).await.expect("create v10 database");
    migrator_through_v10("postgres")
        .await
        .run(&pool)
        .await
        .expect("apply migrations through v10");
    sqlx::query(
        "INSERT INTO edge_meta(singleton,edge_id,created_at) VALUES(1,'edge-upgrade-v10',1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for (sequence, record_json) in v10_backfill_records() {
        sqlx::query(
            "INSERT INTO raw_records(edge_node_id,ledger_epoch,pub_seq,publication_id,\
             record_json,record_sha256,received_at) VALUES('node','epoch',$1,$2,$3,$4,$1)",
        )
        .bind(sequence)
        .bind(format!("publication-{sequence}"))
        .bind(record_json)
        .bind(vec![sequence as u8; 32])
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;

    let storage = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .expect("upgrade v10 database");
    assert_eq!(storage.edge_id().await.unwrap(), "edge-upgrade-v10");
    drop(storage);

    let inspection = PgPool::connect(&dsn)
        .await
        .expect("inspect upgraded database");
    let version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE")
            .fetch_one(&inspection)
            .await
            .unwrap();
    let series_keys: Vec<(i64, Option<String>)> =
        sqlx::query_as("SELECT pub_seq,series_key FROM raw_records ORDER BY pub_seq")
            .fetch_all(&inspection)
            .await
            .unwrap();
    let column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema='public' \
         AND table_name='raw_records' AND column_name='series_key'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    let index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_indexes WHERE schemaname='public' AND tablename='raw_records' \
         AND indexname='ix_raw_records_preview_signal_received'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    assert_eq!(version, 12);
    assert_eq!(
        series_keys,
        vec![
            (1, Some("series-good".into())),
            (2, None),
            (3, None),
            (4, None),
            (5, None),
        ]
    );
    assert_eq!(column_count, 1);
    assert_eq!(index_count, 1);
    inspection.close().await;
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_v11_upgrade_acceptance_and_preview_support_a_2679_byte_series_key() {
    const LONG_SERIES_KEY_BYTES: usize = 2_679;

    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let series_key = incompressible_series_key(LONG_SERIES_KEY_BYTES);
    assert_eq!(series_key.len(), LONG_SERIES_KEY_BYTES);
    let pool = PgPool::connect(&dsn).await.expect("create v10 database");
    migrator_through_v10("postgres")
        .await
        .run(&pool)
        .await
        .expect("apply migrations through v10");
    sqlx::query(
        "INSERT INTO raw_records(edge_node_id,ledger_epoch,pub_seq,publication_id,\
         record_json,record_sha256,received_at) VALUES('long-node','legacy-epoch',1,\
         'legacy-long-key',$1,$2,10)",
    )
    .bind(measurement_record("legacy-epoch", 1, &series_key))
    .bind(vec![0_u8; 32])
    .execute(&pool)
    .await
    .expect("seed long legacy record");
    sqlx::query(
        "INSERT INTO inventory_signals(signal_ref,edge_node_id,series_key,system_id,created_at) \
         VALUES('sig-long-key','long-node',$1,'long-system',1)",
    )
    .bind(&series_key)
    .execute(&pool)
    .await
    .expect("seed long legacy signal identity");
    pool.close().await;

    let storage = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .expect("upgrade v10 long-key record to v11");
    storage
        .initialize_edge_identity(1)
        .await
        .expect("initialize identity");
    let inspection = PgPool::connect(&dsn).await.expect("inspect v11 database");
    let index_definition: String = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes WHERE schemaname='public' AND tablename='raw_records' \
         AND indexname='ix_raw_records_preview_signal_received'",
    )
    .fetch_one(&inspection)
    .await
    .expect("read long-key preview index definition");
    assert!(
        index_definition.contains("md5(series_key)"),
        "PostgreSQL preview index must use a fixed-length key discriminator: {index_definition}"
    );
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "long-node".into(),
            ledger_epoch: "accepted-epoch".into(),
            publication_id: "accepted-long-key".into(),
            received_at: 20,
            records: vec![
                RawRecord::new(1, measurement_record("accepted-epoch", 1, &series_key))
                    .expect("encode long accepted record"),
            ],
        })
        .await
        .expect("accept long v11 record");
    let inputs = storage
        .recent_signal_inputs("sig-long-key", 10)
        .await
        .expect("read exact long-key preview tail");
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.received_at)
            .collect::<Vec<_>>(),
        vec![10, 20]
    );
    let stored_keys: Vec<String> = sqlx::query_scalar(
        "SELECT series_key FROM raw_records WHERE edge_node_id='long-node' \
         ORDER BY received_at",
    )
    .fetch_all(&inspection)
    .await
    .expect("read derived long keys");
    assert_eq!(stored_keys, vec![series_key.clone(), series_key]);
    inspection.close().await;
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_v11_migration_failure_rolls_back_the_new_column() {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let pool = PgPool::connect(&dsn).await.expect("create v10 database");
    migrator_through_v10("postgres")
        .await
        .run(&pool)
        .await
        .expect("apply migrations through v10");
    sqlx::query("CREATE INDEX ix_raw_records_preview_signal_received ON raw_records(received_at)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    assert!(
        Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
            .await
            .is_err()
    );

    let inspection = PgPool::connect(&dsn)
        .await
        .expect("inspect rolled-back database");
    let column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema='public' \
         AND table_name='raw_records' AND column_name='series_key'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    let version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE")
            .fetch_one(&inspection)
            .await
            .unwrap();
    assert_eq!(column_count, 0);
    assert_eq!(version, 10);
    inspection.close().await;
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_v12_status_migration_from_v11_adds_an_empty_latest_only_store() {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let pool = PgPool::connect(&dsn).await.expect("create v11 database");
    migrator_through_v11("postgres")
        .await
        .run(&pool)
        .await
        .expect("apply migrations through v11");
    sqlx::query(
        "INSERT INTO edge_meta(singleton,edge_id,created_at) VALUES(1,'edge-upgrade-v11',1)",
    )
    .execute(&pool)
    .await
    .expect("seed identity");
    pool.close().await;

    let storage = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .expect("upgrade v11 database");
    assert_eq!(storage.edge_id().await.unwrap(), "edge-upgrade-v11");
    drop(storage);

    let inspection = PgPool::connect(&dsn)
        .await
        .expect("inspect upgraded database");
    let version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE")
            .fetch_one(&inspection)
            .await
            .unwrap();
    let status_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema='public' \
         AND table_name='edge_node_status'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    let status_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM edge_node_status")
        .fetch_one(&inspection)
        .await
        .unwrap();
    let pending_since_column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema='public' \
         AND table_name='edge_node_status' AND column_name='pending_since_at'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    let status_fk_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.table_constraints \
         WHERE table_schema='public' AND table_name='edge_node_status' \
         AND constraint_type='FOREIGN KEY'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    let recovery_index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_indexes WHERE schemaname='public' \
         AND tablename='semantic_observations' \
         AND indexname='ix_semantic_observation_recovery'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    let causal_index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_indexes WHERE schemaname='public' AND indexname IN (\
         'ix_raw_records_diagnostic_epoch_signal_received',\
         'ix_semantic_observation_diagnostic_latest',\
         'ix_output_outbox_diagnostic_route_published',\
         'ix_output_outbox_diagnostic_route_pending')",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    assert_eq!(version, 12);
    assert_eq!(status_table_count, 1);
    assert_eq!(status_rows, 0, "v12 does not invent health history");
    assert_eq!(pending_since_column_count, 1);
    assert_eq!(status_fk_count, 1);
    assert_eq!(recovery_index_count, 1);
    assert_eq!(causal_index_count, 4);
    inspection.close().await;
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_v12_status_migration_failure_rolls_back_the_new_table() {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let pool = PgPool::connect(&dsn).await.expect("create v11 database");
    migrator_through_v11("postgres")
        .await
        .run(&pool)
        .await
        .expect("apply migrations through v11");
    sqlx::query("CREATE INDEX ix_edge_node_status_live ON raw_records(received_at)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    assert!(
        Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
            .await
            .is_err()
    );

    let inspection = PgPool::connect(&dsn)
        .await
        .expect("inspect rolled-back database");
    let version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE")
            .fetch_one(&inspection)
            .await
            .unwrap();
    let status_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema='public' \
         AND table_name='edge_node_status'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    assert_eq!(version, 11);
    assert_eq!(status_table_count, 0);
    inspection.close().await;
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_startup_upgrades_v8_with_noncontiguous_receipts_and_snapshots_each_pending_pair()
{
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let pool = PgPool::connect(&dsn).await.expect("create v8 database");
    migrator_through_v8("postgres")
        .await
        .run(&pool)
        .await
        .expect("apply migrations through v8");
    sqlx::query(
        "INSERT INTO edge_meta(singleton,edge_id,created_at) VALUES(1,'edge-upgrade-v8',1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO accepted_cursors(edge_node_id,ledger_epoch,accepted_through,updated_at) \
         VALUES('node','epoch',4,4)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO semantic_signals(signal_ref,edge_node_id,series_key,calibration_revision,\
         scale,calibration_offset,created_at) VALUES('signal','node','series',2,2,0,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO semantic_rules(rule_id,signal_ref,display_name,kind,series_id,revision,\
         spec_json,active,created_at,retired_at) \
         VALUES('rule','signal','Retired numeric','numeric','series-v2',2,'{}'::jsonb,FALSE,1,5)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for (revision, series_id) in [(1_i64, "series-v1"), (2, "series-v2")] {
        sqlx::query(
            "INSERT INTO semantic_rule_revisions(rule_id,revision,series_id,spec_json,created_at) \
             VALUES('rule',$1,$2,'{}'::jsonb,$1)",
        )
        .bind(revision)
        .bind(series_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO semantic_calibration_revisions(\
             signal_ref,revision,scale,calibration_offset,created_at) VALUES('signal',$1,$2,0,$1)",
        )
        .bind(revision)
        .bind(revision as f64)
        .execute(&pool)
        .await
        .unwrap();
    }
    for (revision, boundary) in [(1_i64, 0_i64), (2, 2)] {
        sqlx::query(
            "INSERT INTO semantic_rule_starts(rule_id,revision,ledger_epoch,start_after_pub_seq) \
             VALUES('rule',$1,'epoch',$2)",
        )
        .bind(revision)
        .bind(boundary)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO semantic_calibration_starts(\
             signal_ref,revision,ledger_epoch,start_after_pub_seq) VALUES('signal',$1,'epoch',$2)",
        )
        .bind(revision)
        .bind(boundary)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO semantic_rule_ends(rule_id,ledger_epoch,end_at_pub_seq) \
         VALUES('rule','epoch',4)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO semantic_rule_runtime(rule_id,initialized,detector_active,counter,pending,\
         pending_active,pending_since,applied_revision,applied_calibration_revision,\
         applied_ledger_epoch,applied_series_id,next_sequence) \
         VALUES('rule',TRUE,FALSE,7,FALSE,FALSE,0,2,2,'epoch','series-v2',8)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for sequence in 1_i64..=4 {
        let record = serde_json::json!({
            "family":"measurement","schema_version":1,"epoch":"epoch",
            "pub_seq":sequence,"series_key":"series","values":[sequence as f64],
            "event_time":sequence,"event_time_source":"received_at",
            "time_source":"edge_node","time_quality":"unsynced",
            "received_at":sequence,"device_time":null
        });
        sqlx::query(
            "INSERT INTO raw_records(edge_node_id,ledger_epoch,pub_seq,publication_id,\
             record_json,record_sha256,received_at) VALUES('node','epoch',$1,$2,$3,$4,$1)",
        )
        .bind(sequence)
        .bind(format!("publication-{sequence}"))
        .bind(serde_json::to_vec(&record).unwrap())
        .bind(vec![sequence as u8; 32])
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO semantic_observations(observation_id,rule_id,revision,\
         calibration_revision,series_id,sequence,kind,value_json,reading,signal_ref,\
         edge_node_id,ledger_epoch,source_pub_seq,observed_at,created_at) \
         VALUES('observation-1','rule',1,1,'series-v1',1,'numeric','1'::jsonb,NULL,'signal',\
         'node','epoch',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for (sequence, observation_id) in [(1_i64, Some("observation-1")), (3, None)] {
        sqlx::query(
            "INSERT INTO semantic_projection_receipts(rule_id,ledger_epoch,pub_seq,revision,\
             calibration_revision,observation_id) VALUES('rule','epoch',$1,$2,$2,$3)",
        )
        .bind(sequence)
        .bind(if sequence == 1 { 1 } else { 2 })
        .bind(observation_id)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO semantic_projection_failures(rule_id,ledger_epoch,pub_seq,error_code,\
         attempts,last_failed_at) VALUES('rule','epoch',3,'invalid_observation',2,3)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let storage = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .expect("upgrade v8 database");
    assert_eq!(storage.edge_id().await.unwrap(), "edge-upgrade-v8");
    drop(storage);

    let inspection = PgPool::connect(&dsn)
        .await
        .expect("inspect upgraded database");
    let version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE")
            .fetch_one(&inspection)
            .await
            .unwrap();
    let history_index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_indexes WHERE schemaname='public' \
         AND tablename='semantic_observations' \
         AND indexname='ix_semantic_observation_rule_observed_at_row'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    assert_eq!(version, 12);
    assert_eq!(history_index_count, 1);
    let queue: Vec<(i64, i64, i64)> = sqlx::query_as(
        "SELECT pub_seq,revision,calibration_revision FROM semantic_projection_queue \
         WHERE rule_id='rule' ORDER BY pub_seq",
    )
    .fetch_all(&inspection)
    .await
    .unwrap();
    assert_eq!(queue, vec![(2, 1, 1), (4, 2, 2)]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM semantic_projection_receipts")
            .fetch_one(&inspection)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM semantic_observations")
            .fetch_one(&inspection)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT attempts FROM semantic_projection_failures")
            .fetch_one(&inspection)
            .await
            .unwrap(),
        2
    );
    let runtime: (i64, i64, String, String, i64) = sqlx::query_as(
        "SELECT counter,applied_revision,applied_ledger_epoch,applied_series_id,next_sequence \
         FROM semantic_rule_runtime WHERE rule_id='rule'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    assert_eq!(runtime, (7, 2, "epoch".into(), "series-v2".into(), 8));
    inspection.close().await;
}
