use std::sync::Arc;
use std::time::Duration;

use iotkit_edge::storage::{StatusApply, Storage, StorageProfile};
use iotkit_edge_custody_contract::{CollectorState, StatusHeartbeat};
use tempfile::TempDir;

fn heartbeat(sequence: u64) -> StatusHeartbeat {
    StatusHeartbeat {
        schema_version: 1,
        edge_node_id: "status-node".into(),
        ledger_epoch: "status-epoch".into(),
        boot_id: "boot-0123456789abcdef0123456789abcdef".into(),
        status_seq: sequence,
        collector_state: CollectorState::Running,
        adapters: Vec::new(),
        accepted_through: 0,
        pending_publications: 0,
        storage_pressure: false,
    }
}

async fn seed_sqlite_active_node(storage: &Storage, path: &std::path::Path) {
    storage.initialize_edge_identity(1).await.unwrap();
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", path.display()))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO edge_node_activations(edge_node_ref,edge_node_id,ledger_epoch,state,revision,created_at,updated_at) \
         VALUES('status-node-ref','status-node','status-epoch','active',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
}

#[tokio::test]
async fn sqlite_status_compare_and_set_rejects_a_concurrently_late_lower_sequence() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("edge.db");
    let newer = Storage::connect(StorageProfile::Sqlite { path: path.clone() })
        .await
        .unwrap();
    seed_sqlite_active_node(&newer, &path).await;
    let lower = newer.clone();
    let newer_heartbeat = heartbeat(2);
    let lower_heartbeat = heartbeat(1);

    let (newer_result, lower_result) = tokio::join!(
        newer.apply_edge_node_status(&newer_heartbeat, 20, false),
        async {
            // Both requests are offered together. SQLite has one guarded
            // writer, while PostgreSQL below also exercises independent pools.
            tokio::time::sleep(Duration::from_millis(5)).await;
            lower
                .apply_edge_node_status(&lower_heartbeat, 21, false)
                .await
        }
    );
    assert_eq!(newer_result.unwrap(), StatusApply::AcceptedLive);
    assert_eq!(lower_result.unwrap(), StatusApply::IgnoredReplay);
    let status = newer
        .edge_node_status("status-node")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (status.status_seq, status.last_live_received_at),
        (2, Some(20))
    );
}

