//! Operator-facing storage capacity and cause-oriented diagnostics.

use std::{fs, path::Path};

use crate::{
    mqtt::ingest::{IngestConnectionState, IngestRuntimeHealth},
    storage::{OperationBackend, Storage},
};
use serde::Serialize;
use sqlx::Row;
use x509_parser::{pem::parse_x509_pem, prelude::FromDer};

const HEARTBEAT_WARNING_MS: i64 = 90_000;
const HEARTBEAT_CRITICAL_MS: i64 = 300_000;
const STALE_WORK_MS: i64 = 5 * 60 * 1_000;
const MAX_DIAGNOSTIC_SCOPE: usize = 64;

/// Per-active-rule latest observation lookup. The outer active-config scan is
/// small configuration state; each retained-history lookup is index-bound.
#[doc(hidden)]
pub const SQLITE_DIAGNOSTIC_PROJECTION_LATEST_SQL: &str = "SELECT (SELECT observation.created_at \
    FROM semantic_observations AS observation WHERE observation.rule_id=rule.rule_id \
    ORDER BY observation.created_at DESC LIMIT 1) AS created_at \
    FROM semantic_rules AS rule WHERE rule.active=1";
#[doc(hidden)]
pub const POSTGRES_DIAGNOSTIC_PROJECTION_LATEST_SQL: &str = "SELECT (SELECT observation.created_at \
    FROM semantic_observations AS observation WHERE observation.rule_id=rule.rule_id \
    ORDER BY observation.created_at DESC LIMIT 1) AS created_at \
    FROM semantic_rules AS rule WHERE rule.active=TRUE";

/// Per-active-route delivery and pending work lookups avoid an aggregate over
/// every historical route after a route is retired.
#[doc(hidden)]
pub const SQLITE_DIAGNOSTIC_OUTPUT_DELIVERY_LATEST_SQL: &str = "SELECT (SELECT outbox.published_at \
    FROM output_outbox AS outbox WHERE outbox.route_id=route.route_id \
    AND outbox.published_at IS NOT NULL ORDER BY outbox.published_at DESC LIMIT 1) AS published_at \
    FROM output_routes AS route WHERE route.active=1";
#[doc(hidden)]
pub const POSTGRES_DIAGNOSTIC_OUTPUT_DELIVERY_LATEST_SQL: &str = "SELECT (SELECT outbox.published_at \
    FROM output_outbox AS outbox WHERE outbox.route_id=route.route_id \
    AND outbox.published_at IS NOT NULL ORDER BY outbox.published_at DESC LIMIT 1) AS published_at \
    FROM output_routes AS route WHERE route.active=TRUE";
#[doc(hidden)]
pub const SQLITE_DIAGNOSTIC_OUTPUT_PENDING_OLDEST_SQL: &str = "SELECT (SELECT outbox.created_at \
    FROM output_outbox AS outbox WHERE outbox.route_id=route.route_id \
    AND outbox.published_at IS NULL ORDER BY outbox.created_at ASC LIMIT 1) AS created_at \
    FROM output_routes AS route WHERE route.active=1";
#[doc(hidden)]
pub const POSTGRES_DIAGNOSTIC_OUTPUT_PENDING_OLDEST_SQL: &str = "SELECT (SELECT outbox.created_at \
    FROM output_outbox AS outbox WHERE outbox.route_id=route.route_id \
    AND outbox.published_at IS NULL ORDER BY outbox.created_at ASC LIMIT 1) AS created_at \
    FROM output_routes AS route WHERE route.active=TRUE";

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
    /// Durable rule-record work awaiting semantic projection, not raw-record or receipt lag.
    pub pending_semantic_projection_count: i64,
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

