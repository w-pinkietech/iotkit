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
    assert_eq!(version, 8);
    assert_eq!(column_count, 1);
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
    assert_eq!(version, 8);
    assert_eq!(column_count, 1);
    inspection.close().await;
}
