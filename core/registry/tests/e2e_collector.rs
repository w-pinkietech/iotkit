//! D6判別表のE2E: Envelope → Collector(SqliteRegistry) → readings/series/registry_entries。
//! ackの各語彙とDB状態の対応を、コレクタ実物のトランザクション境界越しに検証する。
use iotkit_core_collector::Collector;
use iotkit_core_ledger as ledger;
use iotkit_core_registry::SqliteRegistry;
use iotkit_ingest_contract::*;
use std::sync::Arc;

fn full_db() -> iotkit_core_storage::DbHandle {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(ledger::MIGRATIONS); // 3, 5
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS); // 2, 4
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS); // 6
    all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    all.sort_by_key(|m| m.version); // 1..=6
    iotkit_core_storage::init_db_memory(&all).unwrap()
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

fn env_with(id: &str, hw: &str, key: &str, channel: Option<u16>, values: Vec<f64>) -> Envelope {
    Envelope {
        envelope_id: id.into(),
        source: "bravepi-mainboard:/dev/ttyAMA0".into(), // 実在ID形式(handle.rs:109)
        declaration_version: None,
        items: vec![ReadingItem {
            subject_hint: Some(hw.into()),
            measurement_key: key.into(),
            channel_index: channel,
            series_variant: None,
            values,
            device_time_ms: None,
            time_source: TimeSource::Gateway,
            age_ms: None, rssi: None, battery_pct: None,
        }],
    }
}

#[tokio::test]
async fn known_key_in_range_is_durable_and_auto_enables() {
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let ack = collector
        .submit(env_with("e-1", "ble:aa", "temperature_c", None, vec![21.5]))
        .await.unwrap();
    assert!(matches!(ack.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored {
            disposition: Disposition::Durable, quarantine_reason: None })));
    let (entries, events, readings): (i64, i64, i64) = db.with_conn_sync(|conn| {
        Ok((
            conn.query_row("SELECT COUNT(*) FROM registry_entries WHERE measurement_key='temperature_c'", [], |r| r.get(0)).unwrap(),
            conn.query_row("SELECT COUNT(*) FROM ledger_events WHERE kind='registry_entry_enabled'", [], |r| r.get(0)).unwrap(),
            conn.query_row("SELECT COUNT(*) FROM readings WHERE quarantined=0", [], |r| r.get(0)).unwrap(),
        ))
    }).unwrap();
    assert_eq!((entries, events, readings), (1, 1, 1),
        "auto-enable+監査イベント+durable行が同一トランザクションで揃う");
}

#[tokio::test]
async fn out_of_range_is_quarantined_row_with_clean_series() {
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let ack = collector
        .submit(env_with("e-2", "ble:aa", "temperature_c", None, vec![5000.0]))
        .await.unwrap();
    assert!(matches!(ack.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored {
            disposition: Disposition::Quarantined,
            quarantine_reason: Some(QuarantineReason::OutOfRange),
        })), "検疫理由がワイヤで可視化される(D1追補)");
    let (s_q, r_q): (i64, i64) = db.with_conn_sync(|conn| {
        Ok((
            conn.query_row("SELECT quarantined FROM series", [], |r| r.get(0)).unwrap(),
            conn.query_row("SELECT quarantined FROM readings", [], |r| r.get(0)).unwrap(),
        ))
    }).unwrap();
    assert_eq!((s_q, r_q), (0, 1));
}

#[tokio::test]
async fn unknown_key_materializes_quarantined_series_with_reason() {
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let ack = collector
        .submit(env_with("e-3", "ble:aa", "custom.tank_level", None, vec![42.0]))
        .await.unwrap();
    assert!(matches!(ack.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored {
            disposition: Disposition::Quarantined,
            quarantine_reason: Some(QuarantineReason::UnknownKey),
        })));
    let (key, q, reason): (String, i64, Option<String>) = db.with_conn_sync(|conn| {
        Ok(conn.query_row(
            "SELECT measurement_key, quarantined, quarantine_reason FROM series",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap())
    }).unwrap();
    assert_eq!(key, "custom.tank_level");
    assert_eq!(q, 1);
    assert_eq!(reason.as_deref(), Some("unknown_key"));
}

#[tokio::test]
async fn value_type_mismatch_rejects_item_but_stores_valid_sibling() {
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let mut e = env_with("e-4", "ble:aa", "temperature_c", None, vec![21.5]);
    e.items.push(ReadingItem {
        subject_hint: Some("ble:aa".into()),
        measurement_key: "contact_state".into(),
        channel_index: None,
        series_variant: None,
        values: vec![3.0], // boolに3.0 → 構造的に解釈不能
        device_time_ms: None,
        time_source: TimeSource::Gateway,
        age_ms: None, rssi: None, battery_pct: None,
    });
    let ack = collector.submit(e).await.unwrap();
    let AckStatus::Accepted { items } = ack.status else { panic!("expected Accepted") };
    assert!(matches!(items[0], ItemStatus::Stored { disposition: Disposition::Durable, .. }));
    assert!(matches!(items[1],
        ItemStatus::ItemRejected { reason_code: ReasonCode::ValueTypeMismatch, .. }));
}

