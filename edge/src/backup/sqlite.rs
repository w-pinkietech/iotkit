use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use uuid::Uuid;

use super::{BackupCursor, BackupError, BackupManifest, SnapshotInfo, crypto, unix_milliseconds};

pub(super) async fn snapshot(
    pool: &SqlitePool,
    source_path: &Path,
) -> Result<(PathBuf, SnapshotInfo), BackupError> {
    let directory = std::env::temp_dir().join(format!(".iotkit-edge-backup-{}", Uuid::new_v4()));
    fs::create_dir(&directory)?;
    crypto::protect_directory(&directory)?;
    let path = directory.join("snapshot.db");
    let result = async {
        ensure_snapshot_capacity(
            fs2::available_space(&directory)?,
            fs::metadata(source_path)?.len(),
        )?;
        let encoded = path.to_string_lossy().into_owned();
        sqlx::query("VACUUM INTO ?")
            .bind(encoded)
            .execute(pool)
            .await?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        File::open(&path)?.sync_all()?;
        let info = inspect_path(&path).await?;
        Ok((path.clone(), info))
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_dir_all(directory);
    }
    result
}

fn ensure_snapshot_capacity(available: u64, source_bytes: u64) -> Result<(), BackupError> {
    if available < source_bytes {
        Err(BackupError::InsufficientCapacity)
    } else {
        Ok(())
    }
}