#[tokio::test]
async fn sqlite_status_is_not_current_after_the_active_ledger_epoch_changes() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("edge.db");
    let storage = Storage::connect(StorageProfile::Sqlite { path: path.clone() })
        .await
        .unwrap();
    seed_sqlite_active_node(&storage, &path).await;
    assert_eq!(
        storage
            .apply_edge_node_status(&heartbeat(1), 10, false)
            .await
            .unwrap(),
        StatusApply::AcceptedLive
    );

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", path.display()))
        .await
        .unwrap();
    sqlx::query(
        "UPDATE edge_node_activations SET ledger_epoch='new-status-epoch' \
         WHERE edge_node_id='status-node'",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    assert!(
        storage
            .edge_node_status("status-node")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn sqlite_pending_interval_restarts_when_the_current_cursor_advances() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("edge.db");
    let storage = Storage::connect(StorageProfile::Sqlite { path: path.clone() })
        .await
        .unwrap();
    seed_sqlite_active_node(&storage, &path).await;

    let mut initial = heartbeat(1);
    initial.accepted_through = 10;
    initial.pending_publications = 3;
    storage
        .apply_edge_node_status(&initial, 100, false)
        .await
        .unwrap();

    let mut progressed = heartbeat(2);
    progressed.accepted_through = 11;
    progressed.pending_publications = 3;
    storage
        .apply_edge_node_status(&progressed, 200, false)
        .await
        .unwrap();
    assert_eq!(
        storage
            .edge_node_status("status-node")
            .await
            .unwrap()
            .unwrap()
            .pending_since_at,
        Some(200),
        "continuous pending work begins a new no-progress interval after custody advances"
    );

    let mut cleared = heartbeat(3);
    cleared.accepted_through = 11;
    storage
        .apply_edge_node_status(&cleared, 201, false)
        .await
        .unwrap();
    assert_eq!(
        storage
            .edge_node_status("status-node")
            .await
            .unwrap()
            .unwrap()
            .pending_since_at,
        None
    );
}

#[tokio::test]
async fn sqlite_pending_interval_restarts_when_the_active_epoch_rotates_on_the_same_boot() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("edge.db");
    let storage = Storage::connect(StorageProfile::Sqlite { path: path.clone() })
        .await
        .unwrap();
    seed_sqlite_active_node(&storage, &path).await;

    let mut initial = heartbeat(1);
    initial.accepted_through = 10;
    initial.pending_publications = 3;
    storage
        .apply_edge_node_status(&initial, 100, false)
        .await
        .unwrap();

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", path.display()))
        .await
        .unwrap();
    sqlx::query(
        "UPDATE edge_node_activations SET ledger_epoch='rotated-status-epoch' \
         WHERE edge_node_id='status-node'",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let mut rotated = heartbeat(2);
    rotated.ledger_epoch = "rotated-status-epoch".into();
    rotated.accepted_through = 10;
    rotated.pending_publications = 3;
    storage
        .apply_edge_node_status(&rotated, 200, false)
        .await
        .unwrap();
    let status = storage
        .edge_node_status("status-node")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.boot_id, initial.boot_id);
    assert_eq!(status.pending_since_at, Some(200));
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_status_compare_and_set_rejects_a_concurrently_late_lower_sequence() {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let newer = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .unwrap();
    newer.initialize_edge_identity(1).await.unwrap();
    let pool = sqlx::PgPool::connect(&dsn).await.unwrap();
    sqlx::query(
        "INSERT INTO edge_node_activations(edge_node_ref,edge_node_id,ledger_epoch,state,revision,created_at,updated_at) \
         VALUES('status-node-ref','status-node','status-epoch','active',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    // A second Storage instance correctly fails the deployment lock. Cloning
    // the live store still gives the two offers independent pooled queries.
    let lower = newer.clone();
    let newer_heartbeat = heartbeat(2);
    let lower_heartbeat = heartbeat(1);

    let (newer_result, lower_result) = tokio::join!(
        newer.apply_edge_node_status(&newer_heartbeat, 20, false),
        async {
            tokio::time::sleep(Duration::from_millis(5)).await;
            lower
                .apply_edge_node_status(&lower_heartbeat, 21, false)
                .await
        }
    );
    assert_eq!(newer_result.unwrap(), StatusApply::AcceptedLive);
    assert_eq!(lower_result.unwrap(), StatusApply::IgnoredReplay);
    let status = newer
        .edge_node_status("status-node")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (status.status_seq, status.last_live_received_at),
        (2, Some(20))
    );
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_pending_interval_restarts_when_the_current_cursor_advances() {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let storage = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    let pool = sqlx::PgPool::connect(&dsn).await.unwrap();
    sqlx::query(
        "INSERT INTO edge_node_activations(edge_node_ref,edge_node_id,ledger_epoch,state,revision,created_at,updated_at) \
         VALUES('status-node-ref','status-node','status-epoch','active',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let mut initial = heartbeat(1);
    initial.accepted_through = 10;
    initial.pending_publications = 3;
    storage
        .apply_edge_node_status(&initial, 100, false)
        .await
        .unwrap();
    let mut progressed = heartbeat(2);
    progressed.accepted_through = 11;
    progressed.pending_publications = 3;
    storage
        .apply_edge_node_status(&progressed, 200, false)
        .await
        .unwrap();
    assert_eq!(
        storage
            .edge_node_status("status-node")
            .await
            .unwrap()
            .unwrap()
            .pending_since_at,
        Some(200)
    );
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_pending_interval_restarts_when_the_active_epoch_rotates_on_the_same_boot() {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let storage = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    let pool = sqlx::PgPool::connect(&dsn).await.unwrap();
    sqlx::query(
        "INSERT INTO edge_node_activations(edge_node_ref,edge_node_id,ledger_epoch,state,revision,created_at,updated_at) \
         VALUES('status-node-ref','status-node','status-epoch','active',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let mut initial = heartbeat(1);
    initial.accepted_through = 10;
    initial.pending_publications = 3;
    storage
        .apply_edge_node_status(&initial, 100, false)
        .await
        .unwrap();

    sqlx::query(
        "UPDATE edge_node_activations SET ledger_epoch='rotated-status-epoch' \
         WHERE edge_node_id='status-node'",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let mut rotated = heartbeat(2);
    rotated.ledger_epoch = "rotated-status-epoch".into();
    rotated.accepted_through = 10;
    rotated.pending_publications = 3;
    storage
        .apply_edge_node_status(&rotated, 200, false)
        .await
        .unwrap();
    let status = storage
        .edge_node_status("status-node")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.boot_id, initial.boot_id);
    assert_eq!(status.pending_since_at, Some(200));
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_concurrent_status_offers_cannot_leave_a_lower_sequence_stored() {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(dsn) => dsn,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let higher = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .unwrap();
    higher.initialize_edge_identity(1).await.unwrap();
    let pool = sqlx::PgPool::connect(&dsn).await.unwrap();
    sqlx::query(
        "INSERT INTO edge_node_activations(edge_node_ref,edge_node_id,ledger_epoch,state,revision,created_at,updated_at) \
         VALUES('status-node-ref','status-node','status-epoch','active',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    // See the reordered regression above: keep one deployment owner while
    // racing two real PostgreSQL pooled queries.
    let lower = higher.clone();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let higher_barrier = barrier.clone();
    let lower_barrier = barrier.clone();
    let higher_heartbeat = heartbeat(2);
    let lower_heartbeat = heartbeat(1);

    let (higher_result, lower_result) = tokio::join!(
        async {
            higher_barrier.wait().await;
            higher
                .apply_edge_node_status(&higher_heartbeat, 20, false)
                .await
        },
        async {
            lower_barrier.wait().await;
            lower
                .apply_edge_node_status(&lower_heartbeat, 21, false)
                .await
        }
    );
    assert_eq!(higher_result.unwrap(), StatusApply::AcceptedLive);
    assert!(matches!(
        lower_result.unwrap(),
        StatusApply::AcceptedLive | StatusApply::IgnoredReplay
    ));
    let status = higher
        .edge_node_status("status-node")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.status_seq, 2);
}
