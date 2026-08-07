use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

use iotkit_edge::{
    application::semantics::{SemanticRuleDraft, Semantics},
    auth::{
        password::{Password, hash_password},
        principal::AccountRole,
        session::{SessionSecrets, SessionWindow},
    },
    backup::{
        BackupError, create_encrypted_backup, restore_encrypted_backup_postgres,
        restore_encrypted_backup_sqlite,
    },
    composition::registered_output_adapters,
    diagnostics::storage_status,
    semantics::{Detector, RuleSpec, SemanticKind, TriggerMode},
    storage::{
        AcceptBatch, AccountProvision, AuditActor, EdgeNodeState, RawRecord, Storage, StorageError,
        StorageProfile,
    },
};
use iotkit_edge_custody_contract::DescriptorSnapshot;
use tempfile::TempDir;

const QUEUE_SERIES: &str = "018f0000-0000-7000-8000-000000000001:temperature:na:primary";

async fn seed_pending_projection(storage: &Storage, edge_node_id: &str, ledger_epoch: &str) {
    let descriptor = DescriptorSnapshot::decode(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "edge_node_id": edge_node_id,
            "ledger_epoch": ledger_epoch,
            "descriptor_revision": 1,
            "complete": true,
            "devices": [{
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "identifier": "backup-queue-device",
                "state": "active",
                "model_id": "contract"
            }],
            "signals": [{
                "series_key": QUEUE_SERIES,
                "system_id": "018f0000-0000-7000-8000-000000000001",
                "measurement_key": "temperature",
                "channel_index": null,
                "variant": "primary",
                "unit": null,
                "value_type": "float"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    storage.apply_descriptor(&descriptor, 2).await.unwrap();
    Semantics::new(storage.clone())
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: edge_node_id.into(),
                series_key: QUEUE_SERIES.into(),
                display_name: "Backup queue temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            3,
        )
        .await
        .unwrap();
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: edge_node_id.into(),
            ledger_epoch: ledger_epoch.into(),
            publication_id: "pending-projection".into(),
            received_at: 4,
            records: vec![
                RawRecord::new(
                    1,
                    serde_json::to_vec(&serde_json::json!({
                        "family": "measurement",
                        "schema_version": 1,
                        "epoch": ledger_epoch,
                        "pub_seq": 1,
                        "series_key": QUEUE_SERIES,
                        "values": [42.0],
                        "event_time": 4,
                        "event_time_source": "received_at",
                        "time_source": "edge_node",
                        "time_quality": "unsynced",
                        "received_at": 4,
                        "device_time": null
                    }))
                    .unwrap(),
                )
                .unwrap(),
            ],
        })
        .await
        .unwrap();
}

async fn populated_store() -> (TempDir, PathBuf, Storage) {
    let directory = TempDir::new().expect("temporary directory");
    let database = directory.path().join("edge.db");
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: database.clone(),
    })
    .await
    .expect("open SQLite store");
    storage
        .initialize_edge_identity(1_721_800_000_000)
        .await
        .expect("initialize Edge identity");
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: "publication-01".into(),
            received_at: 1_721_800_000_100,
            records: vec![
                RawRecord::new(1, br#"{"value":20}"#).expect("record"),
                RawRecord::new(2, br#"{"value":21}"#).expect("record"),
            ],
        })
        .await
        .expect("accept records");
    (directory, database, storage)
}

