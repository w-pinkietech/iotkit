use std::{
    collections::HashMap,
    fs::{self, File},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use url::Url;
use uuid::Uuid;

use super::{
    BackupCursor, BackupError, BackupManifest, SnapshotInfo, crypto, sqlite, unix_milliseconds,
};

pub(super) async fn snapshot(
    pool: &PgPool,
    dsn: &str,
) -> Result<(PathBuf, SnapshotInfo), BackupError> {
    let mut transaction = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *transaction)
        .await?;
    let snapshot_id: String = sqlx::query_scalar("SELECT pg_export_snapshot()")
        .fetch_one(&mut *transaction)
        .await?;
    let info = inspect_transaction(&mut transaction).await?;
    let connection = PgEnvironment::parse(dsn)?;
    let directory =
        std::env::temp_dir().join(format!(".iotkit-edge-postgres-backup-{}", Uuid::new_v4()));
    fs::create_dir(&directory)?;
    crypto::protect_directory(&directory)?;
    let path = directory.join("snapshot.dump");
    let result = async {
        let output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        let command_path = path.clone();
        let status = tokio::task::spawn_blocking(move || {
            let mut command = Command::new("pg_dump");
            command
                .arg("--format=custom")
                .arg("--no-owner")
                .arg("--no-privileges")
                .arg(format!("--snapshot={snapshot_id}"))
                .envs(connection.variables)
                .stdout(Stdio::from(output))
                .stderr(Stdio::null());
            command.status()
        })
        .await
        .map_err(|_| BackupError::Worker)??;
        if !status.success() {
            return Err(BackupError::PostgresTool("pg_dump failed".into()));
        }
        File::open(&command_path)?.sync_all()?;
        transaction.rollback().await?;
        Ok((path.clone(), info))
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_dir_all(directory);
    }
    result
}

pub(super) async fn restore(
    source: &Path,
    target_dsn: &str,
    passphrase: &str,
) -> Result<BackupManifest, BackupError> {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(target_dsn)
        .await?;
    let table_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_tables \
         WHERE schemaname NOT IN ('pg_catalog', 'information_schema')",
    )
    .fetch_one(&pool)
    .await?;
    if table_count != 0 {
        return Err(BackupError::DestinationExists);
    }
    let acquired: bool = sqlx::query_scalar(
        "SELECT pg_try_advisory_lock(\
         hashtextextended('iotkit-edge-storage:' || current_database(), 0))",
    )
    .fetch_one(&pool)
    .await?;
    if !acquired {
        return Err(BackupError::DestinationExists);
    }

    let directory =
        std::env::temp_dir().join(format!(".iotkit-edge-postgres-restore-{}", Uuid::new_v4()));
    fs::create_dir(&directory)?;
    crypto::protect_directory(&directory)?;
    let payload = directory.join("payload");
    let dump = directory.join("snapshot.dump");
    let result = async {
        let source_path = source.to_owned();
        let passphrase = passphrase.to_owned();
        let decrypt_path = payload.clone();
        tokio::task::spawn_blocking(move || {
            crypto::decrypt(&source_path, &decrypt_path, &passphrase)
        })
        .await
        .map_err(|_| BackupError::Worker)??;
        let (manifest, hash) = sqlite::extract_payload(&payload, &dump)?;
        if manifest.storage_profile != "postgres" || manifest.payload_format != "postgres-custom" {
            return Err(BackupError::ProfileMismatch);
        }
        if !hash.eq_ignore_ascii_case(&manifest.database_sha256) {
            return Err(BackupError::Integrity);
        }

        let connection = PgEnvironment::parse(target_dsn)?;
        let database_name = connection.database_name.clone();
        let restore_dump = dump.clone();
        let output = tokio::task::spawn_blocking(move || {
            Command::new("pg_restore")
                .arg("--dbname")
                .arg(database_name)
                .arg("--no-owner")
                .arg("--no-privileges")
                .arg("--exit-on-error")
                .arg("--single-transaction")
                .arg(restore_dump)
                .envs(connection.variables)
                .stdout(Stdio::null())
                .output()
        })
        .await
        .map_err(|_| BackupError::Worker)??;
        if !output.status.success() {
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            return Err(BackupError::PostgresTool(format!(
                "pg_restore failed: {}",
                diagnostic.lines().next().unwrap_or("no diagnostic")
            )));
        }
        let inspected = inspect_pool(&pool).await?;
        sqlite::validate_manifest(&manifest, &inspected)?;
        prepare_restored_pool(&pool, &manifest).await?;
        Ok(manifest)
    }
    .await;
    pool.close().await;
    let _ = fs::remove_dir_all(directory);
    result
}

async fn inspect_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<SnapshotInfo, BackupError> {
    let edge_id: String = sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton = 1")
        .fetch_one(&mut **transaction)
        .await?;
    let schema_version: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = TRUE",
    )
    .fetch_one(&mut **transaction)
    .await?;
    let raw_record_count: i64 = sqlx::query_scalar("SELECT count(*) FROM raw_records")
        .fetch_one(&mut **transaction)
        .await?;
    let cursors = load_cursors(&mut **transaction).await?;
    Ok(SnapshotInfo {
        storage_profile: "postgres",
        payload_format: "postgres-custom",
        edge_id,
        schema_version,
        raw_record_count,
        cursors,
    })
}

