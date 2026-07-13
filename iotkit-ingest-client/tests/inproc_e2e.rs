//! inprocクライアントのE2E: 実コレクタ(SqliteRegistry)相手にD1クライアント義務を検証する。
//! - ack意味論の消費(Accepted/Duplicate=完了、Rejected=終端、NoAck=不変再送)
//! - 有界spool(溢れはdrop-oldest+警告)
//! - Closed(コレクタ死亡)でのタスク退出
use iotkit_core_collector::Collector;
use iotkit_core_ledger as ledger;
use iotkit_core_registry::SqliteRegistry;
use iotkit_ingest_client::{IngestClientEvent, new_envelope, spawn_inproc, spawn_inproc_observed};
use iotkit_ingest_contract::*;
use std::sync::Arc;

fn full_db() -> iotkit_core_storage::DbHandle {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    all.sort_by_key(|m| m.version);
    iotkit_core_storage::init_db_memory(&all).unwrap()
}

fn register_active(db: &iotkit_core_storage::DbHandle, hw: &str) {
    db.with_conn_sync(|conn| {
        ledger::insert_device(
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
        Ok(())
    })
    .unwrap();
}

fn item(hw: &str, key: &str, value: f64) -> ReadingItem {
    ReadingItem {
        subject_hint: Some(hw.into()),
        measurement_key: key.into(),
        channel_index: None,
        series_variant: None,
        values: vec![value],
        device_time_ms: None,
        time_source: TimeSource::Edge,
        age_ms: None,
        rssi: None,
        battery_pct: None,
    }
}

async fn readings_count(db: &iotkit_core_storage::DbHandle) -> i64 {
    db.with_conn_sync(|conn| {
        Ok(conn
            .query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0))
            .unwrap())
    })
    .unwrap()
}

/// クライアントの完了を能動的に待つ(ポーリング。テスト専用)
async fn wait_for_readings(db: &iotkit_core_storage::DbHandle, n: i64) {
    for _ in 0..200 {
        if readings_count(db).await >= n {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {n} readings");
}

async fn wait_for_ingest_event(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<IngestClientEvent>,
    expected: IngestClientEvent,
) {
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for ingest client event")
            .expect("ingest client event channel closed");
        if event == expected {
            return;
        }
    }
}

#[tokio::test]
async fn accepted_envelope_reaches_readings() {
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, issuer, _ch) =
        Collector::spawn_composed(db.clone(), Arc::new(SqliteRegistry), 16);
    let principal = issuer.official_adapter(
        "principal:bravepi-mainboard:/dev/ttyAMA0",
        "bravepi-mainboard:/dev/ttyAMA0",
    );
    let (client, _h) = spawn_inproc(collector, principal, 16, 64);
    let e = new_envelope(
        "bravepi-mainboard:/dev/ttyAMA0",
        vec![item("ble:aa", "temperature_c", 21.5)],
    );
    client.try_submit(e).unwrap();
    wait_for_readings(&db, 1).await;
}

#[tokio::test]
async fn bound_principal_cannot_be_replaced_by_envelope_source() {
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, issuer, _ch) =
        Collector::spawn_composed(db.clone(), Arc::new(SqliteRegistry), 16);
    let principal = issuer.official_adapter("principal:receiver-owned", "receiver-owned");
    let (client, _h) = spawn_inproc(collector, principal, 16, 64);

    client
        .try_submit(new_envelope(
            "forged-sender",
            vec![item("ble:aa", "temperature_c", 20.0)],
        ))
        .unwrap();
    client
        .try_submit(new_envelope(
            "receiver-owned",
            vec![item("ble:aa", "temperature_c", 21.0)],
        ))
        .unwrap();

    wait_for_readings(&db, 1).await;
    assert_eq!(readings_count(&db).await, 1);
}