#[tokio::test]
async fn encrypted_sqlite_backup_round_trips_and_revokes_sessions() {
    let (directory, _database, storage) = populated_store().await;
    let backup = directory.path().join("edge.iotkit-backup");
    let restored = directory.path().join("restored.db");
    let account = storage
        .create_account(
            AccountProvision {
                login_id: "restore-owner".into(),
                display_name: "Restore Owner".into(),
                role: AccountRole::SystemAdmin,
                password_hash: hash_password(&Password::new("account password secret").unwrap())
                    .unwrap(),
                must_change_password: false,
                require_unowned: true,
            },
            AuditActor::local_cli(),
            1_721_800_000_200,
        )
        .await
        .unwrap();
    let secrets = SessionSecrets::generate().unwrap();
    storage
        .create_session(
            &account.account_ref,
            account.revision,
            secrets.session_ref().as_str(),
            secrets.token_digest(),
            secrets.csrf_digest(),
            SessionWindow::issued(1_721_800_000_300).unwrap(),
            1_721_800_000_300,
        )
        .await
        .unwrap();
    assert_eq!(storage.active_session_count().await.unwrap(), 1);

    let manifest = create_encrypted_backup(&storage, &backup, "correct horse battery")
        .await
        .expect("create encrypted backup");
    assert_eq!(manifest.storage_profile, "embedded");
    assert_eq!(manifest.payload_format, "sqlite-database");
    assert_eq!(manifest.raw_record_count, 2);
    assert_eq!(
        fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
        0o600
    );

    restore_encrypted_backup_sqlite(&backup, &restored, "correct horse battery")
        .await
        .expect("restore encrypted backup");
    assert_eq!(
        fs::metadata(&restored).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let restored_store = Storage::connect(StorageProfile::Sqlite {
        path: restored.clone(),
    })
    .await
    .expect("open restored database");
    assert_eq!(restored_store.edge_id().await.unwrap(), manifest.edge_id);
    assert_eq!(
        restored_store
            .raw_records("node-01", "epoch-01")
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(restored_store.active_session_count().await.unwrap(), 0);
}

#[tokio::test]
async fn encrypted_sqlite_backup_retains_pending_semantic_projection_work() {
    let directory = TempDir::new().unwrap();
    let database = directory.path().join("edge.db");
    let backup = directory.path().join("edge.iotkit-backup");
    let restored = directory.path().join("restored.db");
    let storage = Storage::connect(StorageProfile::Sqlite { path: database })
        .await
        .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    seed_pending_projection(&storage, "queue-node", "queue-epoch").await;
    assert_eq!(
        storage_status(&storage, 90)
            .await
            .unwrap()
            .pending_semantic_projection_count,
        1
    );
    create_encrypted_backup(&storage, &backup, "queue backup passphrase")
        .await
        .unwrap();
    drop(storage);

    restore_encrypted_backup_sqlite(&backup, &restored, "queue backup passphrase")
        .await
        .unwrap();
    let restored = Storage::connect(StorageProfile::Sqlite { path: restored })
        .await
        .unwrap();
    assert_eq!(
        storage_status(&restored, 90)
            .await
            .unwrap()
            .pending_semantic_projection_count,
        1
    );
    Semantics::new(restored.clone())
        .project_pending(1, registered_output_adapters())
        .await
        .unwrap();
    assert_eq!(
        storage_status(&restored, 90)
            .await
            .unwrap()
            .pending_semantic_projection_count,
        0
    );
}

#[tokio::test]
async fn backup_refuses_overwrite_wrong_passphrase_and_corruption() {
    let (directory, _database, storage) = populated_store().await;
    let backup = directory.path().join("edge.iotkit-backup");
    create_encrypted_backup(&storage, &backup, "correct horse battery")
        .await
        .unwrap();

    assert!(matches!(
        create_encrypted_backup(&storage, &backup, "correct horse battery").await,
        Err(BackupError::DestinationExists)
    ));
    assert!(
        restore_encrypted_backup_sqlite(
            &backup,
            &directory.path().join("wrong.db"),
            "incorrect passphrase"
        )
        .await
        .is_err()
    );

    let damaged = directory.path().join("damaged.iotkit-backup");
    let mut bytes = fs::read(&backup).unwrap();
    let index = bytes.len() - 10;
    bytes[index] ^= 0x80;
    fs::write(&damaged, bytes).unwrap();
    assert!(
        restore_encrypted_backup_sqlite(
            &damaged,
            &directory.path().join("damaged.db"),
            "correct horse battery"
        )
        .await
        .is_err()
    );

    assert!(matches!(
        restore_encrypted_backup_sqlite(&backup, &backup, "correct horse battery").await,
        Err(BackupError::DestinationExists)
    ));
}

#[tokio::test]
async fn postgres_custom_snapshot_round_trips_through_real_tools_when_required() {
    let source_dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(value) => value,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let restore_dsn = std::env::var("IOTKIT_TEST_POSTGRES_RESTORE_DSN")
        .expect("IOTKIT_TEST_POSTGRES_RESTORE_DSN is required with PostgreSQL");
    let directory = TempDir::new().unwrap();
    let backup = directory.path().join("postgres.iotkit-backup");
    let storage = Storage::connect(StorageProfile::Postgres {
        dsn: source_dsn.clone(),
    })
    .await
    .unwrap();
    storage.initialize_edge_identity(1).await.unwrap();
    let account = storage
        .create_account(
            AccountProvision {
                login_id: "postgres-owner".into(),
                display_name: "Postgres Owner".into(),
                role: AccountRole::SystemAdmin,
                password_hash: hash_password(&Password::new("postgres owner password").unwrap())
                    .unwrap(),
                must_change_password: false,
                require_unowned: true,
            },
            AuditActor::local_cli(),
            1,
        )
        .await
        .unwrap();
    let secrets = SessionSecrets::generate().unwrap();
    storage
        .create_session(
            &account.account_ref,
            account.revision,
            secrets.session_ref().as_str(),
            secrets.token_digest(),
            secrets.csrf_digest(),
            SessionWindow::issued(2).unwrap(),
            2,
        )
        .await
        .unwrap();
    seed_pending_projection(&storage, "postgres-node", "postgres-epoch").await;
    assert_eq!(
        storage_status(&storage, 90)
            .await
            .unwrap()
            .pending_semantic_projection_count,
        1
    );

    let manifest = create_encrypted_backup(&storage, &backup, "postgres backup secret")
        .await
        .unwrap();
    assert_eq!(manifest.storage_profile, "postgres");
    drop(storage);
    assert!(matches!(
        restore_encrypted_backup_sqlite(
            &backup,
            directory.path().join("wrong-profile.db"),
            "postgres backup secret",
        )
        .await,
        Err(BackupError::ProfileMismatch)
    ));
    assert!(matches!(
        restore_encrypted_backup_postgres(&backup, &source_dsn, "postgres backup secret").await,
        Err(BackupError::DestinationExists)
    ));

    assert!(
        restore_encrypted_backup_postgres(&backup, &restore_dsn, "incorrect postgres secret")
            .await
            .is_err()
    );
    let quarantined = match Storage::connect(StorageProfile::Postgres {
        dsn: restore_dsn.clone(),
    })
    .await
    {
        Ok(_) => panic!("incomplete restore must remain quarantined"),
        Err(error) => error,
    };
    assert!(matches!(quarantined, StorageError::RestoreIncomplete));

    let partial_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&restore_dsn)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE partial_restore_artifact(value BIGINT NOT NULL)")
        .execute(&partial_pool)
        .await
        .unwrap();
    partial_pool.close().await;

    let restored =
        restore_encrypted_backup_postgres(&backup, &restore_dsn, "postgres backup secret")
            .await
            .unwrap();
    assert_eq!(restored, manifest);
    let restored_storage = Storage::connect(StorageProfile::Postgres {
        dsn: restore_dsn.clone(),
    })
    .await
    .unwrap();
    assert_eq!(
        restored_storage
            .raw_records("postgres-node", "postgres-epoch")
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        storage_status(&restored_storage, 90)
            .await
            .unwrap()
            .pending_semantic_projection_count,
        1
    );
    Semantics::new(restored_storage.clone())
        .project_pending(1, registered_output_adapters())
        .await
        .unwrap();
    assert_eq!(
        storage_status(&restored_storage, 90)
            .await
            .unwrap()
            .pending_semantic_projection_count,
        0
    );
    assert_eq!(restored_storage.active_session_count().await.unwrap(), 0);
    drop(restored_storage);
    let inspection_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&restore_dsn)
        .await
        .unwrap();
    let partial_table: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.partial_restore_artifact')::text")
            .fetch_one(&inspection_pool)
            .await
            .unwrap();
    assert!(
        partial_table.is_none(),
        "retry must remove partial restore state before restoring"
    );
    inspection_pool.close().await;
}

