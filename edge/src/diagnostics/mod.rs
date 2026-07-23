//! Operator-facing storage capacity and cause-oriented diagnostics.

use std::{fs, path::Path};

use serde::Serialize;
use sqlx::Row;

use crate::storage::{OperationBackend, Storage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageState {
    Healthy,
    Warning,
    Critical,
    Unavailable,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageStatus {
    pub profile: String,
    pub state: StorageState,
    pub filesystem_available: bool,
    pub database_bytes: i64,
    pub reclaimable_bytes: i64,
    pub disk_total_bytes: u64,
    pub disk_available_bytes: u64,
    pub disk_used_percent: i32,
    pub warning_percent: i32,
    pub raw_record_count: i64,
    pub semantic_observation_count: i64,
    pub pending_output_count: i64,
    pub projection_failure_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_backup_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_backup_at: Option<i64>,
    pub absolute_reserve_state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticState {
    Healthy,
    Attention,
    Critical,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticIssue {
    pub code: String,
    pub severity: String,
    pub component: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_ref: Option<String>,
    pub summary: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticReport {
    pub generated_at: i64,
    pub state: DiagnosticState,
    pub issues: Vec<DiagnosticIssue>,
    pub truncated: bool,
    pub limitations: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DiagnosticError {
    #[error("storage warning percent must be between 50 and 99")]
    InvalidWarningPercent,
    #[error("diagnostic database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("diagnostic filesystem error: {0}")]
    Filesystem(#[from] std::io::Error),
}

pub async fn storage_status(
    storage: &Storage,
    warning_percent: i32,
) -> Result<StorageStatus, DiagnosticError> {
    if !(50..=99).contains(&warning_percent) {
        return Err(DiagnosticError::InvalidWarningPercent);
    }
    match storage.operation_backend() {
        OperationBackend::Sqlite { pool, path } => {
            let page_count: i64 = sqlx::query_scalar("PRAGMA page_count")
                .fetch_one(pool)
                .await?;
            let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
                .fetch_one(pool)
                .await?;
            let free_pages: i64 = sqlx::query_scalar("PRAGMA freelist_count")
                .fetch_one(pool)
                .await?;
            let mut database_bytes = page_count * page_size;
            for suffix in ["-wal", "-shm"] {
                if let Ok(metadata) = fs::metadata(format!("{}{suffix}", path.display())) {
                    database_bytes += metadata.len() as i64;
                }
            }
            let total = fs2::total_space(parent(path))?;
            let available = fs2::available_space(parent(path))?;
            let used_percent = ((total.saturating_sub(available) as u128) * 100)
                .checked_div(total as u128)
                .unwrap_or(0) as i32;
            let state = if used_percent >= 97 || available < 512 * 1024 * 1024 {
                StorageState::Critical
            } else if used_percent >= warning_percent || available < 2 * 1024 * 1024 * 1024 {
                StorageState::Warning
            } else {
                StorageState::Healthy
            };
            let (raw, semantic, pending, failures, backup_id, backup_at) =
                sqlite_counts(pool).await?;
            Ok(StorageStatus {
                profile: "embedded".into(),
                state,
                filesystem_available: true,
                database_bytes,
                reclaimable_bytes: free_pages * page_size,
                disk_total_bytes: total,
                disk_available_bytes: available,
                disk_used_percent: used_percent,
                warning_percent,
                raw_record_count: raw,
                semantic_observation_count: semantic,
                pending_output_count: pending,
                projection_failure_count: failures,
                last_backup_id: backup_id,
                last_backup_at: backup_at,
                absolute_reserve_state: if available < 512 * 1024 * 1024 {
                    "critical"
                } else if available < 2 * 1024 * 1024 * 1024 {
                    "warning"
                } else {
                    "adequate"
                }
                .into(),
            })
        }
        OperationBackend::Postgres { pool, .. } => {
            let database_bytes: i64 =
                sqlx::query_scalar("SELECT pg_database_size(current_database())")
                    .fetch_one(pool)
                    .await?;
            let (raw, semantic, pending, failures, backup_id, backup_at) =
                postgres_counts(pool).await?;
            Ok(StorageStatus {
                profile: "postgres".into(),
                state: StorageState::Unavailable,
                filesystem_available: false,
                database_bytes,
                reclaimable_bytes: 0,
                disk_total_bytes: 0,
                disk_available_bytes: 0,
                disk_used_percent: 0,
                warning_percent,
                raw_record_count: raw,
                semantic_observation_count: semantic,
                pending_output_count: pending,
                projection_failure_count: failures,
                last_backup_id: backup_id,
                last_backup_at: backup_at,
                absolute_reserve_state: "unknown".into(),
            })
        }
    }
}

pub async fn diagnostics(
    storage: &Storage,
    warning_percent: i32,
    now: i64,
) -> Result<DiagnosticReport, DiagnosticError> {
    let capacity = storage_status(storage, warning_percent).await?;
    let mut report = DiagnosticReport {
        generated_at: now,
        state: DiagnosticState::Healthy,
        issues: Vec::new(),
        truncated: false,
        limitations: vec![
            "Input Adapter process health is not reported independently.".into(),
            "PostgreSQL filesystem free space must be monitored on the host.".into(),
        ],
    };
    match capacity.state {
        StorageState::Critical => push_issue(
            &mut report,
            "edge_storage_critical",
            "critical",
            "edge_storage",
            "IoTKit Edge storage capacity is critically low",
        ),
        StorageState::Warning => push_issue(
            &mut report,
            "edge_storage_warning",
            "warning",
            "edge_storage",
            "IoTKit Edge storage capacity is low",
        ),
        StorageState::Unavailable => push_issue(
            &mut report,
            "edge_storage_unavailable",
            "warning",
            "edge_storage",
            "Host filesystem capacity is unavailable through SQL",
        ),
        StorageState::Healthy => {}
    }
    if capacity.last_backup_at.is_none() {
        push_issue(
            &mut report,
            "edge_backup_missing",
            "warning",
            "edge_backup",
            "No verified encrypted backup has been recorded",
        );
    }
    if capacity.projection_failure_count > 0 {
        push_issue(
            &mut report,
            "semantic_projection_failed",
            "warning",
            "semantic_projection",
            "Semantic projection failures require attention",
        );
    }
    Ok(report)
}

fn push_issue(
    report: &mut DiagnosticReport,
    code: &str,
    severity: &str,
    component: &str,
    summary: &str,
) {
    if report.issues.len() >= 500 {
        report.truncated = true;
        return;
    }
    report.issues.push(DiagnosticIssue {
        code: code.into(),
        severity: severity.into(),
        component: component.into(),
        resource_ref: None,
        summary: summary.into(),
        detail: "Preserve accepted and pending data while the operator investigates.".into(),
        observed_at: None,
    });
    if severity == "critical" {
        report.state = DiagnosticState::Critical;
    } else if report.state == DiagnosticState::Healthy {
        report.state = DiagnosticState::Attention;
    }
}

async fn sqlite_counts(
    pool: &sqlx::SqlitePool,
) -> Result<(i64, i64, i64, i64, Option<String>, Option<i64>), sqlx::Error> {
    let raw = sqlx::query_scalar("SELECT count(*) FROM raw_records")
        .fetch_one(pool)
        .await?;
    let semantic = sqlite_optional_count(pool, "semantic_observations_v3", None).await?;
    let pending =
        sqlite_optional_count(pool, "output_outbox_v3", Some("published_at IS NULL")).await?;
    let failures = sqlite_optional_count(pool, "semantic_projection_failures_v3", None).await?;
    let backup = sqlx::query(
        "SELECT backup_id, created_at FROM edge_backup_events \
         ORDER BY created_at DESC, backup_id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok((
        raw,
        semantic,
        pending,
        failures,
        backup.as_ref().map(|row| row.get::<String, _>("backup_id")),
        backup.as_ref().map(|row| row.get::<i64, _>("created_at")),
    ))
}

async fn postgres_counts(
    pool: &sqlx::PgPool,
) -> Result<(i64, i64, i64, i64, Option<String>, Option<i64>), sqlx::Error> {
    let raw = sqlx::query_scalar("SELECT count(*) FROM raw_records")
        .fetch_one(pool)
        .await?;
    let semantic = postgres_optional_count(pool, "semantic_observations_v3", None).await?;
    let pending =
        postgres_optional_count(pool, "output_outbox_v3", Some("published_at IS NULL")).await?;
    let failures = postgres_optional_count(pool, "semantic_projection_failures_v3", None).await?;
    let backup = sqlx::query(
        "SELECT backup_id, created_at FROM edge_backup_events \
         ORDER BY created_at DESC, backup_id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok((
        raw,
        semantic,
        pending,
        failures,
        backup.as_ref().map(|row| row.get::<String, _>("backup_id")),
        backup.as_ref().map(|row| row.get::<i64, _>("created_at")),
    ))
}

async fn sqlite_optional_count(
    pool: &sqlx::SqlitePool,
    table: &str,
    predicate: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let exists: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?")
            .bind(table)
            .fetch_one(pool)
            .await?;
    if exists == 0 {
        return Ok(0);
    }
    sqlx::query_scalar(&format!(
        "SELECT count(*) FROM {table}{}",
        predicate.map_or(String::new(), |value| format!(" WHERE {value}"))
    ))
    .fetch_one(pool)
    .await
}

async fn postgres_optional_count(
    pool: &sqlx::PgPool,
    table: &str,
    predicate: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(format!("public.{table}"))
        .fetch_one(pool)
        .await?;
    if !exists {
        return Ok(0);
    }
    sqlx::query_scalar(&format!(
        "SELECT count(*) FROM {table}{}",
        predicate.map_or(String::new(), |value| format!(" WHERE {value}"))
    ))
    .fetch_one(pool)
    .await
}

fn parent(path: &Path) -> &Path {
    path.parent().unwrap_or_else(|| Path::new("."))
}