#[tokio::test]
async fn envelope_id_is_stable_and_duplicate_is_success() {
    // 同一エンベロープを2回投入 → コレクタのdedupがDuplicateを返し、クライアントは成功扱いで前進する
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, issuer, _ch) =
        Collector::spawn_composed(db.clone(), Arc::new(SqliteRegistry), 16);
    let principal = issuer.official_adapter("principal:test-adapter", "test-adapter");
    let (client, _h) = spawn_inproc(collector, principal, 16, 64);
    let e = new_envelope("test-adapter", vec![item("ble:aa", "temperature_c", 21.5)]);
    client.try_submit(e.clone()).unwrap();
    client.try_submit(e).unwrap();
    // 後続が処理されることの証拠に3通目を流す
    let e3 = new_envelope("test-adapter", vec![item("ble:aa", "temperature_c", 22.0)]);
    client.try_submit(e3).unwrap();
    wait_for_readings(&db, 2).await; // 1通目+3通目のみ書かれる(2通目はDuplicate)
    assert_eq!(readings_count(&db).await, 2);
}

#[tokio::test]
async fn noack_is_retried_with_same_envelope_until_recovery() {
    // トリガーでregistry_entriesへのINSERT(auto-enable)を失敗させNoAckを誘発 →
    // クライアントはエンベロープ不変で再送し続け、トリガー除去後に自然回復する(D1)。
    let db = full_db();
    register_active(&db, "ble:aa");
    db.with_conn_sync(|conn| {
        conn.execute_batch(
            "CREATE TRIGGER fail_enable BEFORE INSERT ON registry_entries
             BEGIN SELECT RAISE(ABORT, 'simulated'); END;",
        )?;
        Ok(())
    })
    .unwrap();
    let (collector, issuer, _ch) =
        Collector::spawn_composed(db.clone(), Arc::new(SqliteRegistry), 16);
    let (obs_tx, mut obs_rx) = tokio::sync::mpsc::unbounded_channel();
    let principal = issuer.official_adapter("principal:test-adapter", "test-adapter");
    let (client, _h) = spawn_inproc_observed(collector, principal, 16, 64, obs_tx);
    let e = new_envelope("test-adapter", vec![item("ble:aa", "temperature_c", 21.5)]);
    client.try_submit(e).unwrap();
    wait_for_ingest_event(&mut obs_rx, IngestClientEvent::SubmitNoAck).await;
    assert_eq!(readings_count(&db).await, 0, "障害中は未耐久のまま");
    db.with_conn_sync(|conn| {
        conn.execute_batch("DROP TRIGGER fail_enable;")?;
        Ok(())
    })
    .unwrap();
    wait_for_readings(&db, 1).await; // 再送で回復。entryも監査イベントも1つずつ
    let (entries, events): (i64, i64) = db
        .with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM registry_entries", [], |r| r.get(0))
                    .unwrap(),
                conn.query_row(
                    "SELECT COUNT(*) FROM ledger_events WHERE kind='registry_entry_enabled'",
                    [],
                    |r| r.get(0),
                )
                .unwrap(),
            ))
        })
        .unwrap();
    assert_eq!((entries, events), (1, 1));
}

#[tokio::test]
async fn terminal_rejection_is_not_retried() {
    // 文法違反キー=エンベロープ内item拒否(終端)。クライアントは再送せず前進する
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, issuer, _ch) =
        Collector::spawn_composed(db.clone(), Arc::new(SqliteRegistry), 16);
    let principal = issuer.official_adapter("principal:test-adapter", "test-adapter");
    let (client, _h) = spawn_inproc(collector, principal, 16, 64);
    client
        .try_submit(new_envelope(
            "test-adapter",
            vec![item("ble:aa", "Bad:Key", 1.0)],
        ))
        .unwrap();
    client
        .try_submit(new_envelope(
            "test-adapter",
            vec![item("ble:aa", "temperature_c", 21.5)],
        ))
        .unwrap();
    wait_for_readings(&db, 1).await; // 2通目が届く=1通目で停滞していない
    assert_eq!(readings_count(&db).await, 1);
}

