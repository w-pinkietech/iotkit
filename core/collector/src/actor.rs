use crate::freshness::{FreshnessClock, FreshnessLimits, FreshnessSnapshot, UntrustedSystemClock};
use crate::principal::DevicePrincipalIssuer;
use crate::principal::IngestPrincipal;
use crate::principal::LocalPrincipalIssuer;
use crate::registry_policy::{RegistryPolicy, RegistryVerdict, is_series_level};
use iotkit_core_ledger as ledger;
use iotkit_core_storage::DbHandle;
use iotkit_core_timeseries as ts;
use iotkit_ingest_contract::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

pub const MAX_ITEMS_PER_ENVELOPE: usize = 256;

/// `ingest_dedup` の保持TTL(D1: sender_id+envelope_idキーはTTL+サイズ上限で有界)。
pub const DEDUP_TTL_MS: i64 = 72 * 60 * 60 * 1000;

/// 日和見パージの既定発火間隔。本番はこの値、テストは`spawn_with_purge_interval`で0を注入する。
pub const DEFAULT_PURGE_INTERVAL_MS: i64 = 60 * 60 * 1000;

#[derive(Debug, Clone)]
pub struct IngestRequest {
    pub principal: IngestPrincipal,
    pub envelope: Envelope,
}

struct QueuedRequest {
    request: IngestRequest,
    output: RequestedOutput,
}

enum RequestedOutput {
    Submit(oneshot::Sender<Result<EnvelopeAck, SubmitError>>),
    Validate(oneshot::Sender<Result<ValidationReport, SubmitError>>),
}

/// Sender handles expose submission only; authenticated-principal authority is returned once
/// from `Collector::spawn_device_composed` and cannot be recovered from a clone.
///
/// ```compile_fail
/// use iotkit_core_collector::Collector;
/// fn cannot_mint_from_sender(collector: &Collector) {
///     let _issuer = collector.device_principal_issuer();
/// }
/// ```
#[derive(Clone)]
pub struct Collector {
    tx: mpsc::Sender<QueuedRequest>,
}

/// Bounded security signal emitted without including hostile payload text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntrusionSignal {
    pub principal_id: String,
    pub credential_id: Option<String>,
    pub kind: IntrusionKind,
}

/// Stable intrusion categories exposed by the collector hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntrusionKind {
    SourceMismatch,
    SubjectScopeViolation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitError {
    /// ackなし=未耐久(ストレージ失敗等)。コレクタは生存しており、同一envelope_idの再送で
    /// 回復可能(D1)。再送/スプールは送信側(計画3のアダプタ内クライアント)の責務。
    NoAck,
    /// Absolute freshness comparison requires the shared trusted wall clock.
    /// Network Task 5 maps this to 503 with no ingest acknowledgement.
    ClockUntrusted,
    /// The credential proof lost its ordering race with a committed authority mutation.
    /// Network ingress maps this to 401 with no acknowledgement.
    AuthenticationStale,
    /// コレクタタスク死亡(キュー閉鎖)。送信を継続しても回復しない。
    Closed,
}

/// タスク所有キャッシュ(D5: 起動時全ロードはWave 0では行数が小さいため遅延ロードで開始し、
/// ミス時にDBを引く。iotkit-edge-nodectl(別プロセス)変異はgeneration counterで無効化する(T4、D5決定3))
#[derive(Default)]
struct ResolutionCache {
    generation: i64,
    devices: HashMap<String, (ledger::SystemId, ledger::DeviceState)>, // hardware_id →
    series: HashMap<(ledger::SystemId, String, i32, String), i64>,
}

impl Collector {
    /// Edge Node composition receives both non-cloneable issuer capabilities exactly once.
    pub fn spawn_fully_composed(
        db: DbHandle,
        policy: Arc<dyn RegistryPolicy>,
        queue_cap: usize,
    ) -> (
        Collector,
        LocalPrincipalIssuer,
        DevicePrincipalIssuer,
        tokio::task::JoinHandle<()>,
    ) {
        let (collector, handle) = Self::spawn(db, policy, queue_cap);
        (
            collector,
            LocalPrincipalIssuer::new(),
            DevicePrincipalIssuer::new(),
            handle,
        )
    }

    /// Start the collector and return the non-cloneable receiver composition
    /// capability used to mint principals before handing sender-only handles to
    /// adapters.
    pub fn spawn_composed(
        db: DbHandle,
        policy: Arc<dyn RegistryPolicy>,
        queue_cap: usize,
    ) -> (Collector, LocalPrincipalIssuer, tokio::task::JoinHandle<()>) {
        let (collector, handle) = Self::spawn(db, policy, queue_cap);
        (collector, LocalPrincipalIssuer::new(), handle)
    }

