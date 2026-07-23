use std::{
    fmt,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{
    PgPool, Postgres, Row, Sqlite, SqlitePool, Transaction,
    pool::PoolConnection,
    postgres::PgPoolOptions,
    postgres::PgRow,
    sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
    },
};
use tokio::sync::Mutex;

mod activation;
mod auth;
mod recovery;
mod semantic_output;
pub use activation::{ActivationCommand, DescriptorApply, EdgeNode, EdgeNodeState};
pub use auth::{
    Account, AccountCredential, AccountProvision, AuditActor, AuditEvent, StoredSession,
};
pub use semantic_output::{ClaimedOutput, OutputMark};

static SQLITE_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");
static POSTGRES_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");

#[derive(Clone)]
pub enum StorageProfile {
    Sqlite { path: PathBuf },
    Postgres { dsn: String },
}

impl fmt::Debug for StorageProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite { path } => formatter
                .debug_struct("Sqlite")
                .field("path", path)
                .finish(),
            Self::Postgres { .. } => formatter
                .debug_struct("Postgres")
                .field("dsn", &"[REDACTED]")
                .finish(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawRecord {
    pub pub_seq: i64,
    pub record_json: Vec<u8>,
}

impl RawRecord {
    pub fn new(pub_seq: i64, record_json: impl AsRef<[u8]>) -> Result<Self, StorageError> {
        if pub_seq <= 0 {
            return Err(StorageError::InvalidRecord(
                "pub_seq must be greater than zero".into(),
            ));
        }
        let record_json = compact_json(record_json.as_ref())?;
        Ok(Self {
            pub_seq,
            record_json,
        })
    }

    fn encoded(&self) -> Vec<u8> {
        self.record_json.clone()
    }
}

#[derive(Debug, Clone)]
pub struct AcceptBatch {
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub publication_id: String,
    pub received_at: i64,
    pub records: Vec<RawRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRawRecord {
    pub pub_seq: i64,
    pub publication_id: String,
    pub record_json: Vec<u8>,
    pub received_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptedCursor {
    pub accepted_through: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptResult {
    pub accepted_through: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("storage migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("open storage operation guard: {0}")]
    Guard(std::io::Error),
    #[error("storage is already in use by another IoTKit Edge process")]
    AlreadyInUse,
    #[error("PostgreSQL restore is incomplete and the database is quarantined")]
    RestoreIncomplete,
    #[error(
        "the database uses the unsupported pre-Rust IoTKit Edge schema; start with a fresh database"
    )]
    UnsupportedLegacySchema,
    #[error("invalid raw record: {0}")]
    InvalidRecord(String),
    #[error("encode raw record: {0}")]
    EncodeRecord(serde_json::Error),
    #[error("raw sequence gap: expected {expected}, received {actual}")]
    SequenceGap { expected: i64, actual: i64 },
    #[error("raw record at sequence {sequence} conflicts with the accepted record")]
    RecordConflict { sequence: i64 },
    #[error("Edge Node is not active for IoTKit Edge custody")]
    EdgeNodeNotActive,
    #[error("restored archive cursor requires operator recovery review")]
    ArchiveRecoveryRequired,
    #[error("no restored archive-loss decision is pending for this Edge Node stream")]
    NoArchiveLossDecision,
    #[error("confirmed IoTKit Edge ID does not match this IoTKit Edge")]
    EdgeIdentityMismatch,
    #[error("Edge Node activation result conflicts with the pending activation")]
    ActivationConflict,
    #[error("descriptor revision conflicts with the previously accepted content")]
    DescriptorConflict,
    #[error("Edge account was not found")]
    AccountNotFound,
    #[error("Edge account revision does not match")]
    RevisionMismatch,
    #[error("the last active system administrator cannot be disabled or demoted")]
    LastSystemAdmin,
    #[error("Edge session was not found or is no longer active")]
    SessionNotFound,
    #[error("Edge account already exists")]
    AccountConflict,
    #[error("Edge account operation is invalid: {0}")]
    InvalidAccount(String),
    #[error("semantic operation is invalid: {0}")]
    InvalidSemantic(String),
    #[error("output operation is invalid: {0}")]
    InvalidOutput(String),
    #[error("semantic or output resource was not found")]
    SemanticNotFound,
}

#[derive(Clone)]
pub struct Storage {
    inner: Arc<StorageInner>,
}

enum StorageInner {
    Sqlite {
        pool: SqlitePool,
        path: PathBuf,
        _guard: File,
    },
    Postgres {
        pool: PgPool,
        dsn: String,
        _guard: Mutex<PoolConnection<Postgres>>,
    },
}

pub(crate) enum OperationBackend<'a> {
    Sqlite {
        pool: &'a SqlitePool,
        path: &'a Path,
    },
    Postgres {
        pool: &'a PgPool,
        dsn: &'a str,
    },
}

impl Storage {
    pub async fn connect(profile: StorageProfile) -> Result<Self, StorageError> {
        match profile {
            StorageProfile::Sqlite { path } => {
                let guard = acquire_sqlite_guard(&path)?;
                let options = SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .foreign_keys(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Full)
                    .busy_timeout(Duration::from_secs(5));
                let pool = SqlitePoolOptions::new()
                    .max_connections(1)
                    .connect_with(options)
                    .await?;
                reject_legacy_sqlite_schema(&pool).await?;
                SQLITE_MIGRATOR.run(&pool).await?;
                Ok(Self {
                    inner: Arc::new(StorageInner::Sqlite {
                        pool,
                        path,
                        _guard: guard,
                    }),
                })
            }
            StorageProfile::Postgres { dsn } => {
                let pool = PgPoolOptions::new()
                    .max_connections(20)
                    .connect(&dsn)
                    .await?;
                let guard = acquire_postgres_guard(&pool).await?;
                reject_incomplete_postgres_restore(&pool).await?;
                validate_postgres_durability(&pool).await?;
                reject_legacy_postgres_schema(&pool).await?;
                POSTGRES_MIGRATOR.run(&pool).await?;
                Ok(Self {
                    inner: Arc::new(StorageInner::Postgres {
                        pool,
                        dsn,
                        _guard: Mutex::new(guard),
                    }),
                })
            }
        }
    }

    pub(crate) fn operation_backend(&self) -> OperationBackend<'_> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, path, .. } => OperationBackend::Sqlite {
                pool,
                path: path.as_path(),
            },
            StorageInner::Postgres { pool, dsn, .. } => OperationBackend::Postgres { pool, dsn },
        }
    }

    pub async fn active_session_count(&self) -> Result<i64, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => Ok(sqlx::query_scalar(
                "SELECT count(*) FROM edge_sessions WHERE revoked_at IS NULL",
            )
            .fetch_one(pool)
            .await?),
            StorageInner::Postgres { pool, .. } => Ok(sqlx::query_scalar(
                "SELECT count(*) FROM edge_sessions WHERE revoked_at IS NULL",
            )
            .fetch_one(pool)
            .await?),
        }
    }

    pub async fn accept_batch(&self, batch: AcceptBatch) -> Result<AcceptResult, StorageError> {
        validate_batch(&batch)?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut transaction = pool.begin().await?;
                let accepted_through = accept_sqlite(&mut transaction, &batch).await?;
                transaction.commit().await?;
                Ok(AcceptResult { accepted_through })
            }
            StorageInner::Postgres { pool, .. } => {
                let mut transaction = pool.begin().await?;
                let accepted_through = accept_postgres(&mut transaction, &batch).await?;
                transaction.commit().await?;
                Ok(AcceptResult { accepted_through })
            }
        }
    }

    pub async fn accept_active_batch(
        &self,
        batch: AcceptBatch,
    ) -> Result<AcceptResult, StorageError> {
        validate_batch(&batch)?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut transaction = pool.begin().await?;
                let active: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM edge_node_activations \
                     WHERE edge_node_id = ? AND ledger_epoch = ?",
                )
                .bind(&batch.edge_node_id)
                .bind(&batch.ledger_epoch)
                .fetch_optional(&mut *transaction)
                .await?;
                if active.as_deref() != Some("active") {
                    return Err(StorageError::EdgeNodeNotActive);
                }
                let pending: Option<String> = sqlx::query_scalar(
                    "SELECT checks.restore_id FROM edge_restore_cursor_checks AS checks \
                     JOIN edge_restore_events AS events ON events.restore_id=checks.restore_id \
                     WHERE checks.edge_node_id=? AND checks.ledger_epoch=? \
                       AND checks.state='pending' \
                     ORDER BY events.restored_at DESC, checks.restore_id DESC LIMIT 1",
                )
                .bind(&batch.edge_node_id)
                .bind(&batch.ledger_epoch)
                .fetch_optional(&mut *transaction)
                .await?;
                let cursor: i64 = sqlx::query_scalar(
                    "SELECT accepted_through FROM accepted_cursors \
                     WHERE edge_node_id=? AND ledger_epoch=?",
                )
                .bind(&batch.edge_node_id)
                .bind(&batch.ledger_epoch)
                .fetch_optional(&mut *transaction)
                .await?
                .unwrap_or(0);
                if let Some(restore_id) = pending.as_deref()
                    && batch.records[0].pub_seq > cursor + 1
                {
                    sqlx::query(
                        "UPDATE edge_restore_cursor_checks SET state='recovery_required', \
                         observed_cursor_start=?, updated_at=? WHERE restore_id=? \
                         AND edge_node_id=? AND ledger_epoch=? AND state='pending'",
                    )
                    .bind(batch.records[0].pub_seq)
                    .bind(batch.received_at)
                    .bind(restore_id)
                    .bind(&batch.edge_node_id)
                    .bind(&batch.ledger_epoch)
                    .execute(&mut *transaction)
                    .await?;
                    sqlx::query(
                        "UPDATE edge_node_activations SET state='recovery_hold', \
                         revision=revision+1, updated_at=? WHERE edge_node_id=? \
                         AND ledger_epoch=? AND state='active'",
                    )
                    .bind(batch.received_at)
                    .bind(&batch.edge_node_id)
                    .bind(&batch.ledger_epoch)
                    .execute(&mut *transaction)
                    .await?;
                    transaction.commit().await?;
                    return Err(StorageError::ArchiveRecoveryRequired);
                }
                let accepted_through = accept_sqlite(&mut transaction, &batch).await?;
                if let Some(restore_id) = pending {
                    sqlx::query(
                        "UPDATE edge_restore_cursor_checks SET state='matched', \
                         observed_cursor_start=?, updated_at=? WHERE restore_id=? \
                         AND edge_node_id=? AND ledger_epoch=? AND state='pending'",
                    )
                    .bind(batch.records[0].pub_seq)
                    .bind(batch.received_at)
                    .bind(restore_id)
                    .bind(&batch.edge_node_id)
                    .bind(&batch.ledger_epoch)
                    .execute(&mut *transaction)
                    .await?;
                }
                transaction.commit().await?;
                Ok(AcceptResult { accepted_through })
            }
            StorageInner::Postgres { pool, .. } => {
                let mut transaction = pool.begin().await?;
                let active: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM edge_node_activations \
                     WHERE edge_node_id = $1 AND ledger_epoch = $2 FOR UPDATE",
                )
                .bind(&batch.edge_node_id)
                .bind(&batch.ledger_epoch)
                .fetch_optional(&mut *transaction)
                .await?;
                if active.as_deref() != Some("active") {
                    return Err(StorageError::EdgeNodeNotActive);
                }
                let pending: Option<String> = sqlx::query_scalar(
                    "SELECT checks.restore_id FROM edge_restore_cursor_checks AS checks \
                     JOIN edge_restore_events AS events ON events.restore_id=checks.restore_id \
                     WHERE checks.edge_node_id=$1 AND checks.ledger_epoch=$2 \
                       AND checks.state='pending' \
                     ORDER BY events.restored_at DESC, checks.restore_id DESC LIMIT 1",
                )
                .bind(&batch.edge_node_id)
                .bind(&batch.ledger_epoch)
                .fetch_optional(&mut *transaction)
                .await?;
                let cursor: i64 = sqlx::query_scalar(
                    "SELECT accepted_through FROM accepted_cursors \
                     WHERE edge_node_id=$1 AND ledger_epoch=$2",
                )
                .bind(&batch.edge_node_id)
                .bind(&batch.ledger_epoch)
                .fetch_optional(&mut *transaction)
                .await?
                .unwrap_or(0);
                if let Some(restore_id) = pending.as_deref()
                    && batch.records[0].pub_seq > cursor + 1
                {
                    sqlx::query(
                        "UPDATE edge_restore_cursor_checks SET state='recovery_required', \
                         observed_cursor_start=$1, updated_at=$2 WHERE restore_id=$3 \
                         AND edge_node_id=$4 AND ledger_epoch=$5 AND state='pending'",
                    )
                    .bind(batch.records[0].pub_seq)
                    .bind(batch.received_at)
                    .bind(restore_id)
                    .bind(&batch.edge_node_id)
                    .bind(&batch.ledger_epoch)
                    .execute(&mut *transaction)
                    .await?;
                    sqlx::query(
                        "UPDATE edge_node_activations SET state='recovery_hold', \
                         revision=revision+1, updated_at=$1 WHERE edge_node_id=$2 \
                         AND ledger_epoch=$3 AND state='active'",
                    )
                    .bind(batch.received_at)
                    .bind(&batch.edge_node_id)
                    .bind(&batch.ledger_epoch)
                    .execute(&mut *transaction)
                    .await?;
                    transaction.commit().await?;
                    return Err(StorageError::ArchiveRecoveryRequired);
                }
                let accepted_through = accept_postgres(&mut transaction, &batch).await?;
                if let Some(restore_id) = pending {
                    sqlx::query(
                        "UPDATE edge_restore_cursor_checks SET state='matched', \
                         observed_cursor_start=$1, updated_at=$2 WHERE restore_id=$3 \
                         AND edge_node_id=$4 AND ledger_epoch=$5 AND state='pending'",
                    )
                    .bind(batch.records[0].pub_seq)
                    .bind(batch.received_at)
                    .bind(restore_id)
                    .bind(&batch.edge_node_id)
                    .bind(&batch.ledger_epoch)
                    .execute(&mut *transaction)
                    .await?;
                }
                transaction.commit().await?;
                Ok(AcceptResult { accepted_through })
            }
        }
    }

    pub async fn accepted_through(
        &self,
        edge_node_id: &str,
        ledger_epoch: &str,
    ) -> Result<i64, StorageError> {
        Ok(self
            .accepted_cursor(edge_node_id, ledger_epoch)
            .await?
            .accepted_through)
    }

    pub async fn accepted_cursor(
        &self,
        edge_node_id: &str,
        ledger_epoch: &str,
    ) -> Result<AcceptedCursor, StorageError> {
        let value: Option<(i64, i64)> = match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                sqlx::query_as(
                    "SELECT accepted_through, updated_at FROM accepted_cursors \
                     WHERE edge_node_id = ? AND ledger_epoch = ?",
                )
                .bind(edge_node_id)
                .bind(ledger_epoch)
                .fetch_optional(pool)
                .await?
            }
            StorageInner::Postgres { pool, .. } => {
                sqlx::query_as(
                    "SELECT accepted_through, updated_at FROM accepted_cursors \
                     WHERE edge_node_id = $1 AND ledger_epoch = $2",
                )
                .bind(edge_node_id)
                .bind(ledger_epoch)
                .fetch_optional(pool)
                .await?
            }
        };
        Ok(value.map_or(
            AcceptedCursor {
                accepted_through: 0,
                updated_at: 0,
            },
            |(accepted_through, updated_at)| AcceptedCursor {
                accepted_through,
                updated_at,
            },
        ))
    }

    pub async fn raw_records(
        &self,
        edge_node_id: &str,
        ledger_epoch: &str,
    ) -> Result<Vec<StoredRawRecord>, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let rows = sqlx::query(
                    "SELECT pub_seq, publication_id, record_json, received_at \
                     FROM raw_records WHERE edge_node_id = ? AND ledger_epoch = ? \
                     ORDER BY pub_seq",
                )
                .bind(edge_node_id)
                .bind(ledger_epoch)
                .fetch_all(pool)
                .await?;
                decode_sqlite_rows(rows)
            }
            StorageInner::Postgres { pool, .. } => {
                let rows = sqlx::query(
                    "SELECT pub_seq, publication_id, record_json, received_at \
                     FROM raw_records WHERE edge_node_id = $1 AND ledger_epoch = $2 \
                     ORDER BY pub_seq",
                )
                .bind(edge_node_id)
                .bind(ledger_epoch)
                .fetch_all(pool)
                .await?;
                decode_postgres_rows(rows)
            }
        }
    }
}