#[tokio::test]
async fn restored_gap_enters_durable_recovery_hold_until_audited_archive_loss_acceptance() {
    let (directory, database, storage) = populated_store().await;
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", database.display()))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO edge_node_activations(edge_node_ref,edge_node_id,ledger_epoch,state,revision,\
         created_at,updated_at) VALUES('node-ref','node-01','epoch-01','active',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO edge_restore_events(restore_id,backup_id,restored_at,backup_created_at,\
         backup_edge_id,backup_schema_version,backup_sha256) \
         VALUES('restore-test','backup-test',2,1,(SELECT edge_id FROM edge_meta),5,?)",
    )
    .bind("0".repeat(64))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO edge_restore_cursor_checks(restore_id,edge_node_id,ledger_epoch,\
         backup_accepted_through,state,updated_at) \
         VALUES('restore-test','node-01','epoch-01',2,'pending',2)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let error = storage
        .accept_active_batch(AcceptBatch {
            edge_node_id: "node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: "gap".into(),
            received_at: 3,
            records: vec![RawRecord::new(5, br#"{"value":25}"#).unwrap()],
        })
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::ArchiveRecoveryRequired));
    assert_eq!(
        storage.edge_node("node-01").await.unwrap().state,
        EdgeNodeState::RecoveryHold
    );
    assert_eq!(
        storage
            .accepted_through("node-01", "epoch-01")
            .await
            .unwrap(),
        2
    );

    let edge_id = storage.edge_id().await.unwrap();
    storage
        .accept_restored_archive_loss(
            "node-01",
            "epoch-01",
            &edge_id,
            "original database unavailable",
            4,
        )
        .await
        .unwrap();
    assert_eq!(
        storage.edge_node("node-01").await.unwrap().state,
        EdgeNodeState::Active
    );
    assert_eq!(
        storage
            .accepted_through("node-01", "epoch-01")
            .await
            .unwrap(),
        4
    );
    drop(storage);
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", database.display()))
        .await
        .unwrap();
    let state: String = sqlx::query_scalar(
        "SELECT state FROM edge_restore_cursor_checks WHERE restore_id='restore-test'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_events \
         WHERE operation='edge_restore.accept_archive_loss'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, "archive_lost");
    assert_eq!(audit_count, 1);
    drop(pool);
    drop(directory);
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN; run scripts/test-edge-postgres.sh"]
async fn postgres_restored_gap_requires_audited_archive_loss_acceptance() {
    let dsn = match std::env::var("IOTKIT_TEST_POSTGRES_DSN") {
        Ok(value) => value,
        Err(_) if std::env::var_os("IOTKIT_REQUIRE_POSTGRES").is_some() => {
            panic!("IOTKIT_TEST_POSTGRES_DSN is required")
        }
        Err(_) => return,
    };
    let storage = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .expect("open PostgreSQL store");
    let edge_id = storage
        .initialize_edge_identity(1)
        .await
        .expect("initialize Edge identity");
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "node-pg".into(),
            ledger_epoch: "epoch-pg".into(),
            publication_id: "before-restore".into(),
            received_at: 1,
            records: vec![
                RawRecord::new(1, br#"{"value":20}"#).expect("record"),
                RawRecord::new(2, br#"{"value":21}"#).expect("record"),
            ],
        })
        .await
        .expect("seed restored cursor");
    let pool = sqlx::PgPool::connect(&dsn)
        .await
        .expect("open PostgreSQL inspection pool");
    sqlx::query(
        "INSERT INTO edge_node_activations(edge_node_ref,edge_node_id,ledger_epoch,state,revision,\
         created_at,updated_at) VALUES('node-ref-pg','node-pg','epoch-pg','active',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO edge_restore_events(restore_id,backup_id,restored_at,backup_created_at,\
         backup_edge_id,backup_schema_version,backup_sha256) \
         VALUES('restore-pg','backup-pg',2,1,(SELECT edge_id FROM edge_meta),7,$1)",
    )
    .bind("0".repeat(64))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO edge_restore_cursor_checks(restore_id,edge_node_id,ledger_epoch,\
         backup_accepted_through,state,updated_at) \
         VALUES('restore-pg','node-pg','epoch-pg',2,'pending',2)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let error = storage
        .accept_active_batch(AcceptBatch {
            edge_node_id: "node-pg".into(),
            ledger_epoch: "epoch-pg".into(),
            publication_id: "gap-pg".into(),
            received_at: 3,
            records: vec![RawRecord::new(5, br#"{"value":25}"#).unwrap()],
        })
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::ArchiveRecoveryRequired));
    assert_eq!(
        storage.edge_node("node-pg").await.unwrap().state,
        EdgeNodeState::RecoveryHold
    );
    assert_eq!(
        storage
            .accepted_through("node-pg", "epoch-pg")
            .await
            .unwrap(),
        2
    );

    drop(storage);
    let restarted = Storage::connect(StorageProfile::Postgres { dsn: dsn.clone() })
        .await
        .expect("restart on recovery-hold database");
    assert_eq!(
        restarted.edge_node("node-pg").await.unwrap().state,
        EdgeNodeState::RecoveryHold,
        "recovery hold must survive a process restart"
    );
    let mismatch = restarted
        .accept_restored_archive_loss(
            "node-pg",
            "epoch-pg",
            "edge-wrong-confirmation",
            "original PostgreSQL archive unavailable",
            4,
        )
        .await
        .unwrap_err();
    assert!(matches!(mismatch, StorageError::EdgeIdentityMismatch));
    restarted
        .accept_restored_archive_loss(
            "node-pg",
            "epoch-pg",
            &edge_id,
            "original PostgreSQL archive unavailable",
            5,
        )
        .await
        .expect("accept audited archive loss");
    assert_eq!(
        restarted.edge_node("node-pg").await.unwrap().state,
        EdgeNodeState::Active
    );
    assert_eq!(
        restarted
            .accepted_through("node-pg", "epoch-pg")
            .await
            .unwrap(),
        4
    );
    drop(restarted);

    let inspection = sqlx::PgPool::connect(&dsn)
        .await
        .expect("inspect PostgreSQL recovery result");
    let state: String = sqlx::query_scalar(
        "SELECT state FROM edge_restore_cursor_checks WHERE restore_id='restore-pg'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_events \
         WHERE operation='edge_restore.accept_archive_loss'",
    )
    .fetch_one(&inspection)
    .await
    .unwrap();
    assert_eq!(state, "archive_lost");
    assert_eq!(audit_count, 1);
    inspection.close().await;
}