    /// Start a collector and return the non-cloneable device-auth composition capability.
    /// Ordinary `Collector` clones expose submission only and cannot recreate this issuer.
    pub fn spawn_device_composed(
        db: DbHandle,
        policy: Arc<dyn RegistryPolicy>,
        queue_cap: usize,
    ) -> (
        Collector,
        DevicePrincipalIssuer,
        tokio::task::JoinHandle<()>,
    ) {
        let (collector, handle) = Self::spawn(db, policy, queue_cap);
        (collector, DevicePrincipalIssuer::new(), handle)
    }

    pub fn spawn(
        db: DbHandle,
        policy: Arc<dyn RegistryPolicy>,
        queue_cap: usize,
    ) -> (Collector, tokio::task::JoinHandle<()>) {
        Self::spawn_with_components(
            db,
            policy,
            queue_cap,
            DEFAULT_PURGE_INTERVAL_MS,
            Arc::new(UntrustedSystemClock),
            FreshnessLimits::default(),
            None,
        )
    }

    /// `spawn`と同じだが、日和見dedupパージの発火間隔を注入できる(テスト用: 0を渡すと
    /// 処理成功のたびに毎回パージ判定が真になり、パージ経路をアクター経由で検証できる)。
    pub fn spawn_with_purge_interval(
        db: DbHandle,
        policy: Arc<dyn RegistryPolicy>,
        queue_cap: usize,
        purge_interval_ms: i64,
    ) -> (Collector, tokio::task::JoinHandle<()>) {
        Self::spawn_with_components(
            db,
            policy,
            queue_cap,
            purge_interval_ms,
            Arc::new(UntrustedSystemClock),
            FreshnessLimits::default(),
            None,
        )
    }

    /// Construct the collector with receiver-owned clock evidence and an optional
    /// bounded intrusion channel. The channel is written only with `try_send`, so
    /// a saturated security sink cannot block custody processing.
    pub fn spawn_with_components(
        db: DbHandle,
        policy: Arc<dyn RegistryPolicy>,
        queue_cap: usize,
        purge_interval_ms: i64,
        freshness_clock: Arc<dyn FreshnessClock>,
        freshness_limits: FreshnessLimits,
        intrusion_tx: Option<mpsc::Sender<IntrusionSignal>>,
    ) -> (Collector, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<QueuedRequest>(queue_cap);
        let handle = tokio::spawn(async move {
            let mut cache = ResolutionCache::default();
            let mut last_purge_ms = now_ms();
            let mut maintenance_latch = DedupMaintenanceLatch::default();
            while let Some(req) = rx.recv().await {
                let taken = std::mem::take(&mut cache);
                let policy = Arc::clone(&policy);
                let request = req.request;
                let commit = matches!(req.output, RequestedOutput::Submit(_));
                let purge_after_success = commit;
                let freshness_clock = Arc::clone(&freshness_clock);
                let intrusion_tx = intrusion_tx.clone();
                let result = db
                    .with_conn(move |conn| {
                        let mut c = taken;
                        let outcome = process_envelope_mode(
                            conn,
                            &mut c,
                            policy.as_ref(),
                            freshness_clock.as_ref(),
                            freshness_limits,
                            intrusion_tx.as_ref(),
                            &request,
                            commit,
                        );
                        Ok((outcome, c))
                    })
                    .await;
                match result {
                    Ok((Ok(ack), c)) => {
                        cache = c;
                        match req.output {
                            RequestedOutput::Submit(tx) => {
                                let _ = tx.send(Ok(ack));
                            }
                            RequestedOutput::Validate(tx) => {
                                let _ = tx.send(Ok(validation_report_from_ack(ack)));
                            }
                        }
                        if purge_after_success {
                            // 計画4のTTL/保持ワイヤリング着地までの日和見パージ(受理トランザクション
                            // の外・別途with_connで実行。ack耐久性には影響しない)。Validationは
                            // product/custody stateへの書き込みを一切起動しない。
                            maybe_purge_dedup(
                                &db,
                                purge_interval_ms,
                                &mut last_purge_ms,
                                &mut maintenance_latch,
                            )
                            .await;
                        }
                    }
                    Ok((Err(ProcessError::ClockUntrusted), c)) => {
                        cache = c;
                        match req.output {
                            RequestedOutput::Submit(tx) => {
                                let _ = tx.send(Err(SubmitError::ClockUntrusted));
                            }
                            RequestedOutput::Validate(tx) => {
                                let _ = tx.send(Err(SubmitError::ClockUntrusted));
                            }
                        }
                    }
                    Ok((Err(ProcessError::AuthenticationStale), c)) => {
                        cache = c;
                        match req.output {
                            RequestedOutput::Submit(tx) => {
                                let _ = tx.send(Err(SubmitError::AuthenticationStale));
                            }
                            RequestedOutput::Validate(tx) => {
                                let _ = tx.send(Err(SubmitError::AuthenticationStale));
                            }
                        }
                    }
                    Ok((Err(ProcessError::Storage(e)), _c)) => {
                        tracing::error!(error = %e, "collector: storage failure (envelope aborted)");
                        // ロールバックでキャッシュ済みseries_id(・devices)が無効化されうるため
                        // 全捨てが安全。process_itemはensure_seriesのINSERT直後(コミット前)に
                        // cache.seriesへ書くので、部分ロールバックされたcが持つseries_idはDBに
                        // 実在しない可能性がある。保全すると幻のseries_idが残り、以降の同キー
                        // envelopeがFK違反→ackなしの無限ループになる(再送で回復しない=D1違反)。
                        cache = ResolutionCache::default();
                        // ack_tx をドロップ = 送信側はタイムアウトで再送(ackなし=未耐久、D1と整合)
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "collector: storage failure");
                        // ack_tx をドロップ = 送信側はタイムアウトで再送(ackなし=未耐久、D1と整合)
                    }
                }
            }
        });
        (Collector { tx }, handle)
    }

    pub async fn submit(&self, request: IngestRequest) -> Result<EnvelopeAck, SubmitError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send(QueuedRequest {
                request,
                output: RequestedOutput::Submit(ack_tx),
            })
            .await
            .map_err(|_| SubmitError::Closed)?;
        // ack_txドロップ(ストレージ失敗)はNoAck。コレクタ死亡による中途ドロップも
        // 保守的にNoAckとする(次のsubmitがClosedを返す)。
        ack_rx.await.map_err(|_| SubmitError::NoAck)?
    }

    /// Run the same deterministic collector checks while rolling back all product and custody
    /// writes. Security intrusion signals remain allowed by the Plan 6 validation contract.
    pub async fn validate(&self, request: IngestRequest) -> Result<ValidationReport, SubmitError> {
        let (report_tx, report_rx) = oneshot::channel();
        self.tx
            .send(QueuedRequest {
                request,
                output: RequestedOutput::Validate(report_tx),
            })
            .await
            .map_err(|_| SubmitError::Closed)?;
        report_rx.await.map_err(|_| SubmitError::NoAck)?
    }
}

