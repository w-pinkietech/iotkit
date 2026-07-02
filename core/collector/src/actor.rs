use crate::registry_policy::{RegistryPolicy, RegistryVerdict};
use iotkit_core_ledger as ledger;
use iotkit_core_storage::DbHandle;
use iotkit_core_timeseries as ts;
use iotkit_ingest_contract::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

pub const MAX_ITEMS_PER_ENVELOPE: usize = 256;

pub struct IngestRequest {
    pub envelope: Envelope,
    pub ack_tx: oneshot::Sender<EnvelopeAck>,
}

#[derive(Clone)]
pub struct Collector {
    tx: mpsc::Sender<IngestRequest>,
}

#[derive(Debug)]
pub struct CollectorClosed;

/// タスク所有キャッシュ(D5: 起動時全ロードはWave 0では行数が小さいため遅延ロードで開始し、
/// ミス時にDBを引く。台帳変異は必ずコレクタ経由なので無効化漏れは構造上起きない)
#[derive(Default)]
struct ResolutionCache {
    devices: HashMap<String, (ledger::SystemId, ledger::DeviceState)>, // hardware_id →
    series: HashMap<(ledger::SystemId, String, i32, String), i64>,
}

impl Collector {
    pub fn spawn(
        db: DbHandle,
        policy: Arc<dyn RegistryPolicy>,
        queue_cap: usize,
    ) -> (Collector, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<IngestRequest>(queue_cap);
        let handle = tokio::spawn(async move {
            let mut cache = ResolutionCache::default();
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
                    }
                    Ok((Err(e), c)) => {
                        tracing::error!(error = %e, "collector: storage failure (envelope aborted)");
                        cache = c; // キャッシュ自体はDB変異と無関係なので保全してよい
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

    pub async fn submit(&self, envelope: Envelope) -> Result<EnvelopeAck, CollectorClosed> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send(IngestRequest { envelope, ack_tx })
            .await
            .map_err(|_| CollectorClosed)?;
        ack_rx.await.map_err(|_| CollectorClosed)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let claimed = ts::try_claim_envelope(&tx, &envelope.source, &envelope.envelope_id)
        .map_err(|e| e.to_string())?;
    if !claimed {
        drop(tx); // dedup判定のみ・書き込みなし
        return Ok(EnvelopeAck { envelope_id: eid, status: AckStatus::Duplicate });
    }
    let received_at = now_ms();
    let mut item_statuses = Vec::with_capacity(envelope.items.len());
    for item in &envelope.items {
        item_statuses.push(process_item(&tx, cache, policy, envelope, item, received_at)?);
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(EnvelopeAck { envelope_id: eid, status: AckStatus::Accepted { items: item_statuses } })
}

fn process_item(
    conn: &rusqlite::Connection,
    cache: &mut ResolutionCache,
    policy: &dyn RegistryPolicy,
    envelope: &Envelope,
    item: &ReadingItem,
    received_at: i64,
) -> Result<ItemStatus, String> {
    // 1) レジストリ検証(文法。計画2で値域・未知キー判定に拡張)。決定的な契約違反 → ItemRejected
    let quarantine = match policy.evaluate(item) {
        RegistryVerdict::Accept { quarantine } => quarantine,
        RegistryVerdict::RejectItem { reason_code, message } => {
            return Ok(ItemStatus::ItemRejected { reason_code, message });
        }
    };
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
        // 3) 未知subject → 目撃ステージング(D5決定4経路A、ack=staged)。ストレージ失敗は上へ伝播
        let payload = serde_json::to_string(item).unwrap_or_else(|_| "{}".into());
        ledger::record_sighting(conn, hw, &envelope.source).map_err(|e| e.to_string())?;
        ts::insert_staged_reading(conn, hw, received_at, &payload).map_err(|e| e.to_string())?;
        return Ok(ItemStatus::Stored { disposition: Disposition::Staged });
    };
    // 4) series解決(検疫デバイスのデータは検疫行として保存=D1オンボーディング)
    let device_quarantined = state == ledger::DeviceState::Quarantined;
    let channel: i32 = item.channel_index.map(i32::from).unwrap_or(-1);
    let variant = item.series_variant.as_deref().unwrap_or("primary").to_string();
    let skey = (system_id, item.measurement_key.clone(), channel, variant.clone());
    let series_id = match cache.series.get(&skey) {
        Some(id) => *id,
        None => {
            let id = ledger::ensure_series(conn, &system_id, &item.measurement_key, channel, &variant, false)
                .map_err(|e| e.to_string())?;
            cache.series.insert(skey, id);
            id
        }
    };
    // 5) 書き込み
    let row_quarantined = quarantine || device_quarantined;
    let time_source = match item.time_source {
        TimeSource::DeviceNtp => "device_ntp", TimeSource::DeviceRtc => "device_rtc",
        TimeSource::Gateway => "gateway", TimeSource::GatewayAdjusted => "gateway_adjusted",
    };
    let new = ts::NewReading {
        series_id,
        received_at_ms: received_at,
        device_time_ms: item.device_time_ms,
        time_source: time_source.to_string(),
        values: item.values.clone(),
        rssi: item.rssi,
        battery_pct: item.battery_pct,
        quarantined: row_quarantined,
    };
    ts::insert_reading_v3(conn, &new).map_err(|e| e.to_string())?;
    Ok(ItemStatus::Stored {
        disposition: if row_quarantined { Disposition::Quarantined } else { Disposition::Durable },
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
        all.sort_by_key(|m| m.version);
        iotkit_core_storage::init_db_memory(&all).unwrap()
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

    #[tokio::test]
    async fn known_subject_is_accepted_durable_and_row_exists_before_ack_returns() {
        let db = test_db();
        register_active(&db, "ble:aa");
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let ack = collector.submit(env("e-1", "ble:aa", "temperature_c")).await.unwrap();
        assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Durable })));
        // ack = 耐久点: ackが返った時点で行が存在する(D1)
        let n: i64 = db.with_conn_sync(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap())
        }).unwrap();
        assert_eq!(n, 1);
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
            if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Staged })));
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
        // query_only=ON で以降の書き込みを強制失敗させ、submit()がCollectorClosed(=ackなし、
        // ack_txドロップ)を返すことを確認する。
        let db = test_db();
        register_active(&db, "ble:aa");
        db.with_conn_sync(|conn| {
            conn.execute_batch("PRAGMA query_only = ON;")?;
            Ok(())
        }).unwrap();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let result = collector.submit(env("e-6", "ble:aa", "temperature_c")).await;
        assert!(matches!(result, Err(CollectorClosed)));
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
