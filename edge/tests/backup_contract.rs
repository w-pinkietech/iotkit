use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

use iotkit_edge::{
    auth::{
        password::{Password, hash_password},
        principal::AccountRole,
        session::{SessionSecrets, SessionWindow},
    },
    backup::{
        BackupError, create_encrypted_backup, restore_encrypted_backup_postgres,
        restore_encrypted_backup_sqlite,
    },
    storage::{
        AcceptBatch, AccountProvision, AuditActor, EdgeNodeState, RawRecord, Storage, StorageError,
        StorageProfile,
    },
};
use tempfile::TempDir;

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
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "postgres-node".into(),
            ledger_epoch: "postgres-epoch".into(),
            publication_id: "postgres-publication".into(),
            received_at: 2,
            records: vec![RawRecord::new(1, br#"{"value":42}"#).unwrap()],
        })
        .await
        .unwrap();

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