#[derive(Debug)]
enum ProcessError {
    Storage(String),
    ClockUntrusted,
    AuthenticationStale,
}

impl From<String> for ProcessError {
    fn from(value: String) -> Self {
        Self::Storage(value)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 日和見dedupパージ: 最終パージから`purge_interval_ms`超過していれば
/// `purge_dedup_before(now - DEDUP_TTL_MS)`を実行する。受理トランザクションの外(別途with_conn)
/// で行うため、ack耐久点(D1)には影響しない。パージ自体の失敗は致命的ではないのでログのみ。
#[derive(Debug, Clone, Copy)]
struct PendingDedupMaintenanceTransition {
    at_ms: i64,
    failed: bool,
}

#[derive(Debug, Default)]
struct DedupMaintenanceLatch {
    pending: Option<PendingDedupMaintenanceTransition>,
}

async fn maybe_purge_dedup(
    db: &DbHandle,
    purge_interval_ms: i64,
    last_purge_ms: &mut i64,
    latch: &mut DedupMaintenanceLatch,
) {
    let now = now_ms();
    if now.saturating_sub(*last_purge_ms) < purge_interval_ms {
        latch.retry(db).await;
        return;
    }
    *last_purge_ms = now;
    let cutoff = now - DEDUP_TTL_MS;
    let result = db
        .with_conn(move |conn| Ok(ts::purge_dedup_before(conn, cutoff).map_err(|e| e.to_string())))
        .await;
    let purge_failed = match result {
        Ok(Ok(deleted)) => {
            if deleted > 0 {
                tracing::info!(
                    deleted,
                    cutoff_ms = cutoff,
                    "collector: opportunistic ingest_dedup purge"
                );
            }
            false
        }
        Ok(Err(e)) => {
            tracing::error!(error = %e, "collector: dedup purge failed");
            true
        }
        Err(e) => {
            tracing::error!(error = %e, "collector: dedup purge failed (db)");
            true
        }
    };

    if latch.pending.is_some_and(|pending| !pending.failed) && purge_failed {
        // A recovery transaction that never committed cannot describe a newer failed purge.
        latch.pending = None;
    } else if !latch.retry(db).await {
        return;
    }
    if !record_dedup_maintenance_transition(db, now, purge_failed).await {
        latch.pending = Some(PendingDedupMaintenanceTransition {
            at_ms: now,
            failed: purge_failed,
        });
    }
}

impl DedupMaintenanceLatch {
    async fn retry(&mut self, db: &DbHandle) -> bool {
        let Some(pending) = self.pending else {
            return true;
        };
        if record_dedup_maintenance_transition(db, pending.at_ms, pending.failed).await {
            self.pending = None;
            true
        } else {
            false
        }
    }
}

async fn record_dedup_maintenance_transition(db: &DbHandle, now: i64, failed: bool) -> bool {
    let result = db
        .with_conn(move |conn| {
            Ok((|| -> Result<(), String> {
                let tx = rusqlite::Transaction::new_unchecked(
                    conn,
                    rusqlite::TransactionBehavior::Immediate,
                )
                .map_err(|error| error.to_string())?;
                let transitioned = if failed {
                    ts::mark_dedup_purge_failed(&tx, now).map_err(|error| error.to_string())?
                } else {
                    ts::mark_dedup_purge_recovered(&tx, now)
                        .map_err(|error| error.to_string())?
                };
                if transitioned {
                    let (kind, detail) = if failed {
                        (
                            "dedup_window_degraded",
                            r#"{"state":"degraded","operator_action":"Check database storage and run retention maintenance; duplicate suppression is reduced until recovery."}"#,
                        )
                    } else {
                        (
                            "dedup_window_recovered",
                            r#"{"state":"recovered","summary":"configured duplicate-suppression maintenance resumed"}"#,
                        )
                    };
                    ledger::record_event(&tx, kind, None, detail)
                        .map_err(|error| error.to_string())?;
                }
                tx.commit().map_err(|error| error.to_string())?;
                Ok(())
            })())
        })
        .await;
    match result {
        Err(error) => {
            tracing::error!(error = %error, "collector: dedup maintenance health update failed");
            false
        }
        Ok(Err(error)) => {
            tracing::error!(error = %error, "collector: dedup maintenance health update failed");
            false
        }
        Ok(Ok(())) => true,
    }
}

/// 1エンベロープの受理。**全体が単一トランザクション**(dedup+全item書き込み=ack耐久点、D1)。
///
/// ストレージ起因の失敗(Err)はack終端(Rejected)を作らない。rejected=送信側spool除去なので、
/// 未耐久データにRejectedを返すと無音損失になる(D1)。呼び出し元はErrに対してack_txを
/// ドロップし、送信側の再送に委ねる。トランザクションはコミットせずに終わるため自動ロールバック
/// される(部分コミットしない)。
#[cfg(test)]
fn process_envelope(
    conn: &rusqlite::Connection,
    cache: &mut ResolutionCache,
    policy: &dyn RegistryPolicy,
    freshness_clock: &dyn FreshnessClock,
    freshness_limits: FreshnessLimits,
    intrusion_tx: Option<&mpsc::Sender<IntrusionSignal>>,
    request: &IngestRequest,
) -> Result<EnvelopeAck, ProcessError> {
    process_envelope_mode(
        conn,
        cache,
        policy,
        freshness_clock,
        freshness_limits,
        intrusion_tx,
        request,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn process_envelope_mode(
    conn: &rusqlite::Connection,
    cache: &mut ResolutionCache,
    policy: &dyn RegistryPolicy,
    freshness_clock: &dyn FreshnessClock,
    freshness_limits: FreshnessLimits,
    intrusion_tx: Option<&mpsc::Sender<IntrusionSignal>>,
    request: &IngestRequest,
    commit: bool,
) -> Result<EnvelopeAck, ProcessError> {
    let principal = &request.principal;
    let envelope = &request.envelope;
    let eid = envelope.envelope_id.clone();
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| ProcessError::Storage(e.to_string()))?;
    if !authentication_is_current_at_serialization(&tx, principal)? {
        return Err(ProcessError::AuthenticationStale);
    }
    if envelope.source != principal.configured_source() {
        emit_intrusion(intrusion_tx, principal, IntrusionKind::SourceMismatch);
        return Ok(EnvelopeAck {
            envelope_id: eid,
            status: AckStatus::Rejected {
                reason_code: ReasonCode::SubjectScopeViolation,
                message: "configured source does not match authenticated principal".into(),
                field_path: Some("/source".into()),
                schema_hint: Some("configured authenticated source identity".into()),
            },
        });
    }
    if envelope.items.len() > MAX_ITEMS_PER_ENVELOPE {
        // 決定的な契約違反(サイズ超過はDBに触れる前に判定できる) → 終端Rejectedを維持
        return Ok(EnvelopeAck {
            envelope_id: eid,
            status: AckStatus::Rejected {
                reason_code: ReasonCode::BatchTooLarge,
                message: format!(
                    "items {} > {}",
                    envelope.items.len(),
                    MAX_ITEMS_PER_ENVELOPE
                ),
                field_path: Some("/items".into()),
                schema_hint: Some(format!("at most {MAX_ITEMS_PER_ENVELOPE} items")),
            },
        });
    }
    let clock = freshness_clock
        .snapshot(&tx)
        .map_err(ProcessError::Storage)?;
    if clock.trusted_wall_time_ms.is_none() && envelope.items.iter().any(requires_absolute_time) {
        return Err(ProcessError::ClockUntrusted);
    }
    let generation = ledger::current_generation(&tx).map_err(|e| e.to_string())?;
    if generation != cache.generation {
        cache.devices.clear();
        cache.series.clear();
        cache.generation = generation;
    }
    if commit {
        let claimed = ts::try_claim_envelope_bounded_at(
            &tx,
            principal.principal_id(),
            &envelope.envelope_id,
            clock.received_at_ms,
            ts::DedupLimits::default(),
        )
        .map_err(|e| e.to_string())?;
        if !claimed {
            drop(tx); // dedup判定のみ・書き込みなし
            return Ok(EnvelopeAck {
                envelope_id: eid,
                status: AckStatus::Duplicate,
            });
        }
    }
    let received_at = clock.received_at_ms;
    let epoch = ledger::ledger_epoch(&tx).map_err(|e| e.to_string())?;
    let item_context = ItemContext {
        principal,
        received_at,
        clock,
        freshness_limits,
        intrusion_tx,
        epoch: &epoch,
    };
    let mut item_statuses = Vec::with_capacity(envelope.items.len());
    let mut pending_sightings = Vec::new();
    for (item_index, item) in envelope.items.iter().enumerate() {
        item_statuses.push(process_item(
            &tx,
            cache,
            policy,
            item,
            item_index,
            &item_context,
            &mut pending_sightings,
        )?);
    }
    if !pending_sightings.is_empty() {
        let staged = pending_sightings
            .iter()
            .map(|sighting| ts::StagedSighting {
                hardware_id: &sighting.hardware_id,
                payload_json: &sighting.payload_json,
            })
            .collect::<Vec<_>>();
        let bounded = ts::stage_sightings_at(
            &tx,
            principal.principal_id(),
            received_at,
            &staged,
            ts::StagingLimits::default(),
        )
        .map_err(|e| e.to_string())?;
        for sighting in &pending_sightings {
            ledger::record_sighting(&tx, &sighting.hardware_id, principal.principal_id())
                .map_err(|e| e.to_string())?;
        }
        if bounded.expired_subjects > 0 || bounded.evicted_subjects > 0 {
            let detail = serde_json::json!({
                "expired_subjects": bounded.expired_subjects,
                "evicted_subjects": bounded.evicted_subjects,
            });
            ledger::record_event(&tx, "staging_bounds", None, &detail.to_string())
                .map_err(|e| e.to_string())?;
        }
    }
    if commit {
        tx.commit().map_err(|e| e.to_string())?;
    } else {
        tx.rollback().map_err(|e| e.to_string())?;
        // Validation may have populated IDs for rows that were deliberately rolled back.
        cache.devices.clear();
        cache.series.clear();
    }
    Ok(EnvelopeAck {
        envelope_id: eid,
        status: AckStatus::Accepted {
            items: item_statuses,
        },
    })
}

fn authentication_is_current_at_serialization(
    conn: &rusqlite::Connection,
    principal: &IngestPrincipal,
) -> Result<bool, ProcessError> {
    if principal.actor_kind() != crate::principal::IngestActorKind::DeviceToken {
        return Ok(true);
    }
    let (Some(credential_id), Some(auth_epoch), Some(auth_generation), Some(material_generation)) = (
        principal.credential_id(),
        principal.auth_epoch(),
        principal.auth_generation(),
        principal.principal_material_generation(),
    ) else {
        #[cfg(test)]
        return Ok(true);
        #[cfg(not(test))]
        return Ok(false);
    };
    conn.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM auth_state a
           JOIN device_credentials c ON c.credential_id=?1
           JOIN live_device_ingest_principals p ON p.principal_id=c.principal_id
           WHERE a.id=1 AND c.principal_id=?2 AND c.auth_epoch=?3
             AND c.state IN ('current','pending')
             AND a.auth_epoch=?3 AND a.auth_generation=?4
             AND a.device_credential_generation=?5
         )",
        rusqlite::params![
            credential_id,
            principal.principal_id(),
            auth_epoch,
            auth_generation,
            material_generation,
        ],
        |row| row.get(0),
    )
    .map_err(|error| ProcessError::Storage(error.to_string()))
}

fn validation_report_from_ack(ack: EnvelopeAck) -> ValidationReport {
    let issues = match ack.status {
        AckStatus::Accepted { items } => items
            .into_iter()
            .enumerate()
            .filter_map(|(item_index, status)| match status {
                ItemStatus::Stored { .. } => None,
                ItemStatus::ItemRejected {
                    reason_code,
                    message,
                    field_path,
                    schema_hint,
                } => Some(ValidationIssue {
                    item_index: Some(item_index),
                    reason_code,
                    message,
                    field_path,
                    schema_hint,
                }),
            })
            .collect(),
        AckStatus::Rejected {
            reason_code,
            message,
            field_path,
            schema_hint,
        } => vec![ValidationIssue {
            item_index: None,
            reason_code,
            message,
            field_path,
            schema_hint,
        }],
        AckStatus::Duplicate | AckStatus::Deferred => Vec::new(),
    };
    ValidationReport {
        envelope_id: ack.envelope_id,
        valid: issues.is_empty(),
        issues,
    }
}

fn emit_intrusion(
    intrusion_tx: Option<&mpsc::Sender<IntrusionSignal>>,
    principal: &IngestPrincipal,
    kind: IntrusionKind,
) {
    if let Some(tx) = intrusion_tx {
        let _ = tx.try_send(IntrusionSignal {
            principal_id: principal.principal_id().to_string(),
            credential_id: principal.credential_id().map(str::to_string),
            kind,
        });
    }
}

fn requires_absolute_time(item: &ReadingItem) -> bool {
    item.device_time_ms.is_some()
        && matches!(
            item.time_source,
            TimeSource::DeviceNtp | TimeSource::DeviceRtc | TimeSource::EdgeNodeAdjusted
        )
}

fn restore_device_time(
    received_at: i64,
    device_time_ms: Option<i64>,
    age_ms: Option<u64>,
    declared: TimeSource,
) -> (Option<i64>, TimeSource) {
    match (device_time_ms, age_ms) {
        (Some(dt), _) => (Some(dt), declared),
        (None, Some(age)) => match i64::try_from(age)
            .ok()
            .and_then(|a| received_at.checked_sub(a))
        {
            Some(dt) => (Some(dt), TimeSource::EdgeNodeAdjusted),
            None => (None, declared),
        },
        (None, None) => (None, declared),
    }
}

struct ItemContext<'a> {
    principal: &'a IngestPrincipal,
    received_at: i64,
    clock: FreshnessSnapshot,
    freshness_limits: FreshnessLimits,
    intrusion_tx: Option<&'a mpsc::Sender<IntrusionSignal>>,
    epoch: &'a str,
}

