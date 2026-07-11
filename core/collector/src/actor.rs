use crate::freshness::{FreshnessClock, FreshnessLimits, FreshnessSnapshot, UntrustedSystemClock};
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
    ack_tx: oneshot::Sender<Result<EnvelopeAck, SubmitError>>,
}

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
    /// コレクタタスク死亡(キュー閉鎖)。送信を継続しても回復しない。
    Closed,
}

/// タスク所有キャッシュ(D5: 起動時全ロードはWave 0では行数が小さいため遅延ロードで開始し、
/// ミス時にDBを引く。gatewayctl(別プロセス)変異はgeneration counterで無効化する(T4、D5決定3))
#[derive(Default)]
struct ResolutionCache {
    generation: i64,
    devices: HashMap<String, (ledger::SystemId, ledger::DeviceState)>, // hardware_id →
    series: HashMap<(ledger::SystemId, String, i32, String), i64>,
}

impl Collector {
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
            while let Some(req) = rx.recv().await {
                let taken = std::mem::take(&mut cache);
                let policy = Arc::clone(&policy);
                let request = req.request;
                let freshness_clock = Arc::clone(&freshness_clock);
                let intrusion_tx = intrusion_tx.clone();
                let result = db
                    .with_conn(move |conn| {
                        let mut c = taken;
                        let outcome = process_envelope(
                            conn,
                            &mut c,
                            policy.as_ref(),
                            freshness_clock.as_ref(),
                            freshness_limits,
                            intrusion_tx.as_ref(),
                            &request,
                        );
                        Ok((outcome, c))
                    })
                    .await;
                match result {
                    Ok((Ok(ack), c)) => {
                        cache = c;
                        let _ = req.ack_tx.send(Ok(ack));
                        // 計画4のTTL/保持ワイヤリング着地までの日和見パージ(受理トランザクション
                        // の外・別途with_connで実行。ack耐久性には影響しない)。
                        maybe_purge_dedup(&db, purge_interval_ms, &mut last_purge_ms).await;
                    }
                    Ok((Err(ProcessError::ClockUntrusted), c)) => {
                        cache = c;
                        let _ = req.ack_tx.send(Err(SubmitError::ClockUntrusted));
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
            .send(QueuedRequest { request, ack_tx })
            .await
            .map_err(|_| SubmitError::Closed)?;
        // ack_txドロップ(ストレージ失敗)はNoAck。コレクタ死亡による中途ドロップも
        // 保守的にNoAckとする(次のsubmitがClosedを返す)。
        ack_rx.await.map_err(|_| SubmitError::NoAck)?
    }
}

#[derive(Debug)]
enum ProcessError {
    Storage(String),
    ClockUntrusted,
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
async fn maybe_purge_dedup(db: &DbHandle, purge_interval_ms: i64, last_purge_ms: &mut i64) {
    let now = now_ms();
    if now.saturating_sub(*last_purge_ms) < purge_interval_ms {
        return;
    }
    *last_purge_ms = now;
    let cutoff = now - DEDUP_TTL_MS;
    let result = db
        .with_conn(move |conn| Ok(ts::purge_dedup_before(conn, cutoff).map_err(|e| e.to_string())))
        .await;
    match result {
        Ok(Ok(deleted)) => {
            if deleted > 0 {
                tracing::info!(
                    deleted,
                    cutoff_ms = cutoff,
                    "collector: opportunistic ingest_dedup purge"
                );
            }
        }
        Ok(Err(e)) => tracing::error!(error = %e, "collector: dedup purge failed"),
        Err(e) => tracing::error!(error = %e, "collector: dedup purge failed (db)"),
    }
}

/// 1エンベロープの受理。**全体が単一トランザクション**(dedup+全item書き込み=ack耐久点、D1)。
///
/// ストレージ起因の失敗(Err)はack終端(Rejected)を作らない。rejected=送信側spool除去なので、
/// 未耐久データにRejectedを返すと無音損失になる(D1)。呼び出し元はErrに対してack_txを
/// ドロップし、送信側の再送に委ねる。トランザクションはコミットせずに終わるため自動ロールバック
/// される(部分コミットしない)。
fn process_envelope(
    conn: &rusqlite::Connection,
    cache: &mut ResolutionCache,
    policy: &dyn RegistryPolicy,
    freshness_clock: &dyn FreshnessClock,
    freshness_limits: FreshnessLimits,
    intrusion_tx: Option<&mpsc::Sender<IntrusionSignal>>,
    request: &IngestRequest,
) -> Result<EnvelopeAck, ProcessError> {
    let principal = &request.principal;
    let envelope = &request.envelope;
    let eid = envelope.envelope_id.clone();
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
        .snapshot(conn)
        .map_err(ProcessError::Storage)?;
    if clock.trusted_wall_time_ms.is_none() && envelope.items.iter().any(requires_absolute_time) {
        return Err(ProcessError::ClockUntrusted);
    }
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| ProcessError::Storage(e.to_string()))?;
    let generation = ledger::current_generation(&tx).map_err(|e| e.to_string())?;
    if generation != cache.generation {
        cache.devices.clear();
        cache.series.clear();
        cache.generation = generation;
    }
    let claimed = ts::try_claim_envelope(&tx, principal.principal_id(), &envelope.envelope_id)
        .map_err(|e| e.to_string())?;
    if !claimed {
        drop(tx); // dedup判定のみ・書き込みなし
        return Ok(EnvelopeAck {
            envelope_id: eid,
            status: AckStatus::Duplicate,
        });
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
    for (item_index, item) in envelope.items.iter().enumerate() {
        item_statuses.push(process_item(
            &tx,
            cache,
            policy,
            item,
            item_index,
            &item_context,
        )?);
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(EnvelopeAck {
        envelope_id: eid,
        status: AckStatus::Accepted {
            items: item_statuses,
        },
    })
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
            TimeSource::DeviceNtp | TimeSource::DeviceRtc | TimeSource::GatewayAdjusted
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
            Some(dt) => (Some(dt), TimeSource::GatewayAdjusted),
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

fn process_item(
    conn: &rusqlite::Connection,
    cache: &mut ResolutionCache,
    policy: &dyn RegistryPolicy,
    item: &ReadingItem,
    item_index: usize,
    context: &ItemContext<'_>,
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
        let payload = serde_json::to_string(item).unwrap_or_else(|_| "{}".into());
        ledger::record_sighting(conn, hw, principal.principal_id()).map_err(|e| e.to_string())?;
        ts::insert_staged_reading(conn, hw, context.received_at, &payload)
            .map_err(|e| e.to_string())?;
        return Ok(ItemStatus::Stored {
            disposition: Disposition::Staged,
            quarantine_reason: None, // stagedとquarantinedは直列に成立しない(D1: subject解決が常に先)
        });
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
    // D1: RTCなしデバイスのage_ms → received_at - age_ms で復元(time_source=gateway_adjusted)。
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
        TimeSource::Gateway => "gateway",
        TimeSource::GatewayAdjusted => "gateway_adjusted",
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
    if !row_quarantined {
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
mod tests {
    use super::*;
    use crate::registry_policy::PermissiveRegistry;
    use iotkit_core_ledger as ledger;
    use std::sync::Arc;

    fn test_db() -> iotkit_core_storage::DbHandle {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(ledger::MIGRATIONS);
        all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
        all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
        all.sort_by_key(|m| m.version);
        iotkit_core_storage::init_db_memory(&all).unwrap()
    }

    fn migration_set() -> Vec<iotkit_core_storage::Migration> {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(ledger::MIGRATIONS);
        all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
        all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
        all.sort_by_key(|m| m.version);
        all
    }

    fn env(id: &str, hw: &str, key: &str) -> IngestRequest {
        let envelope = Envelope {
            envelope_id: id.into(),
            source: "test-adapter".into(),
            declaration_version: None,
            items: vec![ReadingItem {
                subject_hint: Some(hw.into()),
                measurement_key: key.into(),
                channel_index: None,
                series_variant: None,
                values: vec![1.0],
                device_time_ms: None,
                time_source: TimeSource::Gateway,
                age_ms: None,
                rssi: None,
                battery_pct: None,
            }],
        };
        IngestRequest {
            principal: IngestPrincipal::trusted_official_adapter(
                "principal:test-adapter",
                "test-adapter",
            ),
            envelope,
        }
    }

    fn process_test(
        conn: &rusqlite::Connection,
        cache: &mut ResolutionCache,
        policy: &dyn RegistryPolicy,
        request: &IngestRequest,
    ) -> Result<EnvelopeAck, ProcessError> {
        process_envelope(
            conn,
            cache,
            policy,
            &UntrustedSystemClock,
            FreshnessLimits::default(),
            None,
            request,
        )
    }

    fn register_active_id(db: &iotkit_core_storage::DbHandle, hw: &str) -> ledger::SystemId {
        db.with_conn_sync(|conn| {
            let id = ledger::insert_device(
                conn,
                &ledger::NewDevice {
                    hardware_id: hw.into(),
                    user_label: None,
                    parent: None,
                    kind: ledger::DeviceKind::Individual,
                    initial_state: ledger::DeviceState::Active,
                },
            )
            .unwrap();
            Ok(id)
        })
        .unwrap()
    }

    fn register_active(db: &iotkit_core_storage::DbHandle, hw: &str) {
        let _ = register_active_id(db, hw);
    }

    struct FixedFreshnessClock(FreshnessSnapshot);

    impl FreshnessClock for FixedFreshnessClock {
        fn snapshot(&self, _conn: &rusqlite::Connection) -> Result<FreshnessSnapshot, String> {
            Ok(self.0)
        }
    }

    fn raw_channel(item: &ReadingItem) -> i32 {
        item.channel_index
            .map(i32::from)
            .unwrap_or(ledger::CHANNEL_NA)
    }

    /// 検疫理由付きのスタブポリシー(コレクタがverdictをseries/行/ackへ正しく写像するかの検証用)
    struct QuarantiningStub(QuarantineReason);
    impl crate::registry_policy::RegistryPolicy for QuarantiningStub {
        fn evaluate(
            &self,
            _conn: &rusqlite::Connection,
            _system_id: &ledger::SystemId,
            item: &ReadingItem,
        ) -> Result<crate::registry_policy::RegistryVerdict, String> {
            Ok(crate::registry_policy::RegistryVerdict::Accept {
                resolved_key: item.measurement_key.clone(),
                channel_index: raw_channel(item),
                quarantine: Some(self.0),
            })
        }
    }

    /// キーとチャネルを書き換えるスタブ(verdictの写像がseries実体化に反映されるかの検証用)
    struct RenamingStub;
    impl crate::registry_policy::RegistryPolicy for RenamingStub {
        fn evaluate(
            &self,
            _conn: &rusqlite::Connection,
            _system_id: &ledger::SystemId,
            _item: &ReadingItem,
        ) -> Result<crate::registry_policy::RegistryVerdict, String> {
            Ok(crate::registry_policy::RegistryVerdict::Accept {
                resolved_key: "temperature_c".into(),
                channel_index: 7, // コレクタが自前計算せずverdictの値を使うことの検証
                quarantine: None,
            })
        }
    }

    /// Errを返すスタブ(ストレージ失敗の伝播=ackなしの検証用)
    struct FailingPolicy;
    impl crate::registry_policy::RegistryPolicy for FailingPolicy {
        fn evaluate(
            &self,
            _conn: &rusqlite::Connection,
            _system_id: &ledger::SystemId,
            _item: &ReadingItem,
        ) -> Result<crate::registry_policy::RegistryVerdict, String> {
            Err("simulated registry storage failure".into())
        }
    }

    #[tokio::test]
    async fn known_subject_is_accepted_durable_and_row_exists_before_ack_returns() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let ack = collector
            .submit(env("e-1", "ble:aa", "temperature_c"))
            .await
            .unwrap();
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Durable, .. })));
        // ack = 耐久点: ackが返った時点で行が存在する(D1)
        let n: i64 = db
            .with_conn_sync(|conn| {
                Ok(conn
                    .query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0))
                    .unwrap())
            })
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn unknown_key_quarantine_marks_series_row_and_ack_reason() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(
            db.clone(),
            Arc::new(QuarantiningStub(QuarantineReason::UnknownKey)),
            16,
        );
        let ack = collector
            .submit(env("e-q1", "ble:aa", "custom.mystery"))
            .await
            .unwrap();
        assert!(
            matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored {
                disposition: Disposition::Quarantined,
                quarantine_reason: Some(QuarantineReason::UnknownKey),
            })),
            "ackに検疫理由が可視化される(D1追補)"
        );
        let (s_q, s_reason, r_q): (i64, Option<String>, i64) = db
            .with_conn_sync(|conn| {
                Ok((
                    conn.query_row("SELECT quarantined FROM series", [], |r| r.get(0))
                        .unwrap(),
                    conn.query_row("SELECT quarantine_reason FROM series", [], |r| r.get(0))
                        .unwrap(),
                    conn.query_row("SELECT quarantined FROM readings", [], |r| r.get(0))
                        .unwrap(),
                ))
            })
            .unwrap();
        assert_eq!(s_q, 1, "unknown keyはseries級検疫");
        assert_eq!(s_reason.as_deref(), Some("unknown_key"));
        assert_eq!(r_q, 1);
    }

    #[tokio::test]
    async fn out_of_range_quarantines_row_but_not_series() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(
            db.clone(),
            Arc::new(QuarantiningStub(QuarantineReason::OutOfRange)),
            16,
        );
        let ack = collector
            .submit(env("e-q2", "ble:aa", "temperature_c"))
            .await
            .unwrap();
        assert!(matches!(ack.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored {
            disposition: Disposition::Quarantined,
            quarantine_reason: Some(QuarantineReason::OutOfRange),
        })));
        let (s_q, s_reason, r_q): (i64, Option<String>, i64) = db
            .with_conn_sync(|conn| {
                Ok((
                    conn.query_row("SELECT quarantined FROM series", [], |r| r.get(0))
                        .unwrap(),
                    conn.query_row("SELECT quarantine_reason FROM series", [], |r| r.get(0))
                        .unwrap(),
                    conn.query_row("SELECT quarantined FROM readings", [], |r| r.get(0))
                        .unwrap(),
                ))
            })
            .unwrap();
        assert_eq!(s_q, 0, "値域外はseriesを汚さない(行級のみ)");
        assert_eq!(s_reason, None);
        assert_eq!(r_q, 1);
    }

    #[test]
    fn non_quarantined_reading_is_enqueued_to_outbox_same_tx() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let mut cache = ResolutionCache::default();
        let envelope = env("e-outbox-1", "ble:aa", "temperature_c");

        let ack = db
            .with_conn_sync(|conn| {
                Ok(process_test(conn, &mut cache, &PermissiveRegistry, &envelope).unwrap())
            })
            .unwrap();
        assert!(matches!(ack.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored {
            disposition: Disposition::Durable,
            quarantine_reason: None,
        })));

        let (reading_count, reading_seq, outbox_count): (i64, i64, i64) = db
            .with_conn_sync(|conn| {
                Ok((
                    conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0))
                        .unwrap(),
                    conn.query_row("SELECT seq FROM readings", [], |r| r.get(0))
                        .unwrap(),
                    conn.query_row(
                        "SELECT COUNT(*) FROM publication_log WHERE kind = 'measurement'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap(),
                ))
            })
            .unwrap();
        assert_eq!(reading_count, 1);
        assert_eq!(outbox_count, 1, "non-quarantined readings must be enqueued");

        let (outbox_reading_seq, outbox_epoch, expected_epoch): (i64, String, String) = db
            .with_conn_sync(|conn| {
                let expected_epoch = ledger::ledger_epoch(conn).unwrap();
                let (outbox_reading_seq, outbox_epoch) = conn
                    .query_row(
                        "SELECT reading_seq, epoch FROM publication_log WHERE kind = 'measurement'",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap();
                Ok((outbox_reading_seq, outbox_epoch, expected_epoch))
            })
            .unwrap();
        assert_eq!(outbox_reading_seq, reading_seq);
        assert_eq!(outbox_epoch, expected_epoch);

        let quarantined_db = test_db();
        register_active(&quarantined_db, "ble:qq");
        let mut quarantine_cache = ResolutionCache::default();
        let quarantined = env("e-outbox-q", "ble:qq", "custom.mystery");

        let ack = quarantined_db
            .with_conn_sync(|conn| {
                Ok(process_test(
                    conn,
                    &mut quarantine_cache,
                    &QuarantiningStub(QuarantineReason::UnknownKey),
                    &quarantined,
                )
                .unwrap())
            })
            .unwrap();
        assert!(matches!(ack.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored {
            disposition: Disposition::Quarantined,
            quarantine_reason: Some(QuarantineReason::UnknownKey),
        })));

        let (quarantined_readings, quarantined_outbox): (i64, i64) = quarantined_db
            .with_conn_sync(|conn| {
                Ok((
                    conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0))
                        .unwrap(),
                    conn.query_row(
                        "SELECT COUNT(*) FROM publication_log WHERE kind = 'measurement'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap(),
                ))
            })
            .unwrap();
        assert_eq!(quarantined_readings, 1);
        assert_eq!(
            quarantined_outbox, 0,
            "quarantined readings must not be enqueued"
        );
    }

    #[test]
    fn multi_item_envelope_enqueues_distinct_outbox_rows() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let mut cache = ResolutionCache::default();
        let mut envelope = env("e-outbox-multi", "ble:aa", "temperature_c");
        envelope.envelope.items.push(ReadingItem {
            subject_hint: Some("ble:aa".into()),
            measurement_key: "humidity_pct".into(),
            channel_index: None,
            series_variant: None,
            values: vec![55.0],
            device_time_ms: None,
            time_source: TimeSource::Gateway,
            age_ms: None,
            rssi: None,
            battery_pct: None,
        });

        let ack = db
            .with_conn_sync(|conn| {
                Ok(process_test(conn, &mut cache, &PermissiveRegistry, &envelope).unwrap())
            })
            .unwrap();
        assert!(matches!(ack.status,
        AckStatus::Accepted { ref items }
        if items.len() == 2 && items.iter().all(|status| matches!(status,
            ItemStatus::Stored {
                disposition: Disposition::Durable,
                quarantine_reason: None,
            }))));

        let (reading_seqs, outbox_rows): (Vec<i64>, Vec<(i64, i64)>) = db
            .with_conn_sync(|conn| {
                let reading_seqs = conn
                    .prepare("SELECT seq FROM readings ORDER BY seq")?
                    .query_map([], |r| r.get(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                let outbox_rows = conn
                    .prepare(
                        "SELECT pub_seq, reading_seq FROM publication_log
                 WHERE kind = 'measurement'
                 ORDER BY pub_seq",
                    )?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((reading_seqs, outbox_rows))
            })
            .unwrap();
        assert_eq!(reading_seqs.len(), 2);
        assert_ne!(reading_seqs[0], reading_seqs[1]);

        assert_eq!(
            outbox_rows.len(),
            2,
            "each non-quarantined reading must be enqueued"
        );
        let outbox_reading_seqs: Vec<i64> = outbox_rows
            .iter()
            .map(|(_pub_seq, reading_seq)| *reading_seq)
            .collect();
        assert_eq!(outbox_reading_seqs, reading_seqs);
        assert_ne!(outbox_rows[0].0, outbox_rows[1].0);
    }

    #[tokio::test]
    async fn device_quarantine_is_visible_as_ack_reason() {
        // 検疫状態デバイス(D5経路A: 承認→検疫→active の途中)のデータは行検疫+理由device_quarantined
        let db = test_db();
        db.with_conn_sync(|conn| {
            ledger::record_sighting(conn, "ble:q", "test-adapter").unwrap();
            ledger::approve_sighting(conn, "ble:q", None, ledger::DeviceKind::Individual).unwrap();
            Ok(())
        })
        .unwrap();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let ack = collector
            .submit(env("e-dq", "ble:q", "temperature_c"))
            .await
            .unwrap();
        let AckStatus::Accepted { items } = ack.status else {
            panic!("expected Accepted")
        };
        assert!(matches!(
            items[0],
            ItemStatus::Stored {
                disposition: Disposition::Quarantined,
                quarantine_reason: Some(QuarantineReason::DeviceQuarantined),
            }
        ));
    }

    #[tokio::test]
    async fn resolution_cache_invalidated_on_generation_bump() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("iotkit.db");
        let migrations = migration_set();
        let db = iotkit_core_storage::init_db(&db_path, &migrations).unwrap();
        let ctl_db = iotkit_core_storage::init_db(&db_path, &migrations).unwrap();

        let system_id = ctl_db
            .with_conn_sync(|conn| {
                ledger::record_sighting(conn, "ble:gen", "test-adapter").unwrap();
                let sid = ledger::approve_sighting(
                    conn,
                    "ble:gen",
                    Some("generation test"),
                    ledger::DeviceKind::Individual,
                )
                .unwrap();
                Ok(sid)
            })
            .unwrap();

        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let first = collector
            .submit(env("e-gen-1", "ble:gen", "temperature_c"))
            .await
            .unwrap();
        assert!(matches!(first.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored {
            disposition: Disposition::Quarantined,
            quarantine_reason: Some(QuarantineReason::DeviceQuarantined),
        })));

        ctl_db
            .with_conn_sync(move |conn| {
                let tx = rusqlite::Transaction::new_unchecked(
                    conn,
                    rusqlite::TransactionBehavior::Immediate,
                )
                .unwrap();
                ledger::activate_device(&tx, &system_id).unwrap();
                ledger::bump_generation(&tx).unwrap();
                tx.commit().unwrap();
                Ok(())
            })
            .unwrap();

        let second = collector
            .submit(env("e-gen-2", "ble:gen", "temperature_c"))
            .await
            .unwrap();
        assert!(
            matches!(second.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored {
                disposition: Disposition::Durable,
                quarantine_reason: None,
            })),
            "generation bump must clear cached quarantined device state"
        );
    }

    #[tokio::test]
    async fn verdict_resolved_key_and_channel_are_used_for_series() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(RenamingStub), 16);
        collector
            .submit(env("e-alias", "ble:aa", "temp_old"))
            .await
            .unwrap();
        let (key, ch): (String, i32) = db
            .with_conn_sync(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT measurement_key, channel_index FROM series",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap())
            })
            .unwrap();
        assert_eq!(key, "temperature_c", "series実体化はresolved_keyを使う");
        assert_eq!(
            ch, 7,
            "コレクタはチャネルを再計算せずverdictのchannel_indexを使う"
        );
    }

    #[tokio::test]
    async fn age_ms_restores_gateway_adjusted_device_time() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let mut e = env("e-age", "ble:aa", "temperature_c");
        e.envelope.items[0].age_ms = Some(5000);
        e.envelope.items[0].time_source = TimeSource::Gateway;
        e.envelope.items[0].device_time_ms = None;
        collector.submit(e).await.unwrap();

        let (received_at, device_time, time_source, event_time, event_time_source):
            (i64, i64, String, i64, String) = db.with_conn_sync(|conn| {
            Ok(conn.query_row(
                "SELECT received_at, device_time, time_source, event_time, event_time_source FROM readings",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            ).unwrap())
        }).unwrap();
        assert_eq!(device_time, received_at - 5000);
        assert_eq!(time_source, "gateway_adjusted");
        assert_eq!(event_time, received_at - 5000);
        assert_eq!(event_time_source, "gateway_adjusted");
    }

    #[tokio::test]
    async fn overflow_scale_age_is_terminally_rejected_before_reconstruction() {
        let db = test_db();
        let (collector, _h) = Collector::spawn(db, Arc::new(PermissiveRegistry), 16);
        let mut request = env("age-overflow", "ble:aa", "temperature_c");
        request.envelope.items[0].age_ms = Some(u64::MAX);

        let ack = collector.submit(request).await.unwrap();

        assert!(matches!(ack.status, AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::ItemRejected {
                reason_code: ReasonCode::StaleTimestamp,
                ref field_path,
                ..
            } if field_path.as_deref() == Some("/items/0/age_ms"))));
    }

    #[test]
    fn restore_device_time_ignores_unrepresentable_age_ms() {
        let (device_time, source) =
            restore_device_time(10_000, None, Some(i64::MAX as u64 + 1), TimeSource::Gateway);
        assert_eq!(device_time, None);
        assert_eq!(source, TimeSource::Gateway);
    }

    #[test]
    fn restore_device_time_ignores_age_ms_that_would_underflow() {
        let (device_time, source) =
            restore_device_time(i64::MIN, None, Some(1), TimeSource::Gateway);
        assert_eq!(device_time, None);
        assert_eq!(source, TimeSource::Gateway);
    }

    #[test]
    fn restore_device_time_age_zero_returns_received_at() {
        let (device_time, source) = restore_device_time(10_000, None, Some(0), TimeSource::Gateway);
        assert_eq!(device_time, Some(10_000));
        assert_eq!(source, TimeSource::GatewayAdjusted);
    }

    #[test]
    fn restore_device_time_prefers_declared_device_time() {
        let (device_time, source) =
            restore_device_time(10_000, Some(9000), Some(5000), TimeSource::DeviceNtp);
        assert_eq!(device_time, Some(9000));
        assert_eq!(source, TimeSource::DeviceNtp);
    }

    #[tokio::test]
    async fn policy_storage_failure_produces_no_ack() {
        // レジストリ評価のErrはRejectedではなくackなし(D1。計画1 T6教訓の踏襲)
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(FailingPolicy), 16);
        let result = collector
            .submit(env("e-fail", "ble:aa", "temperature_c"))
            .await;
        assert!(matches!(result, Err(SubmitError::NoAck)));
        let n: i64 = db
            .with_conn_sync(|conn| {
                Ok(conn
                    .query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0))
                    .unwrap())
            })
            .unwrap();
        assert_eq!(n, 0, "エンベロープ全体がロールバックされる");
    }

    #[test]
    fn series_level_quarantine_reason_classification_matches_d6() {
        assert!(is_series_level(QuarantineReason::UnknownKey));
        assert!(is_series_level(QuarantineReason::UndeclaredChannel));
        assert!(!is_series_level(QuarantineReason::OutOfRange));
        assert!(!is_series_level(QuarantineReason::DeviceQuarantined));
    }

    #[tokio::test]
    async fn duplicate_envelope_is_reported_and_not_written_twice() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let e = env("e-dup", "ble:aa", "temperature_c");
        let a1 = collector.submit(e.clone()).await.unwrap();
        let a2 = collector.submit(e).await.unwrap();
        assert!(matches!(a1.status, AckStatus::Accepted { .. }));
        assert!(matches!(a2.status, AckStatus::Duplicate));
        let n: i64 = db
            .with_conn_sync(|conn| {
                Ok(conn
                    .query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0))
                    .unwrap())
            })
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn unknown_subject_goes_to_sighting_staging_with_staged_disposition() {
        let db = test_db();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let ack = collector
            .submit(env("e-2", "ble:unknown", "temperature_c"))
            .await
            .unwrap();
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Staged, .. })));
        let (sightings, staged): (i64, i64) = db
            .with_conn_sync(|conn| {
                Ok((
                    conn.query_row("SELECT COUNT(*) FROM sightings", [], |r| r.get(0))
                        .unwrap(),
                    conn.query_row("SELECT COUNT(*) FROM staged_readings", [], |r| r.get(0))
                        .unwrap(),
                ))
            })
            .unwrap();
        assert_eq!((sightings, staged), (1, 1));
    }

    #[tokio::test]
    async fn malformed_measurement_key_rejects_item_but_stores_valid_sibling() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let mut e = env("e-3", "ble:aa", "temperature_c");
        let mut bad = e.envelope.items[0].clone();
        bad.measurement_key = "Bad:Key".into();
        e.envelope.items.push(bad);
        let ack = collector.submit(e).await.unwrap();
        let AckStatus::Accepted { items } = ack.status else {
            panic!("expected Accepted")
        };
        assert!(matches!(items[0], ItemStatus::Stored { .. }));
        assert!(matches!(
            items[1],
            ItemStatus::ItemRejected {
                reason_code: ReasonCode::MalformedMeasurementKey,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn missing_subject_hint_is_rejected() {
        let db = test_db();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let mut e = env("e-4", "ble:aa", "temperature_c");
        e.envelope.items[0].subject_hint = None; // ブリッジは多subject送信者なのでhint必須(D5決定1)
        let ack = collector.submit(e).await.unwrap();
        let AckStatus::Accepted { items } = ack.status else {
            panic!("expected Accepted")
        };
        assert!(matches!(
            items[0],
            ItemStatus::ItemRejected {
                reason_code: ReasonCode::UnknownSubject,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn storage_failure_produces_no_ack() {
        // ストレージ起因の失敗(コミット不能)はRejected終端ではなくack自体を返さない(D1)。
        // query_only=ON で以降の書き込みを強制失敗させ、submit()がSubmitError::NoAck(=ackなし、
        // ack_txドロップ)を返すことを確認する。
        let db = test_db();
        register_active(&db, "ble:aa");
        db.with_conn_sync(|conn| {
            conn.execute_batch("PRAGMA query_only = ON;")?;
            Ok(())
        })
        .unwrap();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let result = collector
            .submit(env("e-6", "ble:aa", "temperature_c"))
            .await;
        assert!(matches!(result, Err(SubmitError::NoAck)));
    }

    #[tokio::test]
    async fn opportunistic_purge_fires_through_actor_and_respects_ttl() {
        // purge_dedup_before自体は別途ユニットテスト済み。ここではアクター経由で発火することを
        // 検証する: purge_interval_ms=0を注入し、処理成功のたびに必ずパージ判定が真になるように
        // する(本番はDEFAULT_PURGE_INTERVAL_MS=1h)。
        let db = test_db();
        register_active(&db, "ble:aa");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let old_at = now - 100 * 60 * 60 * 1000; // 100h前 > 72h TTL → パージ対象
        let keep_at = now - 60 * 60 * 1000; // 1h前 < 72h TTL → 残る
        db.with_conn_sync(|conn| {
            conn.execute(
                "INSERT INTO ingest_dedup (sender_id, envelope_id, received_at) VALUES (?1, ?2, ?3)",
                rusqlite::params!["old-sender", "old-env", old_at],
            ).unwrap();
            conn.execute(
                "INSERT INTO ingest_dedup (sender_id, envelope_id, received_at) VALUES (?1, ?2, ?3)",
                rusqlite::params!["keep-sender", "keep-env", keep_at],
            ).unwrap();
            Ok(())
        }).unwrap();

        let (collector, _h) =
            Collector::spawn_with_purge_interval(db.clone(), Arc::new(PermissiveRegistry), 16, 0);
        // 1件目: 処理成功→パージ判定発火(非同期に開始)。2件目のackが返る頃には、アクターは
        // 単一タスクで逐次処理するため1件目の(purge awaitを含む)イテレーションは完了している。
        collector
            .submit(env("e-purge-1", "ble:aa", "temperature_c"))
            .await
            .unwrap();
        collector
            .submit(env("e-purge-2", "ble:aa", "humidity_pct"))
            .await
            .unwrap();

        let (old_count, keep_count): (i64, i64) = db
            .with_conn_sync(|conn| {
                Ok((
                    conn.query_row(
                        "SELECT COUNT(*) FROM ingest_dedup WHERE sender_id = 'old-sender'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap(),
                    conn.query_row(
                        "SELECT COUNT(*) FROM ingest_dedup WHERE sender_id = 'keep-sender'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap(),
                ))
            })
            .unwrap();
        assert_eq!(old_count, 0, "row older than 72h TTL must be purged");
        assert_eq!(keep_count, 1, "row within 72h TTL must be kept");
    }

    #[tokio::test]
    async fn cache_is_reset_after_storage_failure() {
        // 回帰テスト: process_itemはensure_seriesのINSERT直後(コミット前)にcache.seriesへ
        // 書き込む。同一envelope内の後続itemがストレージ失敗すると全体がロールバックされ、
        // series行はDBから消えるが、修正前はcacheに幻のseries_idが残っていた。次のenvelopeが
        // 同キーを使うとFK違反→ackなしの無限ループ(再送しても回復しない=D1違反)になる。
        //
        // f64::NANはserde_json経由だとnullになってしまうため、ReadingItemを直接構築して
        // serdeを迂回し、insert_reading_v3の非有限値チェックで2番目のitemを確実に失敗させる。
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);

        let make_item = |value: f64| ReadingItem {
            subject_hint: Some("ble:aa".into()),
            measurement_key: "temp_a".into(),
            channel_index: None,
            series_variant: None,
            values: vec![value],
            device_time_ms: None,
            time_source: TimeSource::Gateway,
            age_ms: None,
            rssi: None,
            battery_pct: None,
        };

        // 1件目: 新規series(=temp_a)を作成しキャッシュに載せる。2件目: NaNで書き込み失敗。
        // envelope全体がロールバックされる = series行は消えるがキャッシュには残る(修正前)。
        let poison = IngestRequest {
            principal: IngestPrincipal::trusted_official_adapter(
                "principal:test-adapter",
                "test-adapter",
            ),
            envelope: Envelope {
                envelope_id: "e-poison".into(),
                source: "test-adapter".into(),
                declaration_version: None,
                items: vec![make_item(1.0), make_item(f64::NAN)],
            },
        };
        let result = collector.submit(poison).await;
        assert!(
            matches!(result, Err(SubmitError::NoAck)),
            "storage failure must not produce an ack"
        );

        // series行は存在しないはず(ロールバック済み)
        let series_count: i64 = db
            .with_conn_sync(|conn| {
                Ok(conn
                    .query_row("SELECT COUNT(*) FROM series", [], |r| r.get(0))
                    .unwrap())
            })
            .unwrap();
        assert_eq!(series_count, 0, "series insert must have been rolled back");

        // 再送(同キー、正常値)。修正前はキャッシュの幻series_idでFK違反 → ackなしのまま。
        // 修正後はキャッシュがリセットされているのでensure_seriesが再実行され、Acceptedになる。
        let retry = IngestRequest {
            principal: IngestPrincipal::trusted_official_adapter(
                "principal:test-adapter",
                "test-adapter",
            ),
            envelope: Envelope {
                envelope_id: "e-retry".into(),
                source: "test-adapter".into(),
                declaration_version: None,
                items: vec![make_item(2.0)],
            },
        };
        let ack = collector
            .submit(retry)
            .await
            .expect("retry must be accepted after cache reset");
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Durable, .. })));

        let n: i64 = db
            .with_conn_sync(|conn| {
                Ok(conn
                    .query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0))
                    .unwrap())
            })
            .unwrap();
        assert_eq!(n, 1);
    }

    fn device_request(
        principal_id: &str,
        credential_id: &str,
        source: &str,
        subjects: impl IntoIterator<Item = ledger::SystemId>,
        auth_epoch: &str,
        envelope: Envelope,
    ) -> IngestRequest {
        IngestRequest {
            principal: IngestPrincipal::authenticated_device(
                principal_id,
                credential_id,
                source,
                subjects,
                "default",
                auth_epoch,
            ),
            envelope,
        }
    }

    #[tokio::test]
    async fn forged_source_is_envelope_rejected_and_intrusion_hook_is_bounded() {
        let db = test_db();
        let subject = register_active_id(&db, "ble:aa");
        let (signal_tx, mut signal_rx) = mpsc::channel(1);
        let (collector, _h) = Collector::spawn_with_components(
            db.clone(),
            Arc::new(PermissiveRegistry),
            16,
            DEFAULT_PURGE_INTERVAL_MS,
            Arc::new(UntrustedSystemClock),
            FreshnessLimits::default(),
            Some(signal_tx),
        );
        let mut envelope = env("forged-1", "ble:aa", "temperature_c").envelope;
        envelope.source = "attacker-controlled-source".into();
        let request = device_request(
            "principal-a",
            "credential-a",
            "configured-device-a",
            [subject],
            "epoch-1",
            envelope,
        );
        let ack = collector.submit(request.clone()).await.unwrap();
        assert!(matches!(
            ack.status,
            AckStatus::Rejected {
                reason_code: ReasonCode::SubjectScopeViolation,
                field_path: Some(ref path),
                ..
            } if path == "/source"
        ));
        // Leave the capacity-one hook full: another mismatch still completes and
        // cannot grow or block on hostile source cardinality.
        let _ = collector.submit(request).await.unwrap();
        assert_eq!(
            signal_rx.recv().await.unwrap(),
            IntrusionSignal {
                principal_id: "principal-a".into(),
                credential_id: Some("credential-a".into()),
                kind: IntrusionKind::SourceMismatch,
            }
        );
        assert!(
            signal_rx.try_recv().is_err(),
            "saturated hook drops excess signals"
        );
        let (readings, dedup): (i64, i64) = db
            .with_conn_sync(|conn| {
                Ok((
                    conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?,
                    conn.query_row("SELECT COUNT(*) FROM ingest_dedup", [], |row| row.get(0))?,
                ))
            })
            .unwrap();
        assert_eq!((readings, dedup), (0, 0));
    }

    #[tokio::test]
    async fn dedup_uses_stable_principal_and_ignores_auth_epoch() {
        let db = test_db();
        let subject = register_active_id(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let envelope = env("stable-envelope", "ble:aa", "temperature_c").envelope;
        let first = device_request(
            "principal-a",
            "credential-old",
            "test-adapter",
            [subject],
            "auth-epoch-1",
            envelope.clone(),
        );
        let reissued = device_request(
            "principal-a",
            "credential-new",
            "test-adapter",
            [subject],
            "auth-epoch-2",
            envelope,
        );
        assert!(matches!(
            collector.submit(first).await.unwrap().status,
            AckStatus::Accepted { .. }
        ));
        assert!(matches!(
            collector.submit(reissued).await.unwrap().status,
            AckStatus::Duplicate
        ));
        let keys: Vec<(String, String)> = db
            .with_conn_sync(|conn| {
                Ok(conn
                    .prepare("SELECT sender_id, envelope_id FROM ingest_dedup")?
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<_, _>>()?)
            })
            .unwrap();
        assert_eq!(keys, vec![("principal-a".into(), "stable-envelope".into())]);
    }

    #[tokio::test]
    async fn subject_scope_rules_are_positional_and_valid_siblings_commit() {
        let db = test_db();
        let allowed = register_active_id(&db, "ble:allowed");
        let other = register_active_id(&db, "ble:other");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);

        let mut envelope = env("scope-mixed", "ble:allowed", "temperature_c").envelope;
        let mut cross_scope = envelope.items[0].clone();
        cross_scope.subject_hint = Some("ble:other".into());
        let mut unknown = envelope.items[0].clone();
        unknown.subject_hint = Some("ble:unknown-http".into());
        envelope.items.extend([cross_scope, unknown]);
        let request = device_request(
            "principal-http",
            "credential-http",
            "test-adapter",
            [allowed],
            "auth-epoch",
            envelope,
        );
        let AckStatus::Accepted { items } = collector.submit(request).await.unwrap().status else {
            panic!("mixed deterministic item outcomes must remain an accepted envelope")
        };
        assert!(matches!(
            items[0],
            ItemStatus::Stored {
                disposition: Disposition::Durable,
                ..
            }
        ));
        assert!(matches!(
            items[1],
            ItemStatus::ItemRejected {
                reason_code: ReasonCode::SubjectScopeViolation,
                ..
            }
        ));
        assert!(matches!(
            items[2],
            ItemStatus::ItemRejected {
                reason_code: ReasonCode::UnknownSubject,
                ..
            }
        ));
        let readings: i64 = db
            .with_conn_sync(|conn| {
                Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?)
            })
            .unwrap();
        assert_eq!(readings, 1);
        assert_ne!(allowed, other);
    }

    #[tokio::test]
    async fn one_subject_omission_resolves_but_multi_subject_omission_rejects() {
        let db = test_db();
        let one = register_active_id(&db, "ble:one");
        let two = register_active_id(&db, "ble:two");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);

        let mut one_envelope = env("one-omitted", "ble:one", "temperature_c").envelope;
        one_envelope.items[0].subject_hint = None;
        let one_ack = collector
            .submit(device_request(
                "principal-one",
                "c1",
                "test-adapter",
                [one],
                "e1",
                one_envelope,
            ))
            .await
            .unwrap();
        assert!(
            matches!(one_ack.status, AckStatus::Accepted { ref items } if matches!(items[0], ItemStatus::Stored { .. }))
        );

        let mut multi_envelope = env("multi-omitted", "ble:one", "temperature_c").envelope;
        multi_envelope.items[0].subject_hint = None;
        let multi_ack = collector
            .submit(device_request(
                "principal-multi",
                "c2",
                "test-adapter",
                [one, two],
                "e1",
                multi_envelope,
            ))
            .await
            .unwrap();
        assert!(
            matches!(multi_ack.status, AckStatus::Accepted { ref items } if matches!(items[0], ItemStatus::ItemRejected { reason_code: ReasonCode::UnknownSubject, .. }))
        );
    }

    #[tokio::test]
    async fn only_trusted_official_principal_stages_unknown_subject() {
        let db = test_db();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let ack = collector
            .submit(env(
                "official-unknown",
                "ble:official-unknown",
                "temperature_c",
            ))
            .await
            .unwrap();
        assert!(
            matches!(ack.status, AckStatus::Accepted { ref items } if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Staged, .. }))
        );
    }

    #[tokio::test]
    async fn scope_violation_precedes_malformed_measurement_and_emits_intrusion() {
        let db = test_db();
        let allowed = register_active_id(&db, "ble:allowed");
        register_active_id(&db, "ble:outside");
        let (intrusion_tx, mut intrusion_rx) = mpsc::channel(1);
        let (collector, _h) = Collector::spawn_with_components(
            db,
            Arc::new(PermissiveRegistry),
            16,
            DEFAULT_PURGE_INTERVAL_MS,
            Arc::new(UntrustedSystemClock),
            FreshnessLimits::default(),
            Some(intrusion_tx),
        );
        let request = device_request(
            "principal-a",
            "credential-a",
            "test-adapter",
            [allowed],
            "epoch-a",
            env("scope-malformed", "ble:outside", "not valid!").envelope,
        );

        let ack = collector.submit(request).await.unwrap();

        assert!(matches!(ack.status, AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::ItemRejected {
            reason_code: ReasonCode::SubjectScopeViolation,
            ..
        })));
        assert_eq!(
            intrusion_rx.try_recv().unwrap(),
            IntrusionSignal {
                principal_id: "principal-a".into(),
                credential_id: Some("credential-a".into()),
                kind: IntrusionKind::SubjectScopeViolation,
            }
        );
    }

    #[tokio::test]
    async fn trusted_freshness_failures_are_positional_and_untrusted_absolute_time_has_no_ack() {
        let db = test_db();
        let subject = register_active_id(&db, "ble:aa");
        let limits = FreshnessLimits::new(1_000, 100).unwrap();
        let (collector, _h) = Collector::spawn_with_components(
            db.clone(),
            Arc::new(PermissiveRegistry),
            16,
            DEFAULT_PURGE_INTERVAL_MS,
            Arc::new(FixedFreshnessClock(FreshnessSnapshot {
                received_at_ms: 10_000,
                trusted_wall_time_ms: Some(10_000),
            })),
            limits,
            None,
        );
        let mut envelope = env("freshness-mixed", "ble:aa", "temperature_c").envelope;
        envelope.items[0].device_time_ms = Some(10_000);
        envelope.items[0].time_source = TimeSource::DeviceNtp;
        let mut stale_boundary = envelope.items[0].clone();
        stale_boundary.device_time_ms = Some(9_000);
        let mut future_boundary = envelope.items[0].clone();
        future_boundary.device_time_ms = Some(10_100);
        let mut stale = envelope.items[0].clone();
        stale.device_time_ms = Some(8_999);
        let mut future = envelope.items[0].clone();
        future.device_time_ms = Some(10_101);
        envelope
            .items
            .extend([stale_boundary, future_boundary, stale, future]);
        let ack = collector
            .submit(device_request(
                "principal-a",
                "c1",
                "test-adapter",
                [subject],
                "e1",
                envelope,
            ))
            .await
            .unwrap();
        assert!(matches!(ack.status, AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored { .. })
            && matches!(items[1], ItemStatus::Stored { .. })
            && matches!(items[2], ItemStatus::Stored { .. })
            && matches!(items[3], ItemStatus::ItemRejected { reason_code: ReasonCode::StaleTimestamp, .. })
            && matches!(items[4], ItemStatus::ItemRejected { reason_code: ReasonCode::StaleTimestamp, .. })));

        let untrusted_db = test_db();
        let untrusted_subject = register_active_id(&untrusted_db, "ble:aa");
        let (untrusted, _h) = Collector::spawn(untrusted_db, Arc::new(PermissiveRegistry), 16);
        let mut absolute = env("untrusted-absolute", "ble:aa", "temperature_c").envelope;
        absolute.items[0].device_time_ms = Some(10_000);
        absolute.items[0].time_source = TimeSource::DeviceRtc;
        assert!(matches!(
            untrusted
                .submit(device_request(
                    "principal-a",
                    "c1",
                    "test-adapter",
                    [untrusted_subject],
                    "e1",
                    absolute
                ))
                .await,
            Err(SubmitError::ClockUntrusted)
        ));
    }

    #[tokio::test]
    async fn restore_reset_of_dedup_window_accepts_same_principal_and_envelope_again() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let request = env("restore-retry", "ble:aa", "temperature_c");
        assert!(matches!(
            collector.submit(request.clone()).await.unwrap().status,
            AckStatus::Accepted { .. }
        ));
        db.with_conn_sync(|conn| {
            conn.execute("DELETE FROM ingest_dedup", [])?;
            ledger::renew_epoch(conn).unwrap();
            Ok(())
        })
        .unwrap();
        assert!(matches!(
            collector.submit(request).await.unwrap().status,
            AckStatus::Accepted { .. }
        ));
        let readings: i64 = db
            .with_conn_sync(|conn| {
                Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?)
            })
            .unwrap();
        assert_eq!(
            readings, 2,
            "restore resets dedup because readings/outbox are not restored; unchanged retries may be accepted again"
        );
    }

    #[tokio::test]
    async fn oversized_envelope_is_rejected_whole() {
        let db = test_db();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let mut e = env("e-5", "ble:aa", "temperature_c");
        let item = e.envelope.items[0].clone();
        e.envelope.items = std::iter::repeat_with(|| item.clone())
            .take(MAX_ITEMS_PER_ENVELOPE + 1)
            .collect();
        let ack = collector.submit(e).await.unwrap();
        assert!(matches!(
            ack.status,
            AckStatus::Rejected {
                reason_code: ReasonCode::BatchTooLarge,
                ..
            }
        ));
    }
}
