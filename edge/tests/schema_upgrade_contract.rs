use std::{borrow::Cow, path::PathBuf};

use iotkit_edge::storage::{Storage, StorageProfile};
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
    assert_eq!(version, 10);
    assert_eq!(column_count, 1);
    assert_eq!(history_index_count, 1);
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
    assert_eq!(version, 10);
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
    assert_eq!(version, 10);
    assert_eq!(column_count, 1);
    assert_eq!(history_index_count, 1);
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
    assert_eq!(version, 10);
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
