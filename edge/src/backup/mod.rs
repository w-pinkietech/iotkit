//! Consistent, authenticated operational backup and restore.

mod crypto;
mod postgres;
mod sqlite;

use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::storage::{OperationBackend, Storage};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupCursor {
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub accepted_through: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupManifest {
    pub format_version: u32,
    pub storage_profile: String,
    pub payload_format: String,
    pub backup_id: String,
    pub created_at: i64,
    pub edge_id: String,
    pub schema_version: i64,
    pub raw_record_count: i64,
    pub cursors: Vec<BackupCursor>,
    pub database_sha256: String,
}

impl BackupManifest {
    fn validate(&self) -> Result<(), BackupError> {
        if self.format_version != 1
            || self.backup_id.is_empty()
            || self.created_at < 0
            || self.edge_id.is_empty()
            || self.schema_version < 1
            || self.raw_record_count < 0
            || !matches!(self.storage_profile.as_str(), "embedded" | "postgres")
            || self.database_sha256.len() != 64
            || !self
                .database_sha256
                .bytes()
                .all(|value| value.is_ascii_hexdigit())
            || self.cursors.iter().any(|cursor| {
                cursor.edge_node_id.is_empty()
                    || cursor.ledger_epoch.is_empty()
                    || cursor.accepted_through < 0
            })
        {
            return Err(BackupError::InvalidManifest);
        }
        Ok(())
    }
}

#[derive(Debug)]
struct SnapshotInfo {
    storage_profile: &'static str,
    payload_format: &'static str,
    edge_id: String,
    schema_version: i64,
    raw_record_count: i64,
    cursors: Vec<BackupCursor>,
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("backup passphrase must contain between 12 and 1024 characters")]
    InvalidPassphrase,
    #[error("backup or restore destination already exists")]
    DestinationExists,
    #[error("insufficient capacity for the backup staging operation")]
    InsufficientCapacity,
    #[error("backup storage profile does not match the restore destination")]
    ProfileMismatch,
    #[error("unsupported or damaged Edge backup format")]
    InvalidContainer,
    #[error("Edge backup authentication failed; the passphrase is wrong or the backup changed")]
    Authentication,
    #[error("invalid Edge backup manifest")]
    InvalidManifest,
    #[error("Edge backup manifest does not match the restored database")]
    ManifestMismatch,
    #[error("Edge backup integrity check failed")]
    Integrity,
    #[error("backup cryptographic operation failed")]
    Cryptography,
    #[error("backup worker stopped unexpectedly")]
    Worker,
    #[error("PostgreSQL backup configuration is not supported")]
    PostgresConfiguration,
    #[error("PostgreSQL backup tool failed: {0}")]
    PostgresTool(String),
    #[error("backup I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("backup database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("backup manifest encoding error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("system clock is before the Unix epoch")]
    Clock,
}

pub async fn create_encrypted_backup(
    storage: &Storage,
    destination: impl AsRef<Path>,
    passphrase: &str,
) -> Result<BackupManifest, BackupError> {
    validate_passphrase(passphrase)?;
    let destination = destination.as_ref().to_owned();
    crypto::ensure_absent(&destination)?;

    let (snapshot_path, info) = match storage.operation_backend() {
        OperationBackend::Sqlite { pool, path } => sqlite::snapshot(pool, path).await?,
        OperationBackend::Postgres { pool, dsn } => postgres::snapshot(pool, dsn).await?,
    };
    let result = async {
        let destination_parent = destination.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(destination_parent)?;
        ensure_encryption_capacity(
            fs2::available_space(destination_parent)?,
            fs::metadata(&snapshot_path)?.len(),
        )?;
        let digest = file_sha256(&snapshot_path)?;
        let manifest = BackupManifest {
            format_version: 1,
            storage_profile: info.storage_profile.into(),
            payload_format: info.payload_format.into(),
            backup_id: format!("backup_{}", Uuid::new_v4().simple()),
            created_at: unix_milliseconds()?,
            edge_id: info.edge_id,
            schema_version: info.schema_version,
            raw_record_count: info.raw_record_count,
            cursors: info.cursors,
            database_sha256: digest,
        };
        let encoded = serde_json::to_vec(&manifest)?;
        let snapshot = snapshot_path.clone();
        let output = destination.clone();
        let secret = passphrase.to_owned();
        tokio::task::spawn_blocking(move || crypto::encrypt(&output, &encoded, &snapshot, &secret))
            .await
            .map_err(|_| BackupError::Worker)??;
        if let Err(error) = record_completed_backup(storage, &destination, &manifest).await {
            let _ = fs::remove_file(&destination);
            return Err(error);
        }
        Ok(manifest)
    }
    .await;
    if let Some(parent) = snapshot_path.parent() {
        let _ = fs::remove_dir_all(parent);
    }
    result
}

pub async fn restore_encrypted_backup_sqlite(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    passphrase: &str,
) -> Result<BackupManifest, BackupError> {
    validate_passphrase(passphrase)?;
    sqlite::restore(source.as_ref(), destination.as_ref(), passphrase).await
}