fn validate_batch(batch: &AcceptBatch) -> Result<(), StorageError> {
    if batch.edge_node_id.is_empty() || batch.ledger_epoch.is_empty() {
        return Err(StorageError::InvalidRecord(
            "edge_node_id and ledger_epoch must not be empty".into(),
        ));
    }
    if batch.records.is_empty() {
        return Err(StorageError::InvalidRecord(
            "batch must contain at least one record".into(),
        ));
    }
    if batch.publication_id.is_empty() || batch.received_at < 0 {
        return Err(StorageError::InvalidRecord(
            "publication_id must not be empty and received_at must not be negative".into(),
        ));
    }
    for window in batch.records.windows(2) {
        if window[1].pub_seq != window[0].pub_seq + 1 {
            return Err(StorageError::SequenceGap {
                expected: window[0].pub_seq + 1,
                actual: window[1].pub_seq,
            });
        }
    }
    Ok(())
}

async fn accept_sqlite(
    transaction: &mut Transaction<'_, Sqlite>,
    batch: &AcceptBatch,
) -> Result<i64, StorageError> {
    let mut cursor: i64 = sqlx::query_scalar(
        "SELECT accepted_through FROM accepted_cursors \
         WHERE edge_node_id = ? AND ledger_epoch = ?",
    )
    .bind(&batch.edge_node_id)
    .bind(&batch.ledger_epoch)
    .fetch_optional(&mut **transaction)
    .await?
    .unwrap_or(0);
    let initial_cursor = cursor;

    for record in &batch.records {
        let encoded = record.encoded();
        let hash = Sha256::digest(&encoded).to_vec();
        if record.pub_seq <= cursor {
            let accepted: Option<Vec<u8>> = sqlx::query_scalar(
                "SELECT record_sha256 FROM raw_records \
                 WHERE edge_node_id = ? AND ledger_epoch = ? AND pub_seq = ?",
            )
            .bind(&batch.edge_node_id)
            .bind(&batch.ledger_epoch)
            .bind(record.pub_seq)
            .fetch_optional(&mut **transaction)
            .await?;
            if accepted.as_deref() != Some(hash.as_slice()) {
                return Err(StorageError::RecordConflict {
                    sequence: record.pub_seq,
                });
            }
            continue;
        }
        if record.pub_seq != cursor + 1 {
            return Err(StorageError::SequenceGap {
                expected: cursor + 1,
                actual: record.pub_seq,
            });
        }
        sqlx::query(
            "INSERT INTO raw_records(\
             edge_node_id, ledger_epoch, pub_seq, publication_id, record_json, \
             record_sha256, received_at) VALUES(?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&batch.edge_node_id)
        .bind(&batch.ledger_epoch)
        .bind(record.pub_seq)
        .bind(&batch.publication_id)
        .bind(encoded)
        .bind(hash)
        .bind(batch.received_at)
        .execute(&mut **transaction)
        .await?;
        cursor = record.pub_seq;
    }
    if cursor > initial_cursor {
        sqlx::query(
            "INSERT INTO accepted_cursors(edge_node_id, ledger_epoch, accepted_through, updated_at) \
             VALUES(?, ?, ?, ?) ON CONFLICT(edge_node_id, ledger_epoch) DO UPDATE SET \
             accepted_through = excluded.accepted_through, updated_at = excluded.updated_at",
        )
        .bind(&batch.edge_node_id)
        .bind(&batch.ledger_epoch)
        .bind(cursor)
        .bind(batch.received_at)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(cursor)
}

async fn accept_postgres(
    transaction: &mut Transaction<'_, Postgres>,
    batch: &AcceptBatch,
) -> Result<i64, StorageError> {
    sqlx::query(
        "INSERT INTO accepted_cursors(edge_node_id, ledger_epoch, accepted_through, updated_at) \
         VALUES($1, $2, 0, 0) ON CONFLICT(edge_node_id, ledger_epoch) DO NOTHING",
    )
    .bind(&batch.edge_node_id)
    .bind(&batch.ledger_epoch)
    .execute(&mut **transaction)
    .await?;
    let mut cursor: i64 = sqlx::query_scalar(
        "SELECT accepted_through FROM accepted_cursors \
         WHERE edge_node_id = $1 AND ledger_epoch = $2 FOR UPDATE",
    )
    .bind(&batch.edge_node_id)
    .bind(&batch.ledger_epoch)
    .fetch_optional(&mut **transaction)
    .await?
    .unwrap_or(0);
    let initial_cursor = cursor;

    for record in &batch.records {
        let encoded = record.encoded();
        let hash = Sha256::digest(&encoded).to_vec();
        if record.pub_seq <= cursor {
            let accepted: Option<Vec<u8>> = sqlx::query_scalar(
                "SELECT record_sha256 FROM raw_records \
                 WHERE edge_node_id = $1 AND ledger_epoch = $2 AND pub_seq = $3",
            )
            .bind(&batch.edge_node_id)
            .bind(&batch.ledger_epoch)
            .bind(record.pub_seq)
            .fetch_optional(&mut **transaction)
            .await?;
            if accepted.as_deref() != Some(hash.as_slice()) {
                return Err(StorageError::RecordConflict {
                    sequence: record.pub_seq,
                });
            }
            continue;
        }
        if record.pub_seq != cursor + 1 {
            return Err(StorageError::SequenceGap {
                expected: cursor + 1,
                actual: record.pub_seq,
            });
        }
        sqlx::query(
            "INSERT INTO raw_records(\
             edge_node_id, ledger_epoch, pub_seq, publication_id, record_json, \
             record_sha256, received_at) VALUES($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&batch.edge_node_id)
        .bind(&batch.ledger_epoch)
        .bind(record.pub_seq)
        .bind(&batch.publication_id)
        .bind(encoded)
        .bind(hash)
        .bind(batch.received_at)
        .execute(&mut **transaction)
        .await?;
        cursor = record.pub_seq;
    }
    if cursor > initial_cursor {
        sqlx::query(
            "UPDATE accepted_cursors SET accepted_through = $3, updated_at = $4 \
             WHERE edge_node_id = $1 AND ledger_epoch = $2",
        )
        .bind(&batch.edge_node_id)
        .bind(&batch.ledger_epoch)
        .bind(cursor)
        .bind(batch.received_at)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(cursor)
}

fn decode_sqlite_rows(rows: Vec<SqliteRow>) -> Result<Vec<StoredRawRecord>, StorageError> {
    rows.into_iter()
        .map(|row| {
            let encoded: Vec<u8> = row.try_get("record_json")?;
            Ok(StoredRawRecord {
                pub_seq: row.try_get("pub_seq")?,
                publication_id: row.try_get("publication_id")?,
                record_json: encoded,
                received_at: row.try_get("received_at")?,
            })
        })
        .collect()
}

fn decode_postgres_rows(rows: Vec<PgRow>) -> Result<Vec<StoredRawRecord>, StorageError> {
    rows.into_iter()
        .map(|row| {
            let encoded: Vec<u8> = row.try_get("record_json")?;
            Ok(StoredRawRecord {
                pub_seq: row.try_get("pub_seq")?,
                publication_id: row.try_get("publication_id")?,
                record_json: encoded,
                received_at: row.try_get("received_at")?,
            })
        })
        .collect()
}

async fn validate_postgres_durability(pool: &PgPool) -> Result<(), StorageError> {
    for (setting, expected) in [
        ("fsync", "on"),
        ("synchronous_commit", "on"),
        ("full_page_writes", "on"),
    ] {
        let value: String = sqlx::query_scalar(&format!("SHOW {setting}"))
            .fetch_one(pool)
            .await?;
        if value != expected {
            return Err(StorageError::InvalidRecord(format!(
                "PostgreSQL {setting} must be {expected}"
            )));
        }
    }
    Ok(())
}

async fn reject_incomplete_postgres_restore(pool: &PgPool) -> Result<(), StorageError> {
    let state: Option<String> =
        sqlx::query_scalar("SELECT current_setting('iotkit.restore_state', true)")
            .fetch_one(pool)
            .await?;
    if state.as_deref() == Some("incomplete") {
        return Err(StorageError::RestoreIncomplete);
    }
    Ok(())
}

async fn reject_legacy_sqlite_schema(pool: &SqlitePool) -> Result<(), StorageError> {
    let raw_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'raw_records'",
    )
    .fetch_one(pool)
    .await?;
    let rust_migrations_exist: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await?;
    if raw_exists > 0 && rust_migrations_exist == 0 {
        return Err(StorageError::UnsupportedLegacySchema);
    }
    Ok(())
}

async fn reject_legacy_postgres_schema(pool: &PgPool) -> Result<(), StorageError> {
    let raw_exists: bool =
        sqlx::query_scalar("SELECT to_regclass('public.raw_records') IS NOT NULL")
            .fetch_one(pool)
            .await?;
    let rust_migrations_exist: bool =
        sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations') IS NOT NULL")
            .fetch_one(pool)
            .await?;
    if raw_exists && !rust_migrations_exist {
        return Err(StorageError::UnsupportedLegacySchema);
    }
    Ok(())
}

fn acquire_sqlite_guard(path: &Path) -> Result<File, StorageError> {
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".lock");
    let guard = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(PathBuf::from(lock_path))
        .map_err(StorageError::Guard)?;
    guard
        .try_lock_exclusive()
        .map_err(|_| StorageError::AlreadyInUse)?;
    Ok(guard)
}

async fn acquire_postgres_guard(pool: &PgPool) -> Result<PoolConnection<Postgres>, StorageError> {
    let mut connection = pool.acquire().await?;
    let acquired: bool = sqlx::query_scalar(
        "SELECT pg_try_advisory_lock(\
         hashtextextended('iotkit-edge-storage:' || current_database(), 0))",
    )
    .fetch_one(&mut *connection)
    .await?;
    if !acquired {
        return Err(StorageError::AlreadyInUse);
    }
    Ok(connection)
}

fn compact_json(payload: &[u8]) -> Result<Vec<u8>, StorageError> {
    let _: &serde_json::value::RawValue =
        serde_json::from_slice(payload).map_err(StorageError::EncodeRecord)?;
    let mut compact = Vec::with_capacity(payload.len());
    let mut in_string = false;
    let mut escaped = false;
    for byte in payload {
        if in_string {
            compact.push(*byte);
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
        } else if *byte == b'"' {
            in_string = true;
            compact.push(*byte);
        } else if !byte.is_ascii_whitespace() {
            compact.push(*byte);
        }
    }
    Ok(compact)
}