struct PendingSighting {
    hardware_id: String,
    payload_json: String,
}

fn stage_unknown_item_with<E>(
    item: &ReadingItem,
    hardware_id: &str,
    pending_sightings: &mut Vec<PendingSighting>,
    serialize: impl FnOnce(&ReadingItem) -> Result<String, E>,
) -> Result<ItemStatus, String>
where
    E: std::fmt::Display,
{
    let payload_json = serialize(item).map_err(|error| error.to_string())?;
    pending_sightings.push(PendingSighting {
        hardware_id: hardware_id.to_string(),
        payload_json,
    });
    Ok(ItemStatus::Stored {
        disposition: Disposition::Staged,
        quarantine_reason: None, // stagedとquarantinedは直列に成立しない(D1: subject解決が常に先)
    })
}

fn process_item(
    conn: &rusqlite::Connection,
    cache: &mut ResolutionCache,
    policy: &dyn RegistryPolicy,
    item: &ReadingItem,
    item_index: usize,
    context: &ItemContext<'_>,
    pending_sightings: &mut Vec<PendingSighting>,
) -> Result<ItemStatus, String> {
    let principal = context.principal;
    // 1) subject解決とscope検査。principalだけが省略解決・認可を決める。
    // scope違反は他のsender-controlled item fieldで隠せないよう、schema検査より先に行う。
    let (hardware_hint, resolved) = match item.subject_hint.as_deref() {
        Some(hw) => {
            let resolved = match cache.devices.get(hw) {
                Some(hit) => Some(*hit),
                None => {
                    match ledger::find_alive_by_hardware_id(conn, hw).map_err(|e| e.to_string())? {
                        Some(row) => {
                            cache
                                .devices
                                .insert(hw.to_string(), (row.system_id, row.state));
                            Some((row.system_id, row.state))
                        }
                        None => None,
                    }
                }
            };
            (Some(hw), resolved)
        }
        None => match principal.sole_subject() {
            Some(system_id) => {
                let row = ledger::get_device(conn, &system_id).map_err(|e| e.to_string())?;
                let resolved = row
                    .filter(|row| row.state != ledger::DeviceState::Retired)
                    .map(|row| (row.system_id, row.state));
                (None, resolved)
            }
            None => {
                return Ok(ItemStatus::ItemRejected {
                    reason_code: ReasonCode::UnknownSubject,
                    message: "subject_hint required for a multi-subject principal".into(),
                    field_path: Some(format!("/items/{item_index}/subject_hint")),
                    schema_hint: Some("one subject identifier from the principal scope".into()),
                });
            }
        },
    };

    if let Some((system_id, _)) = resolved
        && !principal.subject_allowed(&system_id)
    {
        emit_intrusion(
            context.intrusion_tx,
            principal,
            IntrusionKind::SubjectScopeViolation,
        );
        return Ok(ItemStatus::ItemRejected {
            reason_code: ReasonCode::SubjectScopeViolation,
            message: "subject is outside the authenticated principal scope".into(),
            field_path: Some(format!("/items/{item_index}/subject_hint")),
            schema_hint: Some("subject authorized for this principal".into()),
        });
    }

    // 2) 文法検査。scope認可済みのitemについて決定的契約違反を返す。
    if let Err(e) = validate_measurement_key(&item.measurement_key) {
        return Ok(ItemStatus::ItemRejected {
            reason_code: ReasonCode::MalformedMeasurementKey,
            message: e.to_string(),
            field_path: Some(format!("/items/{item_index}/measurement_key")),
            schema_hint: Some("canonical measurement key".into()),
        });
    }

    if !item.values.iter().all(|value| value.is_finite()) {
        return Ok(ItemStatus::ItemRejected {
            reason_code: ReasonCode::ValueTypeMismatch,
            message: "values must contain only finite numbers".into(),
            field_path: Some(format!("/items/{item_index}/values")),
            schema_hint: Some("array of finite numbers".into()),
        });
    }

    if let Some(rejected) =
        freshness_rejection(item, item_index, context.clock, context.freshness_limits)
    {
        return Ok(rejected);
    }

    let Some((system_id, state)) = resolved else {
        let field_path = Some(format!("/items/{item_index}/subject_hint"));
        let Some(hw) = hardware_hint else {
            return Ok(ItemStatus::ItemRejected {
                reason_code: ReasonCode::UnknownSubject,
                message: "the principal's sole subject is not registered".into(),
                field_path,
                schema_hint: Some("registered subject identifier".into()),
            });
        };
        if !principal.can_stage_unknown() {
            return Ok(ItemStatus::ItemRejected {
                reason_code: ReasonCode::UnknownSubject,
                message: "subject is not registered".into(),
                field_path,
                schema_hint: Some("registered subject identifier".into()),
            });
        }
        // 3) trusted official principalだけが未知subjectを目撃ステージングできる。
        return stage_unknown_item_with(item, hw, pending_sightings, serde_json::to_string);
    };
    // 4) レジストリ評価(D6判別表)。Errはストレージ失敗=ackなしへ伝播(D1)
    let (resolved_key, channel, registry_quarantine) =
        match policy.evaluate(conn, &system_id, item)? {
            RegistryVerdict::Accept {
                resolved_key,
                channel_index,
                quarantine,
            } => (resolved_key, channel_index, quarantine),
            RegistryVerdict::RejectItem {
                reason_code,
                message,
            } => {
                return Ok(ItemStatus::ItemRejected {
                    reason_code,
                    message,
                    field_path: None,
                    schema_hint: None,
                });
            }
        };
    // 5) series解決(検疫デバイスのデータは検疫行として保存=D1オンボーディング)。
    //    チャネルは評価器が返した正準値をそのまま使う(再計算しない=Global Constraints)
    let device_quarantined = state == ledger::DeviceState::Quarantined;
    let variant = item
        .series_variant
        .as_deref()
        .unwrap_or(ledger::DEFAULT_VARIANT)
        .to_string();
    let series_quarantined = registry_quarantine.is_some_and(is_series_level);
    let skey = (system_id, resolved_key.clone(), channel, variant.clone());
    let series_id = match cache.series.get(&skey) {
        Some(id) => *id,
        None => {
            let reason = registry_quarantine
                .filter(|q| is_series_level(*q))
                .map(|q| q.as_str());
            let id = ledger::ensure_series(
                conn,
                &system_id,
                &resolved_key,
                channel,
                &variant,
                series_quarantined,
                reason,
            )
            .map_err(|e| e.to_string())?;
            cache.series.insert(skey, id);
            id
        }
    };
    // 6) 書き込み+ackへの検疫理由可視化(D1追補)。レジストリ起因の理由が具体的なので優先し、
    //    無ければデバイス検疫を報告する
    let row_quarantined = registry_quarantine.is_some() || device_quarantined;
    let wire_reason = registry_quarantine
        .or_else(|| device_quarantined.then_some(QuarantineReason::DeviceQuarantined));
    // D1: RTCなしデバイスのage_ms → received_at - age_ms で復元(time_source=edge_node_adjusted)。
    // item.device_time_msが既にあればそれが優先(申告時刻>復元時刻)。
    let (device_time_ms, time_source) = restore_device_time(
        context.received_at,
        item.device_time_ms,
        item.age_ms,
        item.time_source,
    );
    let time_source = match time_source {
        TimeSource::DeviceNtp => "device_ntp",
        TimeSource::DeviceRtc => "device_rtc",
        TimeSource::EdgeNode => "edge_node",
        TimeSource::EdgeNodeAdjusted => "edge_node_adjusted",
    };
    let new = ts::NewReading {
        series_id,
        received_at_ms: context.received_at,
        device_time_ms,
        time_source: time_source.to_string(),
        values: item.values.clone(),
        rssi: item.rssi,
        battery_pct: item.battery_pct,
        quarantined: row_quarantined,
    };
    let seq = ts::insert_reading_v3(conn, &new).map_err(|e| e.to_string())?;
    if !row_quarantined
        && iotkit_core_publish::activation::publication_admitted(conn).map_err(|e| e.to_string())?
    {
        iotkit_core_publish::store::enqueue_measurement(
            conn,
            context.epoch,
            seq,
            context.received_at,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(ItemStatus::Stored {
        disposition: if row_quarantined {
            Disposition::Quarantined
        } else {
            Disposition::Durable
        },
        quarantine_reason: if row_quarantined { wire_reason } else { None },
    })
}

fn freshness_rejection(
    item: &ReadingItem,
    item_index: usize,
    clock: FreshnessSnapshot,
    limits: FreshnessLimits,
) -> Option<ItemStatus> {
    if item.device_time_ms.is_none()
        && item
            .age_ms
            .is_some_and(|age| age > limits.max_age_ms() as u64)
    {
        return Some(ItemStatus::ItemRejected {
            reason_code: ReasonCode::StaleTimestamp,
            message: "observation age exceeds the configured freshness window".into(),
            field_path: Some(format!("/items/{item_index}/age_ms")),
            schema_hint: Some(format!("age_ms <= {}", limits.max_age_ms())),
        });
    }

    if requires_absolute_time(item) {
        let now = clock
            .trusted_wall_time_ms
            .expect("absolute freshness is preflighted against clock trust");
        let timestamp = item
            .device_time_ms
            .expect("absolute time requires timestamp");
        if timestamp < now.saturating_sub(limits.max_age_ms()) {
            return Some(ItemStatus::ItemRejected {
                reason_code: ReasonCode::StaleTimestamp,
                message: "device timestamp is older than the configured freshness window".into(),
                field_path: Some(format!("/items/{item_index}/device_time_ms")),
                schema_hint: Some(format!(
                    "device_time_ms >= trusted_now_ms - {}",
                    limits.max_age_ms()
                )),
            });
        }
        if timestamp > now.saturating_add(limits.max_future_skew_ms()) {
            return Some(ItemStatus::ItemRejected {
                reason_code: ReasonCode::StaleTimestamp,
                message: "device timestamp exceeds the configured future-skew limit".into(),
                field_path: Some(format!("/items/{item_index}/device_time_ms")),
                schema_hint: Some(format!(
                    "device_time_ms <= trusted_now_ms + {}",
                    limits.max_future_skew_ms()
                )),
            });
        }
    }
    None
}

#[cfg(test)]
#[path = "../tests/unit/actor_tests.rs"]
mod tests;