pub(super) async fn inspect_path(path: &Path) -> Result<SnapshotInfo, BackupError> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    let check: String = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(&pool)
        .await?;
    if check != "ok" {
        return Err(BackupError::Integrity);
    }
    let edge_id: String = sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton = 1")
        .fetch_one(&pool)
        .await?;
    let schema_version: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = TRUE",
    )
    .fetch_one(&pool)
    .await?;
    let raw_record_count: i64 = sqlx::query_scalar("SELECT count(*) FROM raw_records")
        .fetch_one(&pool)
        .await?;
    let rows = sqlx::query(
        "SELECT edge_node_id, ledger_epoch, accepted_through FROM accepted_cursors \
         UNION ALL \
         SELECT activation.edge_node_id, activation.ledger_epoch, 0 \
         FROM edge_node_activations AS activation \
         WHERE activation.state = 'active' AND NOT EXISTS ( \
           SELECT 1 FROM accepted_cursors AS cursor \
           WHERE cursor.edge_node_id = activation.edge_node_id \
             AND cursor.ledger_epoch = activation.ledger_epoch) \
         ORDER BY edge_node_id, ledger_epoch",
    )
    .fetch_all(&pool)
    .await?;
    let cursors = rows
        .into_iter()
        .map(|row| {
            Ok(BackupCursor {
                edge_node_id: row.try_get("edge_node_id")?,
                ledger_epoch: row.try_get("ledger_epoch")?,
                accepted_through: row.try_get("accepted_through")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    pool.close().await;
    Ok(SnapshotInfo {
        storage_profile: "embedded",
        payload_format: "sqlite-database",
        edge_id,
        schema_version,
        raw_record_count,
        cursors,
    })
}

pub(super) async fn restore(
    source: &Path,
    destination: &Path,
    passphrase: &str,
) -> Result<BackupManifest, BackupError> {
    crypto::ensure_absent(destination)?;
    let payload = crypto::temporary_sibling(destination, "restore-payload")?;
    let source = source.to_owned();
    let passphrase = passphrase.to_owned();
    let decrypt_path = payload.clone();
    tokio::task::spawn_blocking(move || crypto::decrypt(&source, &decrypt_path, &passphrase))
        .await
        .map_err(|_| BackupError::Worker)??;

    let database = crypto::temporary_sibling(destination, "restore-database")?;
    let result = async {
        let (manifest, hash) = extract_payload(&payload, &database)?;
        if !hash.eq_ignore_ascii_case(&manifest.database_sha256) {
            return Err(BackupError::Integrity);
        }
        if manifest.storage_profile != "embedded" || manifest.payload_format != "sqlite-database" {
            return Err(BackupError::ProfileMismatch);
        }
        let inspected = inspect_path(&database).await?;
        validate_manifest(&manifest, &inspected)?;
        prepare_restored_database(&database, &manifest).await?;
        crypto::publish_new_file(&database, destination)?;
        Ok(manifest)
    }
    .await;
    let _ = fs::remove_file(&payload);
    if result.is_err() {
        let _ = fs::remove_file(&database);
        let _ = fs::remove_file(format!("{}-wal", database.display()));
        let _ = fs::remove_file(format!("{}-shm", database.display()));
    }
    result
}

pub(super) fn extract_payload(
    payload_path: &Path,
    database_path: &Path,
) -> Result<(BackupManifest, String), BackupError> {
    let mut payload = File::open(payload_path)?;
    let mut length = [0_u8; 4];
    payload.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > 1024 * 1024 {
        return Err(BackupError::InvalidManifest);
    }
    let mut encoded = vec![0_u8; length];
    payload.read_exact(&mut encoded)?;
    let manifest: BackupManifest =
        serde_json::from_slice(&encoded).map_err(|_| BackupError::InvalidManifest)?;
    manifest.validate()?;
    let mut database = crypto::private_new_file(database_path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let count = payload.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        database.write_all(&buffer[..count])?;
    }
    database.sync_all()?;
    Ok((manifest, format!("{:x}", hash.finalize())))
}

async fn prepare_restored_database(
    path: &Path,
    manifest: &BackupManifest,
) -> Result<(), BackupError> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .foreign_keys(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Full);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    let mut tx = pool.begin().await?;
    let now = unix_milliseconds()?;
    sqlx::query("UPDATE edge_sessions SET revoked_at = ? WHERE revoked_at IS NULL")
        .bind(now)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT OR IGNORE INTO edge_backup_events(backup_id, created_at, destination_name, \
         database_sha256, raw_record_count) VALUES(?, ?, 'restored-backup', ?, ?)",
    )
    .bind(&manifest.backup_id)
    .bind(manifest.created_at)
    .bind(&manifest.database_sha256)
    .bind(manifest.raw_record_count)
    .execute(&mut *tx)
    .await?;
    let restore_id = format!("restore_{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO edge_restore_events(restore_id, backup_id, restored_at, backup_created_at, \
         backup_edge_id, backup_schema_version, backup_sha256) VALUES(?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&restore_id)
    .bind(&manifest.backup_id)
    .bind(now)
    .bind(manifest.created_at)
    .bind(&manifest.edge_id)
    .bind(manifest.schema_version)
    .bind(&manifest.database_sha256)
    .execute(&mut *tx)
    .await?;
    for cursor in &manifest.cursors {
        sqlx::query(
            "INSERT INTO edge_restore_cursor_checks(restore_id, edge_node_id, ledger_epoch, \
             backup_accepted_through, state, updated_at) VALUES(?, ?, ?, ?, 'pending', ?)",
        )
        .bind(&restore_id)
        .bind(&cursor.edge_node_id)
        .bind(&cursor.ledger_epoch)
        .bind(cursor.accepted_through)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO audit_events(occurred_at, actor_class, actor_ref, operation, resource_ref, \
         outcome, summary_json) VALUES(?, 'local_cli', 'local-cli', 'edge_backup.restore', ?, \
         'success', ?)",
    )
    .bind(now)
    .bind(&restore_id)
    .bind(serde_json::to_vec(&serde_json::json!({
        "backup_id": manifest.backup_id,
        "database_sha256": manifest.database_sha256,
        "raw_record_count": manifest.raw_record_count,
    }))?)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&pool)
        .await?;
    pool.close().await;
    File::open(path)?.sync_all()?;
    Ok(())
}

pub(super) fn validate_manifest(
    manifest: &BackupManifest,
    snapshot: &SnapshotInfo,
) -> Result<(), BackupError> {
    if manifest.edge_id != snapshot.edge_id
        || manifest.schema_version != snapshot.schema_version
        || manifest.raw_record_count != snapshot.raw_record_count
        || manifest.cursors != snapshot.cursors
    {
        return Err(BackupError::ManifestMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_preflight_rejects_a_snapshot_larger_than_available_space() {
        assert!(matches!(
            ensure_snapshot_capacity(99, 100),
            Err(BackupError::InsufficientCapacity)
        ));
        assert!(ensure_snapshot_capacity(100, 100).is_ok());
    }
}