#[tokio::test]
async fn spool_overflow_drops_oldest_and_keeps_newest() {
    // コレクタを恒久障害にし、全量投入→解除の決定的手順で有界性とdrop-oldestを検証する。
    // バックオフ待機中も入力排出が継続する設計なので、12件全てがspool(cap=4)へ流れ込み
    // 古い側から溢れる——生き残るのは新しい側のみ。
    let db = full_db();
    register_active(&db, "ble:aa");
    db.with_conn_sync(|conn| {
        conn.execute_batch(
            "CREATE TRIGGER fail_all BEFORE INSERT ON ingest_dedup
             BEGIN SELECT RAISE(ABORT, 'down'); END;",
        )?;
        Ok(())
    })
    .unwrap();
    let (collector, issuer, _ch) =
        Collector::spawn_composed(db.clone(), Arc::new(SqliteRegistry), 16);
    let (obs_tx, mut obs_rx) = tokio::sync::mpsc::unbounded_channel();
    let principal = issuer.official_adapter("principal:test-adapter", "test-adapter");
    let (client, _h) = spawn_inproc_observed(collector, principal, 64, 4, obs_tx); // queue_cap=64: 入力側では落ちない
    for i in 0..12 {
        let e = new_envelope(
            "test-adapter",
            vec![item("ble:aa", "temperature_c", 20.0 + i as f64)],
        );
        client.try_submit(e).unwrap();
    }
    // 全量がspoolへ排出されdrop-oldestが起きるまで待つ(障害中は未耐久のまま)
    for _ in 0..8 {
        wait_for_ingest_event(&mut obs_rx, IngestClientEvent::SpoolOverflow).await;
    }
    assert_eq!(readings_count(&db).await, 0, "障害中は未耐久");
    db.with_conn_sync(|conn| {
        conn.execute_batch("DROP TRIGGER fail_all;")?;
        Ok(())
    })
    .unwrap();
    wait_for_readings(&db, 1).await;
    wait_for_readings(&db, 4).await;
    let n = readings_count(&db).await;
    assert!((1..=5).contains(&n), "有界spool(cap=4+送信中1): n={n}");
    // drop-oldestの証拠: 最新エンベロープ(値31.0)が生き残る
    let max: f64 = db
        .with_conn_sync(|conn| {
            Ok(conn
                .query_row(
                    "SELECT MAX(CAST(json_extract(values_json, '$[0]') AS REAL)) FROM readings",
                    [],
                    |r| r.get(0),
                )
                .unwrap())
        })
        .unwrap();
    assert_eq!(max, 31.0, "最新エンベロープはドロップされない(drop-oldest)");
}

#[tokio::test]
async fn collector_death_exits_client_task() {
    // コレクタのJoinHandleをabort → submitがClosed → クライアントタスクが退出する
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, issuer, collector_handle) =
        Collector::spawn_composed(db.clone(), Arc::new(SqliteRegistry), 16);
    let principal = issuer.official_adapter("principal:test-adapter", "test-adapter");
    let (client, client_handle) = spawn_inproc(collector, principal, 16, 64);
    collector_handle.abort();
    let _ = client.try_submit(new_envelope(
        "test-adapter",
        vec![item("ble:aa", "temperature_c", 21.5)],
    ));
    // クライアントタスクはClosed検知で終了する(ゲートウェイのfail-fast検知点)
    tokio::time::timeout(std::time::Duration::from_secs(5), client_handle)
        .await
        .expect("client task must exit after collector death")
        .expect("client task must not panic");
}

#[test]
fn new_envelope_assigns_unique_ids_and_source() {
    let e1 = iotkit_ingest_client::new_envelope("s", vec![]);
    let e2 = iotkit_ingest_client::new_envelope("s", vec![]);
    assert_ne!(e1.envelope_id, e2.envelope_id);
    assert_eq!(e1.source, "s");
    assert_eq!(e1.declaration_version, None);
}