/// Ordered causal layers shown by the Console. The enum is part of the JSON
/// contract so callers cannot silently invent a parallel status taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticStageKind {
    Sensor,
    Adapter,
    Node,
    Broker,
    RawCustody,
    Projection,
    ExternalOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticStageState {
    Ok,
    Warning,
    Critical,
    Unknown,
    NotApplicable,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticStage {
    pub stage: DiagnosticStageKind,
    pub state: DiagnosticStageState,
    /// Stable machine-readable classification. It never contains an error
    /// string, topic, endpoint, credential, or customer payload.
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<i64>,
    pub affected_count: usize,
    pub scope: String,
    pub cause: String,
    pub action: String,
    pub href: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<DiagnosticStageKind>,
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
    pub stages: Vec<DiagnosticStage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broker_certificate: Option<CertificateStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CertificateStatus {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_after: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_remaining: Option<i64>,
    pub needs_action: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DiagnosticError {
    #[error("storage warning percent must be between 50 and 99")]
    InvalidWarningPercent,
    #[error("diagnostic database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("diagnostic filesystem error: {0}")]
    Filesystem(#[from] std::io::Error),
    #[error("diagnostic storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
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
            let (raw, semantic, projection_pending, pending, failures, backup_id, backup_at) =
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
                pending_semantic_projection_count: projection_pending,
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
            let (raw, semantic, projection_pending, pending, failures, backup_id, backup_at) =
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
                pending_semantic_projection_count: projection_pending,
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
    diagnostics_with_runtime(
        storage,
        warning_percent,
        now,
        None,
        IngestRuntimeHealth {
            state: IngestConnectionState::Unknown,
            last_ready_at: None,
        },
    )
    .await
}

pub async fn diagnostics_with_certificate(
    storage: &Storage,
    warning_percent: i32,
    now: i64,
    certificate_file: Option<&Path>,
) -> Result<DiagnosticReport, DiagnosticError> {
    diagnostics_with_runtime(
        storage,
        warning_percent,
        now,
        certificate_file,
        IngestRuntimeHealth {
            state: IngestConnectionState::Unknown,
            last_ready_at: None,
        },
    )
    .await
}

pub async fn diagnostics_with_runtime(
    storage: &Storage,
    warning_percent: i32,
    now: i64,
    certificate_file: Option<&Path>,
    ingest_health: IngestRuntimeHealth,
) -> Result<DiagnosticReport, DiagnosticError> {
    let capacity = storage_status(storage, warning_percent).await?;
    let mut report = DiagnosticReport {
        generated_at: now,
        state: DiagnosticState::Healthy,
        issues: Vec::new(),
        truncated: false,
        limitations: vec![
            "Input Adapter health is bounded by the latest Edge Node heartbeat.".into(),
            "PostgreSQL filesystem free space must be monitored on the host.".into(),
        ],
        stages: Vec::new(),
        broker_certificate: None,
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
    match storage.operation_backend() {
        OperationBackend::Sqlite { pool, .. } => {
            for row in sqlx::query(
                "SELECT edge_node_ref FROM edge_node_activations \
                 WHERE state='recovery_hold' ORDER BY edge_node_ref",
            )
            .fetch_all(pool)
            .await?
            {
                push_resource_issue(
                    &mut report,
                    "edge_node_recovery_hold",
                    "critical",
                    "edge_node",
                    row.get("edge_node_ref"),
                    "An Edge Node is held for recovery review",
                    None,
                );
            }
            for row in sqlx::query(
                "SELECT edge_node_id,updated_at FROM edge_restore_cursor_checks \
                 WHERE state='recovery_required' ORDER BY updated_at,edge_node_id",
            )
            .fetch_all(pool)
            .await?
            {
                push_resource_issue(
                    &mut report,
                    "archive_recovery_required",
                    "critical",
                    "edge_restore",
                    row.get("edge_node_id"),
                    "Restored archive data may be missing",
                    Some(row.get("updated_at")),
                );
            }
            if sqlite_table_exists(pool, "output_outbox").await? {
                let oldest: Option<i64> = sqlx::query_scalar(
                    "SELECT MIN(created_at) FROM output_outbox WHERE published_at IS NULL",
                )
                .fetch_one(pool)
                .await?;
                if oldest.is_some_and(|value| now.saturating_sub(value) > 5 * 60 * 1000) {
                    push_resource_issue(
                        &mut report,
                        "output_delivery_stale",
                        "warning",
                        "output",
                        "pending-output".into(),
                        "External output delivery is stale",
                        oldest,
                    );
                }
            }
        }
        OperationBackend::Postgres { pool, .. } => {
            for row in sqlx::query(
                "SELECT edge_node_ref FROM edge_node_activations \
                 WHERE state='recovery_hold' ORDER BY edge_node_ref",
            )
            .fetch_all(pool)
            .await?
            {
                push_resource_issue(
                    &mut report,
                    "edge_node_recovery_hold",
                    "critical",
                    "edge_node",
                    row.get("edge_node_ref"),
                    "An Edge Node is held for recovery review",
                    None,
                );
            }
            for row in sqlx::query(
                "SELECT edge_node_id,updated_at FROM edge_restore_cursor_checks \
                 WHERE state='recovery_required' ORDER BY updated_at,edge_node_id",
            )
            .fetch_all(pool)
            .await?
            {
                push_resource_issue(
                    &mut report,
                    "archive_recovery_required",
                    "critical",
                    "edge_restore",
                    row.get("edge_node_id"),
                    "Restored archive data may be missing",
                    Some(row.get("updated_at")),
                );
            }
            if postgres_table_exists(pool, "output_outbox").await? {
                let oldest: Option<i64> = sqlx::query_scalar(
                    "SELECT MIN(created_at) FROM output_outbox WHERE published_at IS NULL",
                )
                .fetch_one(pool)
                .await?;
                if oldest.is_some_and(|value| now.saturating_sub(value) > 5 * 60 * 1000) {
                    push_resource_issue(
                        &mut report,
                        "output_delivery_stale",
                        "warning",
                        "output",
                        "pending-output".into(),
                        "External output delivery is stale",
                        oldest,
                    );
                }
            }
        }
    }
    if let Some(path) = certificate_file {
        match read_certificate_status(path, now) {
            Some(status) => {
                if status.needs_action {
                    push_resource_issue(
                        &mut report,
                        "broker_certificate_expiring",
                        "warning",
                        "broker_certificate",
                        path.display().to_string(),
                        "Broker certificate is expired or expires within 30 days",
                        status.not_after,
                    );
                }
                report.broker_certificate = Some(status);
            }
            None => {
                report.broker_certificate = Some(CertificateStatus {
                    available: false,
                    not_after: None,
                    days_remaining: None,
                    needs_action: true,
                });
                push_resource_issue(
                    &mut report,
                    "broker_certificate_unavailable",
                    "warning",
                    "broker_certificate",
                    path.display().to_string(),
                    "Broker certificate status is unavailable",
                    None,
                );
            }
        }
    }
    report.stages = causal_stages(storage, now, &ingest_health).await?;
    for state in report
        .stages
        .iter()
        .map(|stage| stage.state)
        .collect::<Vec<_>>()
    {
        update_report_state(&mut report, state);
    }
    Ok(report)
}

#[derive(Default)]
struct CausalFacts {
    active_rule_count: i64,
    oldest_projection_at: Option<i64>,
    projection_last_success_at: Option<i64>,
    unresolved_projection_failures: i64,
    active_output_route_count: i64,
    oldest_pending_output_at: Option<i64>,
    latest_output_delivery_at: Option<i64>,
    unresolved_output_transform_errors: i64,
}

/// Complete aggregate evidence for active nodes.  We deliberately aggregate
/// in SQL instead of fetching a prefix and calling it representative: a 65th
/// stale node must not turn a status page green.
#[derive(Default)]
struct NodeStatusFacts {
    active_nodes: i64,
    unknown_nodes: i64,
    stale_warning_nodes: i64,
    stale_critical_nodes: i64,
    stopped_collectors: i64,
    fresh_last_live_received_at: Option<i64>,
    stale_warning_last_live_received_at: Option<i64>,
    stale_critical_last_live_received_at: Option<i64>,
}

#[derive(Default)]
struct AdapterFacts {
    adapters: i64,
    restarting: i64,
    exhausted: i64,
    stopped: i64,
    last_live_received_at: Option<i64>,
}

#[derive(Default)]
struct RawCustodyFacts {
    /// Node claims custody beyond the Edge cursor.  This is an impossible
    /// direction for an application acceptance cursor and is therefore a
    /// conflict, not ordinary pending work.
    edge_cursor_behind: i64,
    /// Both sides report the same cursor but no accepted-cursor progress has
    /// happened for five minutes while a node still reports pending work.
    stalled_equal_cursor: i64,
    /// Edge has accepted beyond the last node report.  This is normal ACK
    /// convergence evidence; it is kept to explain the healthy state without
    /// turning it into a false incident.
    edge_cursor_ahead: i64,
    storage_pressure: i64,
    last_accepted_at: Option<i64>,
}

#[derive(Default)]
struct SensorFacts {
    inspected: usize,
    capped: bool,
    missing: usize,
    stale: usize,
    last_success_at: Option<i64>,
}

macro_rules! node_status_facts_from_row {
    ($row:expr) => {
        Ok(NodeStatusFacts {
            active_nodes: $row.try_get("active_nodes")?,
            unknown_nodes: $row.try_get("unknown_nodes")?,
            stale_warning_nodes: $row.try_get("stale_warning_nodes")?,
            stale_critical_nodes: $row.try_get("stale_critical_nodes")?,
            stopped_collectors: $row.try_get("stopped_collectors")?,
            fresh_last_live_received_at: $row.try_get("fresh_last_live_received_at")?,
            stale_warning_last_live_received_at: $row
                .try_get("stale_warning_last_live_received_at")?,
            stale_critical_last_live_received_at: $row
                .try_get("stale_critical_last_live_received_at")?,
        })
    };
}

macro_rules! adapter_facts_from_row {
    ($row:expr) => {
        Ok(AdapterFacts {
            adapters: $row.try_get("adapters")?,
            restarting: $row.try_get("restarting")?,
            exhausted: $row.try_get("exhausted")?,
            stopped: $row.try_get("stopped")?,
            last_live_received_at: $row.try_get("last_live_received_at")?,
        })
    };
}

macro_rules! raw_custody_facts_from_row {
    ($row:expr) => {
        Ok(RawCustodyFacts {
            edge_cursor_behind: $row.try_get("edge_cursor_behind")?,
            stalled_equal_cursor: $row.try_get("stalled_equal_cursor")?,
            edge_cursor_ahead: $row.try_get("edge_cursor_ahead")?,
            storage_pressure: $row.try_get("storage_pressure")?,
            last_accepted_at: $row.try_get("last_accepted_at")?,
        })
    };
}

/// Builds the causal list from bounded, already-owned facts. This deliberately
/// does not inspect process logs or arbitrary MQTT state: unknown remains
/// unknown rather than becoming an invented healthy result.
async fn causal_stages(
    storage: &Storage,
    now: i64,
    ingest_health: &IngestRuntimeHealth,
) -> Result<Vec<DiagnosticStage>, DiagnosticError> {
    let broker = broker_stage(ingest_health);
    // Broker is classified first even though it is displayed after Node.  A
    // disconnected ingest path makes heartbeat age unknowable rather than a
    // second, independent node outage.
    let node_facts = node_status_facts(storage, now).await?;
    let facts = causal_facts(storage).await?;
    let node = if is_healthy(broker.state) {
        node_stage(&node_facts)
    } else {
        blocked_stage(
            DiagnosticStageKind::Node,
            DiagnosticStageKind::Broker,
            "node_blocked_by_broker",
            "内部Broker経路を確認できないため、収集ノードの現在のハートビートを判定できません。",
            "内部Broker経路を確認する",
            "/system",
        )
    };
    let adapter = if is_healthy(node.state) {
        adapter_stage(&adapter_facts(storage, now).await?)
    } else {
        blocked_stage(
            DiagnosticStageKind::Adapter,
            DiagnosticStageKind::Node,
            "adapter_blocked_by_node",
            "収集ノードの最新状態を確認できないため、入力アダプターの状態を判定できません。",
            "収集ノードの状態を確認する",
            "/equipment",
        )
    };
    let raw_custody = if is_healthy(node.state) && is_healthy(broker.state) {
        raw_custody_stage(&raw_custody_facts(storage, now).await?)
    } else {
        let blocked_by = if !is_healthy(node.state) {
            DiagnosticStageKind::Node
        } else {
            DiagnosticStageKind::Broker
        };
        blocked_stage(
            DiagnosticStageKind::RawCustody,
            blocked_by,
            "raw_custody_blocked_by_upstream",
            "上流の状態を確認できないため、新しい受信データの保管状態を判定できません。",
            "上流の状態を確認する",
            "/status",
        )
    };
    let sensor = if is_healthy(adapter.state) && is_healthy(node.state) && is_healthy(broker.state)
    {
        sensor_stage(storage, now).await?
    } else {
        let blocked_by = if !is_healthy(adapter.state) {
            DiagnosticStageKind::Adapter
        } else if !is_healthy(node.state) {
            DiagnosticStageKind::Node
        } else {
            DiagnosticStageKind::Broker
        };
        blocked_stage(
            DiagnosticStageKind::Sensor,
            blocked_by,
            "sensor_blocked_by_upstream",
            "上流の状態を確認できないため、センサーから新しい値が届いているかを判定できません。",
            "上流の状態を確認する",
            "/equipment",
        )
    };
    // Durable queue/failure evidence has its own causal value. Do not hide an
    // already-persisted projection incident just because the next input is
    // absent or stale upstream.
    let projection = if facts.active_rule_count == 0
        || projection_needs_attention(&facts, now)
        || (is_healthy(raw_custody.state) && is_healthy(sensor.state))
    {
        projection_stage(&facts, now)
    } else {
        let blocked_by = if !is_healthy(raw_custody.state) {
            DiagnosticStageKind::RawCustody
        } else {
            DiagnosticStageKind::Sensor
        };
        blocked_stage(
            DiagnosticStageKind::Projection,
            blocked_by,
            "projection_blocked_by_no_new_input",
            "新しい入力を確認できないため、計測ルールの処理状態を判定できません。",
            "受信と保管の状態を確認する",
            "/logs",
        )
    };
    // Likewise an existing transform error or durable PUBACK wait is not
    // merely downstream speculation: it remains actionable while projection
    // is blocked by a separate upstream condition.
    let output = if facts.active_output_route_count == 0
        || output_needs_attention(&facts, now)
        || is_healthy(projection.state)
    {
        output_stage(&facts, now)
    } else {
        blocked_stage(
            DiagnosticStageKind::ExternalOutput,
            DiagnosticStageKind::Projection,
            "external_output_blocked_by_projection",
            "計測ルールの新しい結果を確認できないため、外部出力の新しい処理状態を判定できません。",
            "計測ルールの状態を確認する",
            "/logs",
        )
    };

    Ok(vec![
        sensor,
        adapter,
        node,
        broker,
        raw_custody,
        projection,
        output,
    ])
}

fn projection_needs_attention(facts: &CausalFacts, now: i64) -> bool {
    facts.unresolved_projection_failures > 0
        || facts
            .oldest_projection_at
            .is_some_and(|received_at| now.saturating_sub(received_at) > STALE_WORK_MS)
}

fn output_needs_attention(facts: &CausalFacts, now: i64) -> bool {
    facts.unresolved_output_transform_errors > 0
        || facts
            .oldest_pending_output_at
            .is_some_and(|created_at| now.saturating_sub(created_at) > STALE_WORK_MS)
}

fn node_stage(facts: &NodeStatusFacts) -> DiagnosticStage {
    if facts.active_nodes == 0 {
        return stage(
            DiagnosticStageKind::Node,
            DiagnosticStageState::Unknown,
            "node_status_unknown",
            None,
            0,
            "有効な収集ノード",
            "有効な収集ノードの最新ハートビートがありません。登録状態だけではオンラインを示しません。",
            "収集ノードの接続を確認する",
            "/equipment",
            None,
        );
    }
    let (state, code, last_success_at, affected, cause) = if facts.stopped_collectors > 0 {
        (
            DiagnosticStageState::Critical,
            "node_collector_stopped",
            None,
            facts.stopped_collectors,
            "収集ノードが収集処理の停止を明示的に報告しています。",
        )
    } else if facts.stale_critical_nodes > 0 {
        (
            DiagnosticStageState::Critical,
            "node_heartbeat_stale_critical",
            facts.stale_critical_last_live_received_at,
            facts.stale_critical_nodes,
            "ハートビートが300秒以上更新されていません。",
        )
    } else if facts.stale_warning_nodes > 0 {
        (
            DiagnosticStageState::Warning,
            "node_heartbeat_stale_warning",
            facts.stale_warning_last_live_received_at,
            facts.stale_warning_nodes,
            "ハートビートが90秒以上更新されていません。",
        )
    } else if facts.unknown_nodes > 0 {
        (
            DiagnosticStageState::Unknown,
            "node_retained_or_no_live_status",
            None,
            facts.unknown_nodes,
            "保持メッセージまたは未確認の状態だけでは、収集ノードが現在オンラインとは判断できません。",
        )
    } else {
        (
            DiagnosticStageState::Ok,
            "node_heartbeat_fresh",
            facts.fresh_last_live_received_at,
            0,
            "収集ノードから最新のハートビートを受信しています。",
        )
    };
    stage(
        DiagnosticStageKind::Node,
        state,
        code,
        last_success_at,
        bounded_count(affected),
        &scope_count(facts.active_nodes, "有効な収集ノード"),
        cause,
        "収集ノードの電源とネットワークを確認する",
        "/equipment",
        None,
    )
}

fn adapter_stage(facts: &AdapterFacts) -> DiagnosticStage {
    let (state, code, last_success_at, affected, cause) = if facts.adapters == 0 {
        (
            DiagnosticStageState::NotApplicable,
            "adapter_not_reported",
            None,
            0,
            "入力アダプターはハートビートに報告されていません。",
        )
    } else if facts.stopped > 0 {
        (
            DiagnosticStageState::Critical,
            "adapter_stopped",
            None,
            facts.stopped,
            "入力アダプターが停止を報告しています。",
        )
    } else if facts.exhausted > 0 {
        (
            DiagnosticStageState::Critical,
            "adapter_exhausted",
            None,
            facts.exhausted,
            "入力アダプターが再試行を使い切った状態です。",
        )
    } else if facts.restarting > 0 {
        (
            DiagnosticStageState::Warning,
            "adapter_restarting",
            None,
            facts.restarting,
            "入力アダプターが再起動中です。",
        )
    } else {
        (
            DiagnosticStageState::Ok,
            "adapter_running",
            facts.last_live_received_at,
            0,
            "報告された入力アダプターは実行中です。",
        )
    };
    stage(
        DiagnosticStageKind::Adapter,
        state,
        code,
        last_success_at,
        bounded_count(affected),
        &scope_count(facts.adapters, "入力アダプター"),
        cause,
        "入力アダプターを確認する",
        "/equipment",
        None,
    )
}

fn broker_stage(health: &IngestRuntimeHealth) -> DiagnosticStage {
    let (state, code, cause) = match health.state {
        IngestConnectionState::Ready => (
            DiagnosticStageState::Ok,
            "broker_ready",
            "IoTKit Edgeは必要なMQTT購読をすべて確認しています。",
        ),
        IngestConnectionState::Connecting => (
            DiagnosticStageState::Warning,
            "broker_connecting",
            "IoTKit EdgeはMQTT接続または購読確認を進めています。",
        ),
        IngestConnectionState::Disconnected => (
            DiagnosticStageState::Critical,
            "broker_disconnected",
            "IoTKit Edgeの内部Broker経路（ネットワーク、TLS、認証を含む）が利用できません。",
        ),
        IngestConnectionState::Unknown => (
            DiagnosticStageState::Unknown,
            "broker_state_unknown",
            "IoTKit Edge MQTT接続の結果をまだ確認していません。",
        ),
    };
    stage(
        DiagnosticStageKind::Broker,
        state,
        code,
        health.last_ready_at,
        usize::from(state != DiagnosticStageState::Ok),
        "IoTKit Edge MQTT ingest",
        cause,
        "IoTKit Edgeサービスを確認する",
        "/system",
        None,
    )
}

fn raw_custody_stage(facts: &RawCustodyFacts) -> DiagnosticStage {
    let (state, code, affected, cause) = if facts.edge_cursor_behind > 0 {
        (
            DiagnosticStageState::Critical,
            "raw_custody_cursor_conflict",
            facts.edge_cursor_behind,
            "収集ノードが報告する受理カーソルがIoTKit Edgeの耐久済みカーソルより進んでいます。",
        )
    } else if facts.storage_pressure > 0 {
        (
            DiagnosticStageState::Warning,
            "raw_custody_storage_pressure",
            facts.storage_pressure,
            "収集ノードがローカル保存領域の圧迫を報告しています。",
        )
    } else if facts.stalled_equal_cursor > 0 {
        (
            DiagnosticStageState::Warning,
            "raw_custody_pending_stalled",
            facts.stalled_equal_cursor,
            "保留公開があり、受理カーソルが5分以上進んでいません。",
        )
    } else if facts.edge_cursor_ahead > 0 {
        (
            DiagnosticStageState::Ok,
            "raw_custody_cursor_converging",
            0,
            "IoTKit Edgeの受理カーソルは収集ノードの直近報告より進んでおり、確認応答の収束を待っています。",
        )
    } else {
        (
            DiagnosticStageState::Ok,
            "raw_custody_current",
            0,
            "Edge Nodeの保留公開とIoTKit Edgeの受理カーソルに遅れは見つかりません。",
        )
    };
    stage(
        DiagnosticStageKind::RawCustody,
        state,
        code,
        facts.last_accepted_at,
        bounded_count(affected),
        "有効な収集ノードの受理カーソル集計",
        cause,
        "BrokerとIoTKit Edgeの受信状態を確認する",
        "/logs",
        None,
    )
}

async fn sensor_stage(storage: &Storage, now: i64) -> Result<DiagnosticStage, DiagnosticError> {
    // Asking for one more ref makes a >64 inventory explicit.  We never turn
    // a fresh prefix into a whole-inventory green result.
    let receipts = storage
        .diagnostic_signal_receipts((MAX_DIAGNOSTIC_SCOPE + 1) as i64)
        .await?;
    let mut facts = SensorFacts {
        inspected: receipts.len().min(MAX_DIAGNOSTIC_SCOPE),
        capped: receipts.len() > MAX_DIAGNOSTIC_SCOPE,
        ..SensorFacts::default()
    };
    for latest in receipts.into_iter().take(MAX_DIAGNOSTIC_SCOPE) {
        match latest {
            None => facts.missing += 1,
            Some(received_at) => {
                facts.last_success_at = max_option(facts.last_success_at, Some(received_at));
                if now.saturating_sub(received_at) > STALE_WORK_MS {
                    facts.stale += 1;
                }
            }
        }
    }
    let scope = if facts.capped {
        "64+件の現在登録済み信号（先頭64件を確認）".to_owned()
    } else {
        scope_count(facts.inspected as i64, "現在登録済み信号")
    };
    let (state, code, affected, cause) = if facts.inspected == 0 {
        (
            DiagnosticStageState::NotApplicable,
            "sensor_not_registered",
            0,
            "現在登録済みのセンサー信号はありません。",
        )
    } else if facts.capped {
        (
            DiagnosticStageState::Unknown,
            "sensor_scope_capped",
            0,
            "64件を超える登録済み信号があるため、確認範囲外を含む現在の入力状態は断定できません。",
        )
    } else if facts.missing > 0 {
        (
            DiagnosticStageState::Unknown,
            "sensor_no_raw_evidence",
            facts.missing,
            "受信済みのセンサー入力がないため、現在の入力状態を判断できません。",
        )
    } else if facts.stale > 0 {
        (
            DiagnosticStageState::Warning,
            "sensor_no_new_input_advisory",
            facts.stale,
            "5分を超えて新しいセンサー入力を受信していません。イベント駆動センサーの停止を意味するものではありません。",
        )
    } else {
        (
            DiagnosticStageState::Ok,
            "sensor_input_recent",
            0,
            "最近のセンサー入力を受信しています。",
        )
    };
    Ok(stage(
        DiagnosticStageKind::Sensor,
        state,
        code,
        facts.last_success_at,
        affected,
        &scope,
        cause,
        "センサー入力を確認する",
        "/equipment",
        None,
    ))
}

fn projection_stage(facts: &CausalFacts, now: i64) -> DiagnosticStage {
    if facts.active_rule_count == 0 {
        return stage(
            DiagnosticStageKind::Projection,
            DiagnosticStageState::NotApplicable,
            "projection_not_configured",
            None,
            0,
            "有効な計測ルールはありません。",
            "計測ルールを設定すると処理状態が表示されます。",
            "計測ルールを設定する",
            "/sensors",
            None,
        );
    }
    let (state, code, last_success_at, affected, cause) =
        if facts.unresolved_projection_failures > 0 {
            (
                DiagnosticStageState::Critical,
                "projection_active_failure",
                facts.projection_last_success_at,
                bounded_count(facts.unresolved_projection_failures),
                "有効な同じルールの後続成功がない計測ルール処理失敗があります。",
            )
        } else if facts
            .oldest_projection_at
            .is_some_and(|received_at| now.saturating_sub(received_at) > STALE_WORK_MS)
        {
            (
                DiagnosticStageState::Warning,
                "projection_queue_stale",
                facts.projection_last_success_at,
                1,
                "計測ルールの処理待ちデータが5分を超えています。",
            )
        } else {
            (
                DiagnosticStageState::Ok,
                "projection_current",
                facts.projection_last_success_at,
                0,
                "有効な計測ルールの処理待ちは期限内です。",
            )
        };
    stage(
        DiagnosticStageKind::Projection,
        state,
        code,
        last_success_at,
        affected,
        &format!(
            "{}件までの有効な計測ルール",
            bounded_count(facts.active_rule_count)
        ),
        cause,
        "計測ルールの処理状況を確認する",
        "/logs",
        None,
    )
}

fn output_stage(facts: &CausalFacts, now: i64) -> DiagnosticStage {
    if facts.active_output_route_count == 0 {
        return stage(
            DiagnosticStageKind::ExternalOutput,
            DiagnosticStageState::NotApplicable,
            "external_output_not_configured",
            None,
            0,
            "有効な外部出力はありません。",
            "外部出力を設定すると配信状態が表示されます。",
            "外部出力を設定する",
            "/output",
            None,
        );
    }
    let (state, code, last_success_at, affected, cause) =
        if facts.unresolved_output_transform_errors > 0 {
            (
                DiagnosticStageState::Critical,
                "external_output_transform_error",
                facts.latest_output_delivery_at,
                bounded_count(facts.unresolved_output_transform_errors),
                "後続成功で回復していない外部出力の変換エラーがあります。",
            )
        } else if facts
            .oldest_pending_output_at
            .is_some_and(|created_at| now.saturating_sub(created_at) > STALE_WORK_MS)
        {
            (
                DiagnosticStageState::Warning,
                "external_output_pending",
                facts.latest_output_delivery_at,
                1,
                "外部出力のPUBACK待ちデータが5分を超えています。",
            )
        } else {
            (
                DiagnosticStageState::Ok,
                "external_output_current",
                facts.latest_output_delivery_at,
                0,
                "外部出力の保留は期限内です。",
            )
        };
    stage(
        DiagnosticStageKind::ExternalOutput,
        state,
        code,
        last_success_at,
        affected,
        &format!(
            "{}件までの有効な外部出力",
            bounded_count(facts.active_output_route_count)
        ),
        cause,
        "外部出力の詳細を確認する",
        "/output",
        None,
    )
}

fn blocked_stage(
    stage_kind: DiagnosticStageKind,
    blocked_by: DiagnosticStageKind,
    code: &str,
    cause: &str,
    action: &str,
    href: &str,
) -> DiagnosticStage {
    stage(
        stage_kind,
        DiagnosticStageState::Unknown,
        code,
        None,
        0,
        "上流の新しい入力待ち",
        cause,
        action,
        href,
        Some(blocked_by),
    )
}

#[allow(clippy::too_many_arguments)]
fn stage(
    stage_kind: DiagnosticStageKind,
    state: DiagnosticStageState,
    code: &str,
    last_success_at: Option<i64>,
    affected_count: usize,
    scope: &str,
    cause: &str,
    action: &str,
    href: &str,
    blocked_by: Option<DiagnosticStageKind>,
) -> DiagnosticStage {
    DiagnosticStage {
        stage: stage_kind,
        state,
        code: code.into(),
        last_success_at,
        affected_count: affected_count.min(MAX_DIAGNOSTIC_SCOPE),
        scope: scope.into(),
        cause: cause.into(),
        action: action.into(),
        href: href.into(),
        blocked_by,
    }
}

fn is_healthy(state: DiagnosticStageState) -> bool {
    state == DiagnosticStageState::Ok || state == DiagnosticStageState::NotApplicable
}

fn max_option(current: Option<i64>, candidate: Option<i64>) -> Option<i64> {
    match (current, candidate) {
        (Some(current), Some(candidate)) => Some(current.max(candidate)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn bounded_count(value: i64) -> usize {
    usize::try_from(value.max(0))
        .unwrap_or(MAX_DIAGNOSTIC_SCOPE)
        .min(MAX_DIAGNOSTIC_SCOPE)
}

fn scope_count(value: i64, noun: &str) -> String {
    if value > MAX_DIAGNOSTIC_SCOPE as i64 {
        format!("{}+件の{noun}（集計）", MAX_DIAGNOSTIC_SCOPE)
    } else {
        format!("{}件の{noun}", value.max(0))
    }
}

fn update_report_state(report: &mut DiagnosticReport, state: DiagnosticStageState) {
    match state {
        DiagnosticStageState::Critical => report.state = DiagnosticState::Critical,
        DiagnosticStageState::Warning | DiagnosticStageState::Unknown
            if report.state == DiagnosticState::Healthy =>
        {
            report.state = DiagnosticState::Attention;
        }
        DiagnosticStageState::Ok
        | DiagnosticStageState::NotApplicable
        | DiagnosticStageState::Warning
        | DiagnosticStageState::Unknown => {}
    }
}

async fn causal_facts(storage: &Storage) -> Result<CausalFacts, DiagnosticError> {
    match storage.operation_backend() {
        OperationBackend::Sqlite { pool, .. } => causal_facts_from_sqlite(pool).await,
        OperationBackend::Postgres { pool, .. } => causal_facts_from_postgres(pool).await,
    }
}

async fn node_status_facts(
    storage: &Storage,
    now: i64,
) -> Result<NodeStatusFacts, DiagnosticError> {
    match storage.operation_backend() {
        OperationBackend::Sqlite { pool, .. } => {
            let row = sqlx::query(
                "WITH current AS (SELECT status.edge_node_id,status.last_live_received_at,status.collector_state,\
                   ?-status.last_live_received_at AS age FROM edge_node_activations AS activation \
                   LEFT JOIN edge_node_status AS status ON status.edge_node_id=activation.edge_node_id \
                   AND status.ledger_epoch=activation.ledger_epoch WHERE activation.state='active') \
                 SELECT COUNT(*) AS active_nodes,\
                 COALESCE(SUM(CASE WHEN edge_node_id IS NULL OR last_live_received_at IS NULL THEN 1 ELSE 0 END),0) AS unknown_nodes,\
                 COALESCE(SUM(CASE WHEN last_live_received_at IS NOT NULL AND age>=? AND age<? THEN 1 ELSE 0 END),0) AS stale_warning_nodes,\
                 COALESCE(SUM(CASE WHEN last_live_received_at IS NOT NULL AND age>=? THEN 1 ELSE 0 END),0) AS stale_critical_nodes,\
                 COALESCE(SUM(CASE WHEN last_live_received_at IS NOT NULL AND age<? AND collector_state='stopped' THEN 1 ELSE 0 END),0) AS stopped_collectors,\
                 MAX(CASE WHEN last_live_received_at IS NOT NULL AND age<? THEN last_live_received_at END) AS fresh_last_live_received_at,\
                 MAX(CASE WHEN last_live_received_at IS NOT NULL AND age>=? AND age<? THEN last_live_received_at END) AS stale_warning_last_live_received_at,\
                 MAX(CASE WHEN last_live_received_at IS NOT NULL AND age>=? THEN last_live_received_at END) AS stale_critical_last_live_received_at FROM current",
            )
            .bind(now)
            .bind(HEARTBEAT_WARNING_MS)
            .bind(HEARTBEAT_CRITICAL_MS)
            .bind(HEARTBEAT_CRITICAL_MS)
            .bind(HEARTBEAT_WARNING_MS)
            .bind(HEARTBEAT_WARNING_MS)
            .bind(HEARTBEAT_WARNING_MS)
            .bind(HEARTBEAT_CRITICAL_MS)
            .bind(HEARTBEAT_CRITICAL_MS)
            .fetch_one(pool)
            .await?;
            node_status_facts_from_row!(row)
        }
        OperationBackend::Postgres { pool, .. } => {
            let row = sqlx::query(
                "WITH current AS (SELECT status.edge_node_id,status.last_live_received_at,status.collector_state,\
                   $1-status.last_live_received_at AS age FROM edge_node_activations AS activation \
                   LEFT JOIN edge_node_status AS status ON status.edge_node_id=activation.edge_node_id \
                   AND status.ledger_epoch=activation.ledger_epoch WHERE activation.state='active') \
                 SELECT COUNT(*)::BIGINT AS active_nodes,\
                 COALESCE(SUM(CASE WHEN edge_node_id IS NULL OR last_live_received_at IS NULL THEN 1 ELSE 0 END),0)::BIGINT AS unknown_nodes,\
                 COALESCE(SUM(CASE WHEN last_live_received_at IS NOT NULL AND age>=$2 AND age<$3 THEN 1 ELSE 0 END),0)::BIGINT AS stale_warning_nodes,\
                 COALESCE(SUM(CASE WHEN last_live_received_at IS NOT NULL AND age>=$3 THEN 1 ELSE 0 END),0)::BIGINT AS stale_critical_nodes,\
                 COALESCE(SUM(CASE WHEN last_live_received_at IS NOT NULL AND age<$2 AND collector_state='stopped' THEN 1 ELSE 0 END),0)::BIGINT AS stopped_collectors,\
                 MAX(CASE WHEN last_live_received_at IS NOT NULL AND age<$2 THEN last_live_received_at END) AS fresh_last_live_received_at,\
                 MAX(CASE WHEN last_live_received_at IS NOT NULL AND age>=$2 AND age<$3 THEN last_live_received_at END) AS stale_warning_last_live_received_at,\
                 MAX(CASE WHEN last_live_received_at IS NOT NULL AND age>=$3 THEN last_live_received_at END) AS stale_critical_last_live_received_at FROM current",
            )
            .bind(now)
            .bind(HEARTBEAT_WARNING_MS)
            .bind(HEARTBEAT_CRITICAL_MS)
            .fetch_one(pool)
            .await?;
            node_status_facts_from_row!(row)
        }
    }
}

async fn adapter_facts(storage: &Storage, now: i64) -> Result<AdapterFacts, DiagnosticError> {
    match storage.operation_backend() {
        OperationBackend::Sqlite { pool, .. } => {
            let row = sqlx::query(
                "SELECT COUNT(adapter.value) AS adapters,\
                 COALESCE(SUM(CASE WHEN json_extract(adapter.value,'$.state')='restarting' THEN 1 ELSE 0 END),0) AS restarting,\
                 COALESCE(SUM(CASE WHEN json_extract(adapter.value,'$.state')='exhausted' THEN 1 ELSE 0 END),0) AS exhausted,\
                 COALESCE(SUM(CASE WHEN json_extract(adapter.value,'$.state')='stopped' THEN 1 ELSE 0 END),0) AS stopped,\
                 MAX(status.last_live_received_at) AS last_live_received_at \
                 FROM edge_node_activations AS activation JOIN edge_node_status AS status \
                 ON status.edge_node_id=activation.edge_node_id AND status.ledger_epoch=activation.ledger_epoch \
                 CROSS JOIN json_each(CAST(status.adapters_json AS TEXT)) AS adapter \
                 WHERE activation.state='active' AND status.last_live_received_at IS NOT NULL \
                 AND ?-status.last_live_received_at<=?",
            )
            .bind(now)
            .bind(HEARTBEAT_WARNING_MS)
            .fetch_one(pool)
            .await?;
            adapter_facts_from_row!(row)
        }
        OperationBackend::Postgres { pool, .. } => {
            let row = sqlx::query(
                "SELECT COUNT(adapter.value)::BIGINT AS adapters,\
                 COALESCE(SUM(CASE WHEN adapter.value->>'state'='restarting' THEN 1 ELSE 0 END),0)::BIGINT AS restarting,\
                 COALESCE(SUM(CASE WHEN adapter.value->>'state'='exhausted' THEN 1 ELSE 0 END),0)::BIGINT AS exhausted,\
                 COALESCE(SUM(CASE WHEN adapter.value->>'state'='stopped' THEN 1 ELSE 0 END),0)::BIGINT AS stopped,\
                 MAX(status.last_live_received_at) AS last_live_received_at \
                 FROM edge_node_activations AS activation JOIN edge_node_status AS status \
                 ON status.edge_node_id=activation.edge_node_id AND status.ledger_epoch=activation.ledger_epoch \
                 CROSS JOIN LATERAL jsonb_array_elements(status.adapters_json) AS adapter(value) \
                 WHERE activation.state='active' AND status.last_live_received_at IS NOT NULL \
                 AND $1-status.last_live_received_at<=$2",
            )
            .bind(now)
            .bind(HEARTBEAT_WARNING_MS)
            .fetch_one(pool)
            .await?;
            adapter_facts_from_row!(row)
        }
    }
}

async fn raw_custody_facts(
    storage: &Storage,
    now: i64,
) -> Result<RawCustodyFacts, DiagnosticError> {
    let stalled_before = now.saturating_sub(STALE_WORK_MS);
    match storage.operation_backend() {
        OperationBackend::Sqlite { pool, .. } => {
            let row = sqlx::query(
                "SELECT COALESCE(SUM(CASE WHEN status.accepted_through>COALESCE(cursor.accepted_through,0) THEN 1 ELSE 0 END),0) AS edge_cursor_behind,\
                 COALESCE(SUM(CASE WHEN status.pending_publications>0 AND COALESCE(cursor.accepted_through,0)=status.accepted_through \
                   AND status.pending_since_at IS NOT NULL AND status.pending_since_at<=? THEN 1 ELSE 0 END),0) AS stalled_equal_cursor,\
                 COALESCE(SUM(CASE WHEN status.pending_publications>0 AND cursor.accepted_through>status.accepted_through THEN 1 ELSE 0 END),0) AS edge_cursor_ahead,\
                 COALESCE(SUM(CASE WHEN status.storage_pressure=1 THEN 1 ELSE 0 END),0) AS storage_pressure,\
                 MAX(cursor.updated_at) AS last_accepted_at \
                 FROM edge_node_activations AS activation JOIN edge_node_status AS status \
                 ON status.edge_node_id=activation.edge_node_id AND status.ledger_epoch=activation.ledger_epoch \
                 LEFT JOIN accepted_cursors AS cursor ON cursor.edge_node_id=status.edge_node_id \
                 AND cursor.ledger_epoch=status.ledger_epoch WHERE activation.state='active'",
            )
            .bind(stalled_before)
            .fetch_one(pool)
            .await?;
            raw_custody_facts_from_row!(row)
        }
        OperationBackend::Postgres { pool, .. } => {
            let row = sqlx::query(
                "SELECT COALESCE(SUM(CASE WHEN status.accepted_through>COALESCE(cursor.accepted_through,0) THEN 1 ELSE 0 END),0)::BIGINT AS edge_cursor_behind,\
                 COALESCE(SUM(CASE WHEN status.pending_publications>0 AND COALESCE(cursor.accepted_through,0)=status.accepted_through \
                   AND status.pending_since_at IS NOT NULL AND status.pending_since_at<=$1 THEN 1 ELSE 0 END),0)::BIGINT AS stalled_equal_cursor,\
                 COALESCE(SUM(CASE WHEN status.pending_publications>0 AND cursor.accepted_through>status.accepted_through THEN 1 ELSE 0 END),0)::BIGINT AS edge_cursor_ahead,\
                 COALESCE(SUM(CASE WHEN status.storage_pressure=TRUE THEN 1 ELSE 0 END),0)::BIGINT AS storage_pressure,\
                 MAX(cursor.updated_at) AS last_accepted_at \
                 FROM edge_node_activations AS activation JOIN edge_node_status AS status \
                 ON status.edge_node_id=activation.edge_node_id AND status.ledger_epoch=activation.ledger_epoch \
                 LEFT JOIN accepted_cursors AS cursor ON cursor.edge_node_id=status.edge_node_id \
                 AND cursor.ledger_epoch=status.ledger_epoch WHERE activation.state='active'",
            )
            .bind(stalled_before)
            .fetch_one(pool)
            .await?;
            raw_custody_facts_from_row!(row)
        }
    }
}

async fn causal_facts_from_sqlite(pool: &sqlx::SqlitePool) -> Result<CausalFacts, DiagnosticError> {
    let active_rule_count =
        sqlx::query_scalar("SELECT COUNT(*) FROM semantic_rules WHERE active=1")
            .fetch_one(pool)
            .await?;
    let oldest_projection_at = sqlx::query_scalar(
        "SELECT MIN(queue.received_at) FROM semantic_projection_queue AS queue \
         JOIN semantic_rules AS rule ON rule.rule_id=queue.rule_id WHERE rule.active=1",
    )
    .fetch_one(pool)
    .await?;
    let projection_last_success_at =
        sqlx::query_scalar::<_, Option<i64>>(SQLITE_DIAGNOSTIC_PROJECTION_LATEST_SQL)
            .fetch_all(pool)
            .await?
            .into_iter()
            .flatten()
            .max();
    let projection_failure = sqlx::query(
        "SELECT COUNT(*) AS count,MAX(failure.last_failed_at) AS last_failed_at \
         FROM semantic_projection_failures AS failure \
         JOIN semantic_rules AS rule ON rule.rule_id=failure.rule_id AND rule.active=1 \
         JOIN semantic_signals AS signal ON signal.signal_ref=rule.signal_ref \
         JOIN edge_node_activations AS activation ON activation.edge_node_id=signal.edge_node_id \
           AND activation.ledger_epoch=failure.ledger_epoch AND activation.state='active' \
         WHERE NOT EXISTS(SELECT 1 FROM semantic_observations AS observation \
           WHERE observation.rule_id=failure.rule_id AND observation.ledger_epoch=failure.ledger_epoch \
             AND observation.source_pub_seq>failure.pub_seq)",
    )
    .fetch_one(pool)
    .await?;
    let active_output_route_count =
        sqlx::query_scalar("SELECT COUNT(*) FROM output_routes WHERE active=1")
            .fetch_one(pool)
            .await?;
    let oldest_pending_output_at =
        sqlx::query_scalar::<_, Option<i64>>(SQLITE_DIAGNOSTIC_OUTPUT_PENDING_OLDEST_SQL)
            .fetch_all(pool)
            .await?
            .into_iter()
            .flatten()
            .min();
    let output_delivery =
        sqlx::query_scalar::<_, Option<i64>>(SQLITE_DIAGNOSTIC_OUTPUT_DELIVERY_LATEST_SQL)
            .fetch_all(pool)
            .await?
            .into_iter()
            .flatten()
            .max();
    let output_error = sqlx::query(
        "SELECT COUNT(*) AS count,MAX(last_transform_error_at) AS last_error_at \
         FROM output_routes WHERE active=1 AND last_transform_error_code IS NOT NULL \
         AND (last_transform_success_at IS NULL OR last_transform_success_at<=last_transform_error_at)",
    )
    .fetch_one(pool)
    .await?;
    Ok(CausalFacts {
        active_rule_count,
        oldest_projection_at,
        projection_last_success_at,
        unresolved_projection_failures: projection_failure.try_get("count")?,
        active_output_route_count,
        oldest_pending_output_at,
        latest_output_delivery_at: output_delivery,
        unresolved_output_transform_errors: output_error.try_get("count")?,
    })
}

async fn causal_facts_from_postgres(pool: &sqlx::PgPool) -> Result<CausalFacts, DiagnosticError> {
    let active_rule_count =
        sqlx::query_scalar("SELECT COUNT(*) FROM semantic_rules WHERE active=TRUE")
            .fetch_one(pool)
            .await?;
    let oldest_projection_at = sqlx::query_scalar(
        "SELECT MIN(queue.received_at) FROM semantic_projection_queue AS queue \
         JOIN semantic_rules AS rule ON rule.rule_id=queue.rule_id WHERE rule.active=TRUE",
    )
    .fetch_one(pool)
    .await?;
    let projection_last_success_at =
        sqlx::query_scalar::<_, Option<i64>>(POSTGRES_DIAGNOSTIC_PROJECTION_LATEST_SQL)
            .fetch_all(pool)
            .await?
            .into_iter()
            .flatten()
            .max();
    let projection_failure = sqlx::query(
        "SELECT COUNT(*) AS count,MAX(failure.last_failed_at) AS last_failed_at \
         FROM semantic_projection_failures AS failure \
         JOIN semantic_rules AS rule ON rule.rule_id=failure.rule_id AND rule.active=TRUE \
         JOIN semantic_signals AS signal ON signal.signal_ref=rule.signal_ref \
         JOIN edge_node_activations AS activation ON activation.edge_node_id=signal.edge_node_id \
           AND activation.ledger_epoch=failure.ledger_epoch AND activation.state='active' \
         WHERE NOT EXISTS(SELECT 1 FROM semantic_observations AS observation \
           WHERE observation.rule_id=failure.rule_id AND observation.ledger_epoch=failure.ledger_epoch \
             AND observation.source_pub_seq>failure.pub_seq)",
    )
    .fetch_one(pool)
    .await?;
    let active_output_route_count =
        sqlx::query_scalar("SELECT COUNT(*) FROM output_routes WHERE active=TRUE")
            .fetch_one(pool)
            .await?;
    let oldest_pending_output_at =
        sqlx::query_scalar::<_, Option<i64>>(POSTGRES_DIAGNOSTIC_OUTPUT_PENDING_OLDEST_SQL)
            .fetch_all(pool)
            .await?
            .into_iter()
            .flatten()
            .min();
    let output_delivery =
        sqlx::query_scalar::<_, Option<i64>>(POSTGRES_DIAGNOSTIC_OUTPUT_DELIVERY_LATEST_SQL)
            .fetch_all(pool)
            .await?
            .into_iter()
            .flatten()
            .max();
    let output_error = sqlx::query(
        "SELECT COUNT(*) AS count,MAX(last_transform_error_at) AS last_error_at \
         FROM output_routes WHERE active=TRUE AND last_transform_error_code IS NOT NULL \
         AND (last_transform_success_at IS NULL OR last_transform_success_at<=last_transform_error_at)",
    )
    .fetch_one(pool)
    .await?;
    Ok(CausalFacts {
        active_rule_count,
        oldest_projection_at,
        projection_last_success_at,
        unresolved_projection_failures: projection_failure.try_get("count")?,
        active_output_route_count,
        oldest_pending_output_at,
        latest_output_delivery_at: output_delivery,
        unresolved_output_transform_errors: output_error.try_get("count")?,
    })
}

fn read_certificate_status(path: &Path, now: i64) -> Option<CertificateStatus> {
    let encoded = fs::read(path).ok()?;
    let (_, pem) = parse_x509_pem(&encoded).ok()?;
    let (_, certificate) =
        x509_parser::certificate::X509Certificate::from_der(&pem.contents).ok()?;
    let not_after = certificate
        .validity()
        .not_after
        .timestamp()
        .checked_mul(1000)?;
    Some(certificate_status(not_after, now))
}

fn certificate_status(not_after: i64, now: i64) -> CertificateStatus {
    let days_remaining = not_after.saturating_sub(now) / (24 * 60 * 60 * 1000);
    CertificateStatus {
        available: true,
        not_after: Some(not_after),
        days_remaining: Some(days_remaining),
        needs_action: not_after <= now || days_remaining < 30,
    }
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

fn push_resource_issue(
    report: &mut DiagnosticReport,
    code: &str,
    severity: &str,
    component: &str,
    resource_ref: String,
    summary: &str,
    observed_at: Option<i64>,
) {
    let issue_count = report.issues.len();
    push_issue(report, code, severity, component, summary);
    if report.issues.len() > issue_count
        && let Some(issue) = report.issues.last_mut()
    {
        issue.resource_ref = Some(resource_ref);
        issue.observed_at = observed_at;
    }
}

async fn sqlite_table_exists(pool: &sqlx::SqlitePool, table: &str) -> Result<bool, sqlx::Error> {
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?")
            .bind(table)
            .fetch_one(pool)
            .await?;
    Ok(count != 0)
}

async fn postgres_table_exists(pool: &sqlx::PgPool, table: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(format!("public.{table}"))
        .fetch_one(pool)
        .await
}

async fn sqlite_counts(
    pool: &sqlx::SqlitePool,
) -> Result<(i64, i64, i64, i64, i64, Option<String>, Option<i64>), sqlx::Error> {
    let raw = sqlx::query_scalar("SELECT count(*) FROM raw_records")
        .fetch_one(pool)
        .await?;
    let semantic = sqlite_optional_count(pool, "semantic_observations", None).await?;
    let projection_pending = sqlite_optional_count(pool, "semantic_projection_queue", None).await?;
    let pending =
        sqlite_optional_count(pool, "output_outbox", Some("published_at IS NULL")).await?;
    let failures = sqlite_optional_count(pool, "semantic_projection_failures", None).await?;
    let backup = sqlx::query(
        "SELECT backup_id, created_at FROM edge_backup_events \
         ORDER BY created_at DESC, backup_id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok((
        raw,
        semantic,
        projection_pending,
        pending,
        failures,
        backup.as_ref().map(|row| row.get::<String, _>("backup_id")),
        backup.as_ref().map(|row| row.get::<i64, _>("created_at")),
    ))
}

async fn postgres_counts(
    pool: &sqlx::PgPool,
) -> Result<(i64, i64, i64, i64, i64, Option<String>, Option<i64>), sqlx::Error> {
    let raw = sqlx::query_scalar("SELECT count(*) FROM raw_records")
        .fetch_one(pool)
        .await?;
    let semantic = postgres_optional_count(pool, "semantic_observations", None).await?;
    let projection_pending =
        postgres_optional_count(pool, "semantic_projection_queue", None).await?;
    let pending =
        postgres_optional_count(pool, "output_outbox", Some("published_at IS NULL")).await?;
    let failures = postgres_optional_count(pool, "semantic_projection_failures", None).await?;
    let backup = sqlx::query(
        "SELECT backup_id, created_at FROM edge_backup_events \
         ORDER BY created_at DESC, backup_id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok((
        raw,
        semantic,
        projection_pending,
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

#[cfg(test)]
#[path = "../../tests/unit/diagnostics_tests.rs"]
mod tests;