async fn inspect_pool(pool: &PgPool) -> Result<SnapshotInfo, BackupError> {
    let edge_id: String = sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton = 1")
        .fetch_one(pool)
        .await?;
    let schema_version: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = TRUE",
    )
    .fetch_one(pool)
    .await?;
    let raw_record_count: i64 = sqlx::query_scalar("SELECT count(*) FROM raw_records")
        .fetch_one(pool)
        .await?;
    let cursors = load_cursors(pool).await?;
    Ok(SnapshotInfo {
        storage_profile: "postgres",
        payload_format: "postgres-custom",
        edge_id,
        schema_version,
        raw_record_count,
        cursors,
    })
}

async fn load_cursors<'e, E>(executor: E) -> Result<Vec<BackupCursor>, BackupError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query(
        "SELECT edge_node_id, ledger_epoch, accepted_through FROM accepted_cursors \
         UNION ALL SELECT activation.edge_node_id, activation.ledger_epoch, 0 \
         FROM edge_node_activations AS activation \
         WHERE activation.state = 'active' AND NOT EXISTS ( \
           SELECT 1 FROM accepted_cursors AS cursor \
           WHERE cursor.edge_node_id = activation.edge_node_id \
             AND cursor.ledger_epoch = activation.ledger_epoch) \
         ORDER BY edge_node_id, ledger_epoch",
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(BackupCursor {
                edge_node_id: row.try_get("edge_node_id")?,
                ledger_epoch: row.try_get("ledger_epoch")?,
                accepted_through: row.try_get("accepted_through")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
}

async fn prepare_restored_pool(
    pool: &PgPool,
    manifest: &BackupManifest,
) -> Result<(), BackupError> {
    let now = unix_milliseconds()?;
    let restore_id = format!("restore_{}", Uuid::new_v4().simple());
    let mut transaction = pool.begin().await?;
    sqlx::query("UPDATE edge_sessions SET revoked_at = $1 WHERE revoked_at IS NULL")
        .bind(now)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO edge_backup_events(backup_id, created_at, destination_name, \
         database_sha256, raw_record_count) VALUES($1, $2, 'restored-backup', $3, $4) \
         ON CONFLICT(backup_id) DO NOTHING",
    )
    .bind(&manifest.backup_id)
    .bind(manifest.created_at)
    .bind(&manifest.database_sha256)
    .bind(manifest.raw_record_count)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO edge_restore_events(restore_id, backup_id, restored_at, backup_created_at, \
         backup_edge_id, backup_schema_version, backup_sha256) \
         VALUES($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&restore_id)
    .bind(&manifest.backup_id)
    .bind(now)
    .bind(manifest.created_at)
    .bind(&manifest.edge_id)
    .bind(manifest.schema_version)
    .bind(&manifest.database_sha256)
    .execute(&mut *transaction)
    .await?;
    for cursor in &manifest.cursors {
        sqlx::query(
            "INSERT INTO edge_restore_cursor_checks(restore_id, edge_node_id, ledger_epoch, \
             backup_accepted_through, state, updated_at) \
             VALUES($1, $2, $3, $4, 'pending', $5)",
        )
        .bind(&restore_id)
        .bind(&cursor.edge_node_id)
        .bind(&cursor.ledger_epoch)
        .bind(cursor.accepted_through)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query(
        "INSERT INTO audit_events(occurred_at, actor_class, actor_ref, operation, resource_ref, \
         outcome, summary_json) VALUES($1, 'local_cli', 'local-cli', 'edge_backup.restore', $2, \
         'success', $3)",
    )
    .bind(now)
    .bind(&restore_id)
    .bind(serde_json::json!({
        "backup_id": manifest.backup_id,
        "database_sha256": manifest.database_sha256,
        "raw_record_count": manifest.raw_record_count,
    }))
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

struct PgEnvironment {
    database_name: String,
    variables: HashMap<String, String>,
}

impl PgEnvironment {
    fn parse(dsn: &str) -> Result<Self, BackupError> {
        let parsed = Url::parse(dsn).map_err(|_| BackupError::PostgresConfiguration)?;
        if !matches!(parsed.scheme(), "postgres" | "postgresql")
            || parsed.host_str().is_none()
            || parsed.path().trim_matches('/').is_empty()
        {
            return Err(BackupError::PostgresConfiguration);
        }
        let mut variables = HashMap::new();
        variables.insert("PGHOST".into(), parsed.host_str().unwrap().into());
        variables.insert("PGPORT".into(), parsed.port().unwrap_or(5432).to_string());
        let database_name = parsed.path().trim_start_matches('/').to_owned();
        variables.insert("PGDATABASE".into(), database_name.clone());
        if !parsed.username().is_empty() {
            variables.insert("PGUSER".into(), parsed.username().into());
        }
        if let Some(password) = parsed.password() {
            variables.insert("PGPASSWORD".into(), password.into());
        }
        let allowed = [
            ("sslmode", "PGSSLMODE"),
            ("sslcert", "PGSSLCERT"),
            ("sslkey", "PGSSLKEY"),
            ("sslrootcert", "PGSSLROOTCERT"),
            ("connect_timeout", "PGCONNECT_TIMEOUT"),
            ("target_session_attrs", "PGTARGETSESSIONATTRS"),
            ("application_name", "PGAPPNAME"),
        ];
        for (key, value) in parsed.query_pairs() {
            let Some((_, environment)) = allowed.iter().find(|(allowed, _)| *allowed == key) else {
                return Err(BackupError::PostgresConfiguration);
            };
            variables.insert((*environment).into(), value.into_owned());
        }
        Ok(Self {
            database_name,
            variables,
        })
    }
}