pub async fn restore_encrypted_backup_postgres(
    source: impl AsRef<Path>,
    target_dsn: &str,
    passphrase: &str,
) -> Result<BackupManifest, BackupError> {
    validate_passphrase(passphrase)?;
    postgres::restore(source.as_ref(), target_dsn, passphrase).await
}

fn validate_passphrase(passphrase: &str) -> Result<(), BackupError> {
    if !(12..=1024).contains(&passphrase.chars().count()) {
        return Err(BackupError::InvalidPassphrase);
    }
    Ok(())
}

fn ensure_encryption_capacity(available: u64, payload_bytes: u64) -> Result<(), BackupError> {
    const CONTAINER_ALLOWANCE_BYTES: u64 = 128 * 1024;
    let required = payload_bytes
        .checked_add(CONTAINER_ALLOWANCE_BYTES)
        .ok_or(BackupError::InsufficientCapacity)?;
    if available < required {
        Err(BackupError::InsufficientCapacity)
    } else {
        Ok(())
    }
}

async fn record_completed_backup(
    storage: &Storage,
    destination: &Path,
    manifest: &BackupManifest,
) -> Result<(), BackupError> {
    let destination_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("encrypted-backup");
    match storage.operation_backend() {
        OperationBackend::Sqlite { pool, .. } => {
            let mut transaction = pool.begin().await?;
            sqlx::query(
                "INSERT INTO edge_backup_events(backup_id, created_at, destination_name, \
                 database_sha256, raw_record_count) VALUES(?, ?, ?, ?, ?)",
            )
            .bind(&manifest.backup_id)
            .bind(manifest.created_at)
            .bind(destination_name)
            .bind(&manifest.database_sha256)
            .bind(manifest.raw_record_count)
            .execute(&mut *transaction)
            .await?;
            for cursor in &manifest.cursors {
                sqlx::query(
                    "INSERT INTO edge_backup_cursors(backup_id, edge_node_id, ledger_epoch, \
                     accepted_through) VALUES(?, ?, ?, ?)",
                )
                .bind(&manifest.backup_id)
                .bind(&cursor.edge_node_id)
                .bind(&cursor.ledger_epoch)
                .bind(cursor.accepted_through)
                .execute(&mut *transaction)
                .await?;
            }
            let summary = serde_json::to_vec(&serde_json::json!({
                "backup_id": manifest.backup_id,
                "database_sha256": manifest.database_sha256,
                "raw_record_count": manifest.raw_record_count,
            }))?;
            sqlx::query(
                "INSERT INTO audit_events(occurred_at, actor_class, actor_ref, operation, \
                 resource_ref, outcome, summary_json) \
                 VALUES(?, 'local_cli', 'local-cli', 'edge_backup.create', ?, 'success', ?)",
            )
            .bind(manifest.created_at)
            .bind(&manifest.backup_id)
            .bind(summary)
            .execute(&mut *transaction)
            .await?;
            transaction.commit().await?;
        }
        OperationBackend::Postgres { pool, .. } => {
            let mut transaction = pool.begin().await?;
            sqlx::query(
                "INSERT INTO edge_backup_events(backup_id, created_at, destination_name, \
                 database_sha256, raw_record_count) VALUES($1, $2, $3, $4, $5)",
            )
            .bind(&manifest.backup_id)
            .bind(manifest.created_at)
            .bind(destination_name)
            .bind(&manifest.database_sha256)
            .bind(manifest.raw_record_count)
            .execute(&mut *transaction)
            .await?;
            for cursor in &manifest.cursors {
                sqlx::query(
                    "INSERT INTO edge_backup_cursors(backup_id, edge_node_id, ledger_epoch, \
                     accepted_through) VALUES($1, $2, $3, $4)",
                )
                .bind(&manifest.backup_id)
                .bind(&cursor.edge_node_id)
                .bind(&cursor.ledger_epoch)
                .bind(cursor.accepted_through)
                .execute(&mut *transaction)
                .await?;
            }
            sqlx::query(
                "INSERT INTO audit_events(occurred_at, actor_class, actor_ref, operation, \
                 resource_ref, outcome, summary_json) \
                 VALUES($1, 'local_cli', 'local-cli', 'edge_backup.create', $2, 'success', $3)",
            )
            .bind(manifest.created_at)
            .bind(&manifest.backup_id)
            .bind(serde_json::json!({
                "backup_id": manifest.backup_id,
                "database_sha256": manifest.database_sha256,
                "raw_record_count": manifest.raw_record_count,
            }))
            .execute(&mut *transaction)
            .await?;
            transaction.commit().await?;
        }
    }
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String, BackupError> {
    let mut input = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    std::io::copy(&mut input, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn unix_milliseconds() -> Result<i64, BackupError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| BackupError::Clock)?;
    i64::try_from(duration.as_millis()).map_err(|_| BackupError::Clock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encryption_capacity_includes_container_overhead() {
        assert!(matches!(
            ensure_encryption_capacity(256 * 1024, 256 * 1024),
            Err(BackupError::InsufficientCapacity)
        ));
        assert!(ensure_encryption_capacity(384 * 1024, 256 * 1024).is_ok());
    }
}
