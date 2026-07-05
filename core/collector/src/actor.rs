use crate::registry_policy::{is_series_level, RegistryPolicy, RegistryVerdict};
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

pub struct IngestRequest {
    pub envelope: Envelope,
    pub ack_tx: oneshot::Sender<EnvelopeAck>,
}

#[derive(Clone)]
pub struct Collector {
    tx: mpsc::Sender<IngestRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitError {
    /// ackなし=未耐久(ストレージ失敗等)。コレクタは生存しており、同一envelope_idの再送で
    /// 回復可能(D1)。再送/スプールは送信側(計画3のアダプタ内クライアント)の責務。
    NoAck,
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
    pub fn spawn(
        db: DbHandle,
        policy: Arc<dyn RegistryPolicy>,
        queue_cap: usize,
    ) -> (Collector, tokio::task::JoinHandle<()>) {
        Self::spawn_with_purge_interval(db, policy, queue_cap, DEFAULT_PURGE_INTERVAL_MS)
    }

    /// `spawn`と同じだが、日和見dedupパージの発火間隔を注入できる(テスト用: 0を渡すと
    /// 処理成功のたびに毎回パージ判定が真になり、パージ経路をアクター経由で検証できる)。
    pub fn spawn_with_purge_interval(
        db: DbHandle,
        policy: Arc<dyn RegistryPolicy>,
        queue_cap: usize,
        purge_interval_ms: i64,
    ) -> (Collector, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<IngestRequest>(queue_cap);
        let handle = tokio::spawn(async move {
            let mut cache = ResolutionCache::default();
            let mut last_purge_ms = now_ms();
            while let Some(req) = rx.recv().await {
                let taken = std::mem::take(&mut cache);
                let policy = Arc::clone(&policy);
                let envelope = req.envelope;
                let result = db
                    .with_conn(move |conn| {
                        let mut c = taken;
                        let outcome = process_envelope(conn, &mut c, policy.as_ref(), &envelope);
                        Ok((outcome, c))
                    })
                    .await;
                match result {
                    Ok((Ok(ack), c)) => {
                        cache = c;
                        let _ = req.ack_tx.send(ack);
                        // 計画4のTTL/保持ワイヤリング着地までの日和見パージ(受理トランザクション
                        // の外・別途with_connで実行。ack耐久性には影響しない)。
                        maybe_purge_dedup(&db, purge_interval_ms, &mut last_purge_ms).await;
                    }
                    Ok((Err(e), _c)) => {
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

    pub async fn submit(&self, envelope: Envelope) -> Result<EnvelopeAck, SubmitError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send(IngestRequest { envelope, ack_tx })
            .await
            .map_err(|_| SubmitError::Closed)?;
        // ack_txドロップ(ストレージ失敗)はNoAck。コレクタ死亡による中途ドロップも
        // 保守的にNoAckとする(次のsubmitがClosedを返す)。
        ack_rx.await.map_err(|_| SubmitError::NoAck)
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
                tracing::info!(deleted, cutoff_ms = cutoff, "collector: opportunistic ingest_dedup purge");
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
    envelope: &Envelope,
) -> Result<EnvelopeAck, String> {
    let eid = envelope.envelope_id.clone();
    if envelope.items.len() > MAX_ITEMS_PER_ENVELOPE {
        // 決定的な契約違反(サイズ超過はDBに触れる前に判定できる) → 終端Rejectedを維持
        return Ok(EnvelopeAck {
            envelope_id: eid,
            status: AckStatus::Rejected {
                reason_code: ReasonCode::BatchTooLarge,
                message: format!("items {} > {}", envelope.items.len(), MAX_ITEMS_PER_ENVELOPE),
            },
        });
    }
    let tx = rusqlite::Transaction::new_unchecked(
        conn,
        rusqlite::TransactionBehavior::Immediate,
    )
    .map_err(|e| e.to_string())?;
    let generation = ledger::current_generation(&tx).map_err(|e| e.to_string())?;
    if generation != cache.generation {
        cache.devices.clear();
        cache.series.clear();
        cache.generation = generation;
    }
    let claimed = ts::try_claim_envelope(&tx, &envelope.source, &envelope.envelope_id)
        .map_err(|e| e.to_string())?;
    if !claimed {
        drop(tx); // dedup判定のみ・書き込みなし
        return Ok(EnvelopeAck { envelope_id: eid, status: AckStatus::Duplicate });
    }
    let received_at = now_ms();
    let epoch = ledger::ledger_epoch(&tx).map_err(|e| e.to_string())?;
    let mut item_statuses = Vec::with_capacity(envelope.items.len());
    for item in &envelope.items {
        item_statuses.push(process_item(
            &tx,
            cache,
            policy,
            envelope,
            item,
            received_at,
            &epoch,
        )?);
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(EnvelopeAck { envelope_id: eid, status: AckStatus::Accepted { items: item_statuses } })
}

fn restore_device_time(
    received_at: i64,
    device_time_ms: Option<i64>,
    age_ms: Option<u64>,
    declared: TimeSource,
) -> (Option<i64>, TimeSource) {
    match (device_time_ms, age_ms) {
        (Some(dt), _) => (Some(dt), declared),
        (None, Some(age)) => match i64::try_from(age).ok().and_then(|a| received_at.checked_sub(a))
        {
            Some(dt) => (Some(dt), TimeSource::GatewayAdjusted),
            None => (None, declared),
        },
        (None, None) => (None, declared),
    }
}

fn process_item(
    conn: &rusqlite::Connection,
    cache: &mut ResolutionCache,
    policy: &dyn RegistryPolicy,
    envelope: &Envelope,
    item: &ReadingItem,
    received_at: i64,
    epoch: &str,
) -> Result<ItemStatus, String> {
    // 1) 文法検査(決定的契約違反。レジストリにもDBにも触れず判定できるためprecheck)
    if let Err(e) = validate_measurement_key(&item.measurement_key) {
        return Ok(ItemStatus::ItemRejected {
            reason_code: ReasonCode::MalformedMeasurementKey,
            message: e.to_string(),
        });
    }
    // 2) subject解決(D5決定1: 送信者+subject_hint→台帳)。hint欠如も決定的な契約違反
    let Some(hw) = item.subject_hint.as_deref() else {
        return Ok(ItemStatus::ItemRejected {
            reason_code: ReasonCode::UnknownSubject,
            message: "subject_hint required for multi-subject sender".into(),
        });
    };
    let resolved = match cache.devices.get(hw) {
        Some(hit) => Some(*hit),
        None => match ledger::find_alive_by_hardware_id(conn, hw).map_err(|e| e.to_string())? {
            Some(row) => {
                cache.devices.insert(hw.to_string(), (row.system_id, row.state));
                Some((row.system_id, row.state))
            }
            None => None,
        },
    };
    let Some((system_id, state)) = resolved else {
        // 3) 未知subject → 目撃ステージング(D5決定4経路A、ack=staged)。レジストリ評価はしない
        let payload = serde_json::to_string(item).unwrap_or_else(|_| "{}".into());
        ledger::record_sighting(conn, hw, &envelope.source).map_err(|e| e.to_string())?;
        ts::insert_staged_reading(conn, hw, received_at, &payload).map_err(|e| e.to_string())?;
        return Ok(ItemStatus::Stored {
            disposition: Disposition::Staged,
            quarantine_reason: None, // stagedとquarantinedは直列に成立しない(D1: subject解決が常に先)
        });
    };
    // 4) レジストリ評価(D6判別表)。Errはストレージ失敗=ackなしへ伝播(D1)
    let (resolved_key, channel, registry_quarantine) =
        match policy.evaluate(conn, &system_id, item)? {
            RegistryVerdict::Accept { resolved_key, channel_index, quarantine } => {
                (resolved_key, channel_index, quarantine)
            }
            RegistryVerdict::RejectItem { reason_code, message } => {
                return Ok(ItemStatus::ItemRejected { reason_code, message });
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
    let series_quarantined = registry_quarantine.map_or(false, is_series_level);
    let skey = (system_id, resolved_key.clone(), channel, variant.clone());
    let series_id = match cache.series.get(&skey) {
        Some(id) => *id,
        None => {
            let reason = registry_quarantine
                .filter(|q| is_series_level(*q))
                .map(|q| q.as_str());
            let id = ledger::ensure_series(
                conn, &system_id, &resolved_key, channel, &variant, series_quarantined, reason,
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
    let (device_time_ms, time_source) =
        restore_device_time(received_at, item.device_time_ms, item.age_ms, item.time_source);
    let time_source = match time_source {
        TimeSource::DeviceNtp => "device_ntp", TimeSource::DeviceRtc => "device_rtc",
        TimeSource::Gateway => "gateway", TimeSource::GatewayAdjusted => "gateway_adjusted",
    };
    let new = ts::NewReading {
        series_id,
        received_at_ms: received_at,
        device_time_ms,
        time_source: time_source.to_string(),
        values: item.values.clone(),
        rssi: item.rssi,
        battery_pct: item.battery_pct,
        quarantined: row_quarantined,
    };
    let seq = ts::insert_reading_v3(conn, &new).map_err(|e| e.to_string())?;
    if !row_quarantined {
        iotkit_core_publish::store::enqueue_measurement(conn, epoch, seq, received_at)
            .map_err(|e| e.to_string())?;
    }
    Ok(ItemStatus::Stored {
        disposition: if row_quarantined { Disposition::Quarantined } else { Disposition::Durable },
        quarantine_reason: if row_quarantined { wire_reason } else { None },
    })
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

    fn env(id: &str, hw: &str, key: &str) -> Envelope {
        Envelope {
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
                age_ms: None, rssi: None, battery_pct: None,
            }],
        }
    }

    fn register_active(db: &iotkit_core_storage::DbHandle, hw: &str) {
        db.with_conn_sync(|conn| {
            ledger::insert_device(conn, &ledger::NewDevice {
                hardware_id: hw.into(), user_label: None, parent: None,
                kind: ledger::DeviceKind::Individual,
                initial_state: ledger::DeviceState::Active,
            }).unwrap();
            Ok(())
        }).unwrap();
    }

    fn raw_channel(item: &ReadingItem) -> i32 {
        item.channel_index.map(i32::from).unwrap_or(ledger::CHANNEL_NA)
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
        let ack = collector.submit(env("e-1", "ble:aa", "temperature_c")).await.unwrap();
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Durable, .. })));
        // ack = 耐久点: ackが返った時点で行が存在する(D1)
        let n: i64 = db.with_conn_sync(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap())
        }).unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn unknown_key_quarantine_marks_series_row_and_ack_reason() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(
            db.clone(), Arc::new(QuarantiningStub(QuarantineReason::UnknownKey)), 16);
        let ack = collector.submit(env("e-q1", "ble:aa", "custom.mystery")).await.unwrap();
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored {
                disposition: Disposition::Quarantined,
                quarantine_reason: Some(QuarantineReason::UnknownKey),
            })), "ackに検疫理由が可視化される(D1追補)");
        let (s_q, s_reason, r_q): (i64, Option<String>, i64) = db.with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT quarantined FROM series", [], |r| r.get(0)).unwrap(),
                conn.query_row("SELECT quarantine_reason FROM series", [], |r| r.get(0)).unwrap(),
                conn.query_row("SELECT quarantined FROM readings", [], |r| r.get(0)).unwrap(),
            ))
        }).unwrap();
        assert_eq!(s_q, 1, "unknown keyはseries級検疫");
        assert_eq!(s_reason.as_deref(), Some("unknown_key"));
        assert_eq!(r_q, 1);
    }

    #[tokio::test]
    async fn out_of_range_quarantines_row_but_not_series() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(
            db.clone(), Arc::new(QuarantiningStub(QuarantineReason::OutOfRange)), 16);
        let ack = collector.submit(env("e-q2", "ble:aa", "temperature_c")).await.unwrap();
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored {
                disposition: Disposition::Quarantined,
                quarantine_reason: Some(QuarantineReason::OutOfRange),
            })));
        let (s_q, s_reason, r_q): (i64, Option<String>, i64) = db.with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT quarantined FROM series", [], |r| r.get(0)).unwrap(),
                conn.query_row("SELECT quarantine_reason FROM series", [], |r| r.get(0)).unwrap(),
                conn.query_row("SELECT quarantined FROM readings", [], |r| r.get(0)).unwrap(),
            ))
        }).unwrap();
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

        let ack = db.with_conn_sync(|conn| {
            Ok(process_envelope(
                conn,
                &mut cache,
                &PermissiveRegistry,
                &envelope,
            ).unwrap())
        }).unwrap();
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored {
                disposition: Disposition::Durable,
                quarantine_reason: None,
            })));

        let (reading_count, reading_seq, outbox_count): (i64, i64, i64) = db.with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap(),
                conn.query_row("SELECT seq FROM readings", [], |r| r.get(0)).unwrap(),
                conn.query_row(
                    "SELECT COUNT(*) FROM publication_log WHERE kind = 'measurement'",
                    [],
                    |r| r.get(0),
                ).unwrap(),
            ))
        }).unwrap();
        assert_eq!(reading_count, 1);
        assert_eq!(outbox_count, 1, "non-quarantined readings must be enqueued");

        let (outbox_reading_seq, outbox_epoch, expected_epoch): (i64, String, String) =
            db.with_conn_sync(|conn| {
                let expected_epoch = ledger::ledger_epoch(conn).unwrap();
                let (outbox_reading_seq, outbox_epoch) = conn.query_row(
                    "SELECT reading_seq, epoch FROM publication_log WHERE kind = 'measurement'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                ).unwrap();
                Ok((outbox_reading_seq, outbox_epoch, expected_epoch))
            }).unwrap();
        assert_eq!(outbox_reading_seq, reading_seq);
        assert_eq!(outbox_epoch, expected_epoch);

        let quarantined_db = test_db();
        register_active(&quarantined_db, "ble:qq");
        let mut quarantine_cache = ResolutionCache::default();
        let quarantined = env("e-outbox-q", "ble:qq", "custom.mystery");

        let ack = quarantined_db.with_conn_sync(|conn| {
            Ok(process_envelope(
                conn,
                &mut quarantine_cache,
                &QuarantiningStub(QuarantineReason::UnknownKey),
                &quarantined,
            ).unwrap())
        }).unwrap();
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored {
                disposition: Disposition::Quarantined,
                quarantine_reason: Some(QuarantineReason::UnknownKey),
            })));

        let (quarantined_readings, quarantined_outbox): (i64, i64) =
            quarantined_db.with_conn_sync(|conn| {
                Ok((
                    conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap(),
                    conn.query_row(
                        "SELECT COUNT(*) FROM publication_log WHERE kind = 'measurement'",
                        [],
                        |r| r.get(0),
                    ).unwrap(),
                ))
            }).unwrap();
        assert_eq!(quarantined_readings, 1);
        assert_eq!(quarantined_outbox, 0, "quarantined readings must not be enqueued");
    }

    #[tokio::test]
    async fn device_quarantine_is_visible_as_ack_reason() {
        // 検疫状態デバイス(D5経路A: 承認→検疫→active の途中)のデータは行検疫+理由device_quarantined
        let db = test_db();
        db.with_conn_sync(|conn| {
            ledger::record_sighting(conn, "ble:q", "test-adapter").unwrap();
            ledger::approve_sighting(conn, "ble:q", None, ledger::DeviceKind::Individual).unwrap();
            Ok(())
        }).unwrap();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let ack = collector.submit(env("e-dq", "ble:q", "temperature_c")).await.unwrap();
        let AckStatus::Accepted { items } = ack.status else { panic!("expected Accepted") };
        assert!(matches!(items[0], ItemStatus::Stored {
            disposition: Disposition::Quarantined,
            quarantine_reason: Some(QuarantineReason::DeviceQuarantined),
        }));
    }

    #[tokio::test]
    async fn resolution_cache_invalidated_on_generation_bump() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("iotkit.db");
        let migrations = migration_set();
        let db = iotkit_core_storage::init_db(&db_path, &migrations).unwrap();
        let ctl_db = iotkit_core_storage::init_db(&db_path, &migrations).unwrap();

        let system_id = ctl_db.with_conn_sync(|conn| {
            ledger::record_sighting(conn, "ble:gen", "test-adapter").unwrap();
            let sid = ledger::approve_sighting(
                conn,
                "ble:gen",
                Some("generation test"),
                ledger::DeviceKind::Individual,
            ).unwrap();
            Ok(sid)
        }).unwrap();

        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let first = collector.submit(env("e-gen-1", "ble:gen", "temperature_c")).await.unwrap();
        assert!(matches!(first.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored {
                disposition: Disposition::Quarantined,
                quarantine_reason: Some(QuarantineReason::DeviceQuarantined),
            })));

        ctl_db.with_conn_sync(move |conn| {
            let tx = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            ).unwrap();
            ledger::activate_device(&tx, &system_id).unwrap();
            ledger::bump_generation(&tx).unwrap();
            tx.commit().unwrap();
            Ok(())
        }).unwrap();

        let second = collector.submit(env("e-gen-2", "ble:gen", "temperature_c")).await.unwrap();
        assert!(matches!(second.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored {
                disposition: Disposition::Durable,
                quarantine_reason: None,
            })), "generation bump must clear cached quarantined device state");
    }

    #[tokio::test]
    async fn verdict_resolved_key_and_channel_are_used_for_series() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(RenamingStub), 16);
        collector.submit(env("e-alias", "ble:aa", "temp_old")).await.unwrap();
        let (key, ch): (String, i32) = db.with_conn_sync(|conn| {
            Ok(conn.query_row(
                "SELECT measurement_key, channel_index FROM series", [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ).unwrap())
        }).unwrap();
        assert_eq!(key, "temperature_c", "series実体化はresolved_keyを使う");
        assert_eq!(ch, 7, "コレクタはチャネルを再計算せずverdictのchannel_indexを使う");
    }

    #[tokio::test]
    async fn age_ms_restores_gateway_adjusted_device_time() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let mut e = env("e-age", "ble:aa", "temperature_c");
        e.items[0].age_ms = Some(5000);
        e.items[0].time_source = TimeSource::Gateway;
        e.items[0].device_time_ms = None;
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
        let (device_time, source) =
            restore_device_time(10_000, None, Some(0), TimeSource::Gateway);
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
        let result = collector.submit(env("e-fail", "ble:aa", "temperature_c")).await;
        assert!(matches!(result, Err(SubmitError::NoAck)));
        let n: i64 = db.with_conn_sync(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap())
        }).unwrap();
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
        let n: i64 = db.with_conn_sync(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap())
        }).unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn unknown_subject_goes_to_sighting_staging_with_staged_disposition() {
        let db = test_db();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let ack = collector.submit(env("e-2", "ble:unknown", "temperature_c")).await.unwrap();
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Staged, .. })));
        let (sightings, staged): (i64, i64) = db.with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM sightings", [], |r| r.get(0)).unwrap(),
                conn.query_row("SELECT COUNT(*) FROM staged_readings", [], |r| r.get(0)).unwrap(),
            ))
        }).unwrap();
        assert_eq!((sightings, staged), (1, 1));
    }

    #[tokio::test]
    async fn malformed_measurement_key_rejects_item_but_stores_valid_sibling() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let mut e = env("e-3", "ble:aa", "temperature_c");
        let mut bad = e.items[0].clone();
        bad.measurement_key = "Bad:Key".into();
        e.items.push(bad);
        let ack = collector.submit(e).await.unwrap();
        let AckStatus::Accepted { items } = ack.status else { panic!("expected Accepted") };
        assert!(matches!(items[0], ItemStatus::Stored { .. }));
        assert!(matches!(items[1],
            ItemStatus::ItemRejected { reason_code: ReasonCode::MalformedMeasurementKey, .. }));
    }

    #[tokio::test]
    async fn missing_subject_hint_is_rejected() {
        let db = test_db();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let mut e = env("e-4", "ble:aa", "temperature_c");
        e.items[0].subject_hint = None; // ブリッジは多subject送信者なのでhint必須(D5決定1)
        let ack = collector.submit(e).await.unwrap();
        let AckStatus::Accepted { items } = ack.status else { panic!("expected Accepted") };
        assert!(matches!(items[0],
            ItemStatus::ItemRejected { reason_code: ReasonCode::UnknownSubject, .. }));
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
        }).unwrap();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let result = collector.submit(env("e-6", "ble:aa", "temperature_c")).await;
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
        collector.submit(env("e-purge-1", "ble:aa", "temperature_c")).await.unwrap();
        collector.submit(env("e-purge-2", "ble:aa", "humidity_pct")).await.unwrap();

        let (old_count, keep_count): (i64, i64) = db.with_conn_sync(|conn| {
            Ok((
                conn.query_row(
                    "SELECT COUNT(*) FROM ingest_dedup WHERE sender_id = 'old-sender'", [], |r| r.get(0),
                ).unwrap(),
                conn.query_row(
                    "SELECT COUNT(*) FROM ingest_dedup WHERE sender_id = 'keep-sender'", [], |r| r.get(0),
                ).unwrap(),
            ))
        }).unwrap();
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
        let poison = Envelope {
            envelope_id: "e-poison".into(),
            source: "test-adapter".into(),
            declaration_version: None,
            items: vec![make_item(1.0), make_item(f64::NAN)],
        };
        let result = collector.submit(poison).await;
        assert!(matches!(result, Err(SubmitError::NoAck)), "storage failure must not produce an ack");

        // series行は存在しないはず(ロールバック済み)
        let series_count: i64 = db
            .with_conn_sync(|conn| {
                Ok(conn.query_row("SELECT COUNT(*) FROM series", [], |r| r.get(0)).unwrap())
            })
            .unwrap();
        assert_eq!(series_count, 0, "series insert must have been rolled back");

        // 再送(同キー、正常値)。修正前はキャッシュの幻series_idでFK違反 → ackなしのまま。
        // 修正後はキャッシュがリセットされているのでensure_seriesが再実行され、Acceptedになる。
        let retry = Envelope {
            envelope_id: "e-retry".into(),
            source: "test-adapter".into(),
            declaration_version: None,
            items: vec![make_item(2.0)],
        };
        let ack = collector.submit(retry).await.expect("retry must be accepted after cache reset");
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Durable, .. })));

        let n: i64 = db
            .with_conn_sync(|conn| {
                Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap())
            })
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn oversized_envelope_is_rejected_whole() {
        let db = test_db();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let mut e = env("e-5", "ble:aa", "temperature_c");
        let item = e.items[0].clone();
        e.items = std::iter::repeat_with(|| item.clone()).take(MAX_ITEMS_PER_ENVELOPE + 1).collect();
        let ack = collector.submit(e).await.unwrap();
        assert!(matches!(ack.status,
            AckStatus::Rejected { reason_code: ReasonCode::BatchTooLarge, .. }));
    }
}