#[tokio::test]
async fn undeclared_acceleration_channel_is_quarantined() {
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let ok = collector
        .submit(env_with("e-5", "ble:aa", "acceleration_mg", Some(2), vec![100.0]))
        .await.unwrap();
    assert!(matches!(ok.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Durable, .. })));
    let bad = collector
        .submit(env_with("e-6", "ble:aa", "acceleration_mg", Some(3), vec![100.0]))
        .await.unwrap();
    assert!(matches!(bad.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored {
            disposition: Disposition::Quarantined,
            quarantine_reason: Some(QuarantineReason::UndeclaredChannel),
        })));
}

#[tokio::test]
async fn single_mode_none_and_zero_channel_share_one_series() {
    // 正準化(評価器のchannel_index)により None / Some(0) が同一seriesへ落ちる
    let db = full_db();
    register_active(&db, "ble:aa");
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    collector.submit(env_with("e-c1", "ble:aa", "distance_mm", None, vec![100.0])).await.unwrap();
    collector.submit(env_with("e-c2", "ble:aa", "distance_mm", Some(0), vec![200.0])).await.unwrap();
    let (series, channel, readings): (i64, i32, i64) = db.with_conn_sync(|conn| {
        Ok((
            conn.query_row("SELECT COUNT(*) FROM series", [], |r| r.get(0)).unwrap(),
            conn.query_row("SELECT channel_index FROM series", [], |r| r.get(0)).unwrap(),
            conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap(),
        ))
    }).unwrap();
    assert_eq!((series, channel, readings), (1, -1, 2), "series分裂しない(正準チャネル=-1)");
}

#[tokio::test]
async fn alias_routes_new_series_to_canonical_key() {
    let db = full_db();
    register_active(&db, "ble:aa");
    db.with_conn_sync(|conn| {
        let cat = iotkit_core_registry::standard_catalog();
        iotkit_core_registry::enable_entry(
            conn, cat.find("temperature_c").unwrap(), &cat.catalog_version, "test",
        ).unwrap();
        iotkit_core_registry::define_alias(
            conn, "temp_old", "temperature_c", iotkit_core_registry::AliasKind::SiteMapping,
        ).unwrap();
        Ok(())
    }).unwrap();
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    collector
        .submit(env_with("e-7", "ble:aa", "temp_old", None, vec![21.5]))
        .await.unwrap();
    let key: String = db.with_conn_sync(|conn| {
        Ok(conn.query_row("SELECT measurement_key FROM series", [], |r| r.get(0)).unwrap())
    }).unwrap();
    assert_eq!(key, "temperature_c");
}

#[tokio::test]
async fn auto_enable_failure_produces_no_ack_and_retry_recovers_consistently() {
    // auto-enable(registry_entriesへのINSERT)**だけ**をトリガーで失敗させる(query_only方式だと
    // 手前のdedup INSERTで落ちてauto-enable経路を通らない)。ストレージ失敗 → ackなし(D1)。
    // エンベロープ全体がロールバックされるため、dedup予約・entry・監査イベントは何も残らず、
    // トリガー除去後の**同一envelope_id再送**は重複扱いにならず受理され、entryと監査イベントが
    // ちょうど1つずつになる(計画1のキャッシュ全捨て教訓のレジストリ版整合検証)。
    let db = full_db();
    register_active(&db, "ble:aa");
    db.with_conn_sync(|conn| {
        conn.execute_batch(
            "CREATE TRIGGER fail_enable BEFORE INSERT ON registry_entries
             BEGIN SELECT RAISE(ABORT, 'simulated registry failure'); END;",
        )?;
        Ok(())
    }).unwrap();
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(SqliteRegistry), 16);
    let e = env_with("e-8", "ble:aa", "temperature_c", None, vec![21.5]);
    let result = collector.submit(e.clone()).await;
    assert!(matches!(result, Err(iotkit_core_collector::SubmitError::NoAck)),
        "auto-enable失敗はRejectedではなくackなし(D1)");
    let (dedup, entries, events, readings): (i64, i64, i64, i64) = db.with_conn_sync(|conn| {
        conn.execute_batch("DROP TRIGGER fail_enable;")?;
        Ok((
            conn.query_row("SELECT COUNT(*) FROM ingest_dedup", [], |r| r.get(0)).unwrap(),
            conn.query_row("SELECT COUNT(*) FROM registry_entries", [], |r| r.get(0)).unwrap(),
            conn.query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind='registry_entry_enabled'",
                [], |r| r.get(0)).unwrap(),
            conn.query_row("SELECT COUNT(*) FROM readings", [], |r| r.get(0)).unwrap(),
        ))
    }).unwrap();
    assert_eq!((dedup, entries, events, readings), (0, 0, 0, 0), "エンベロープ全体ロールバック");
    // 同一コレクタ(キャッシュ全捨て済み)への再送 → 受理・整合
    let ack = collector.submit(e).await.expect("retry must be accepted");
    assert!(matches!(ack.status,
        AckStatus::Accepted { ref items }
        if matches!(items[0], ItemStatus::Stored { disposition: Disposition::Durable, .. })));
    let (entries2, events2): (i64, i64) = db.with_conn_sync(|conn| {
        Ok((
            conn.query_row("SELECT COUNT(*) FROM registry_entries", [], |r| r.get(0)).unwrap(),
            conn.query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind='registry_entry_enabled'",
                [], |r| r.get(0)).unwrap(),
        ))
    }).unwrap();
    assert_eq!((entries2, events2), (1, 1), "再送後にentryと監査イベントがちょうど1つずつ");
}
