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
            time_source: TimeSource::EdgeNode,
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
async fn device_principal_issuer_is_returned_only_by_receiver_composition() {
    let db = test_db();
    let (collector, _issuer, _handle) =
        Collector::spawn_device_composed(db, Arc::new(PermissiveRegistry), 16);
    let clone = collector.clone();
    drop(clone);
}

#[tokio::test]
async fn edge_composition_receives_distinct_local_and_device_issuers() {
    let db = test_db();
    let (collector, local, device, _handle) =
        Collector::spawn_fully_composed(db, Arc::new(PermissiveRegistry), 16);
    let _local_principal = local.official_adapter("principal:local", "local");
    let _device_issuer = device;
    let clone = collector.clone();
    drop(clone);
}

#[tokio::test]
async fn validation_runs_collector_rules_without_product_or_custody_writes() {
    let db = test_db();
    register_active(&db, "ble:aa");
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
    let mut request = env("validate-1", "ble:aa", "temperature_c");
    request.envelope.items.push(ReadingItem {
        subject_hint: Some("ble:aa".into()),
        measurement_key: "not valid!".into(),
        ..request.envelope.items[0].clone()
    });

    let before: (i64, i64, i64) = db
        .with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM ingest_dedup", [], |row| row.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM staged_readings", [], |row| row.get(0))?,
            ))
        })
        .unwrap();
    let report = collector.validate(request).await.unwrap();
    let after: (i64, i64, i64) = db
        .with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM ingest_dedup", [], |row| row.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM staged_readings", [], |row| row.get(0))?,
            ))
        })
        .unwrap();

    assert_eq!(before, after);
    assert!(!report.valid);
    assert_eq!(report.envelope_id, "validate-1");
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].item_index, Some(1));
    assert_eq!(
        report.issues[0].reason_code,
        ReasonCode::MalformedMeasurementKey
    );
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
fn discovery_only_reading_is_durable_without_a_publication_identity() {
    let db = test_db();
    register_active(&db, "ble:activation-preview");
    db.with_conn_sync(|conn| {
        iotkit_core_publish::activation::install_edge_target(
            conn,
            &iotkit_core_publish::store::TargetRow {
                target_id: "edge".into(),
                endpoint_url: "mqtts://broker.example.test:8883".into(),
                credential_token: String::new(),
                archive_responsible: true,
                schema_version: 1,
                cursor_epoch: None,
                cursor_pub_seq: 0,
            },
            1,
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    let mut cache = ResolutionCache::default();
    let envelope = env(
        "e-activation-preview",
        "ble:activation-preview",
        "temperature_c",
    );

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
    let counts: (i64, i64) = db
        .with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT count(*) FROM readings", [], |row| row.get(0))?,
                conn.query_row("SELECT count(*) FROM publication_log", [], |row| row.get(0))?,
            ))
        })
        .unwrap();
    assert_eq!(counts, (1, 0));
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
        time_source: TimeSource::EdgeNode,
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
async fn age_ms_restores_edge_node_adjusted_device_time() {
    let db = test_db();
    register_active(&db, "ble:aa");
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
    let mut e = env("e-age", "ble:aa", "temperature_c");
    e.envelope.items[0].age_ms = Some(5000);
    e.envelope.items[0].time_source = TimeSource::EdgeNode;
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
    assert_eq!(time_source, "edge_node_adjusted");
    assert_eq!(event_time, received_at - 5000);
    assert_eq!(event_time_source, "edge_node_adjusted");
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
    let (device_time, source) = restore_device_time(
        10_000,
        None,
        Some(i64::MAX as u64 + 1),
        TimeSource::EdgeNode,
    );
    assert_eq!(device_time, None);
    assert_eq!(source, TimeSource::EdgeNode);
}

#[test]
fn restore_device_time_ignores_age_ms_that_would_underflow() {
    let (device_time, source) = restore_device_time(i64::MIN, None, Some(1), TimeSource::EdgeNode);
    assert_eq!(device_time, None);
    assert_eq!(source, TimeSource::EdgeNode);
}

#[test]
fn restore_device_time_age_zero_returns_received_at() {
    let (device_time, source) = restore_device_time(10_000, None, Some(0), TimeSource::EdgeNode);
    assert_eq!(device_time, Some(10_000));
    assert_eq!(source, TimeSource::EdgeNodeAdjusted);
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
async fn non_finite_unknown_subjects_are_terminally_rejected_and_deduplicated() {
    for (suffix, value) in [
        ("nan", f64::NAN),
        ("positive-infinity", f64::INFINITY),
        ("negative-infinity", f64::NEG_INFINITY),
    ] {
        let db = test_db();
        let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
        let mut request = env(
            &format!("non-finite-{suffix}"),
            &format!("unknown-{suffix}"),
            "temperature_c",
        );
        request.envelope.items[0].values = vec![value];

        let ack = collector.submit(request.clone()).await.unwrap();

        assert!(matches!(ack.status,
                AckStatus::Accepted { ref items }
                if matches!(&items[..], [ItemStatus::ItemRejected {
                    reason_code: ReasonCode::ValueTypeMismatch,
                    field_path: Some(path),
                    schema_hint: Some(_),
                    ..
                }] if path == "/items/0/values")));
        let state: (i64, i64, i64) = db
            .with_conn_sync(|conn| {
                Ok((
                    conn.query_row("SELECT COUNT(*) FROM staged_readings", [], |row| row.get(0))?,
                    conn.query_row("SELECT COUNT(*) FROM sightings", [], |row| row.get(0))?,
                    conn.query_row(
                        "SELECT COUNT(*) FROM ingest_dedup WHERE envelope_id=?1",
                        [format!("non-finite-{suffix}")],
                        |row| row.get(0),
                    )?,
                ))
            })
            .unwrap();
        assert_eq!(state, (0, 0, 1));

        let retry = collector.submit(request).await.unwrap();
        assert!(matches!(retry.status, AckStatus::Duplicate));
    }
}

#[tokio::test]
async fn mixed_unknown_subject_envelope_stages_only_finite_sibling() {
    let db = test_db();
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
    let mut request = env(
        "mixed-finite-and-non-finite",
        "unknown-valid",
        "temperature_c",
    );
    let mut invalid = request.envelope.items[0].clone();
    invalid.subject_hint = Some("unknown-invalid".into());
    invalid.values = vec![f64::NAN];
    request.envelope.items.push(invalid);

    let ack = collector.submit(request).await.unwrap();

    assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(&items[..], [
                ItemStatus::Stored { disposition: Disposition::Staged, .. },
                ItemStatus::ItemRejected {
                    reason_code: ReasonCode::ValueTypeMismatch,
                    field_path: Some(path),
                    schema_hint: Some(_),
                    ..
                }
            ] if path == "/items/1/values")));
    let state: (i64, i64, i64, i64, i64) = db
            .with_conn_sync(|conn| {
                Ok((
                    conn.query_row("SELECT COUNT(*) FROM staged_readings", [], |row| row.get(0))?,
                    conn.query_row("SELECT COUNT(*) FROM sightings", [], |row| row.get(0))?,
                    conn.query_row(
                        "SELECT COUNT(*) FROM staged_readings WHERE hardware_id='unknown-valid'",
                        [],
                        |row| row.get(0),
                    )?,
                    conn.query_row(
                        "SELECT COUNT(*) FROM staged_readings WHERE hardware_id='unknown-invalid'",
                        [],
                        |row| row.get(0),
                    )?,
                    conn.query_row(
                        "SELECT COUNT(*) FROM ingest_dedup WHERE envelope_id='mixed-finite-and-non-finite'",
                        [],
                        |row| row.get(0),
                    )?,
                ))
            })
            .unwrap();
    assert_eq!(state, (1, 1, 1, 0, 1));
}

#[tokio::test]
async fn validation_reports_non_finite_unknown_subject_without_state_changes() {
    let db = test_db();
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
    let mut request = env(
        "validate-non-finite-unknown",
        "unknown-validation-valid",
        "temperature_c",
    );
    let mut invalid = request.envelope.items[0].clone();
    invalid.subject_hint = Some("unknown-validation-invalid".into());
    invalid.values = vec![f64::NEG_INFINITY];
    request.envelope.items.push(invalid);
    let state = |db: &iotkit_core_storage::DbHandle| {
        db.with_conn_sync(|conn| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM readings", [], |row| {
                    row.get::<_, i64>(0)
                })?,
                conn.query_row("SELECT COUNT(*) FROM series", [], |row| {
                    row.get::<_, i64>(0)
                })?,
                conn.query_row("SELECT COUNT(*) FROM staged_readings", [], |row| {
                    row.get::<_, i64>(0)
                })?,
                conn.query_row("SELECT COUNT(*) FROM sightings", [], |row| {
                    row.get::<_, i64>(0)
                })?,
                conn.query_row("SELECT COUNT(*) FROM ingest_dedup", [], |row| {
                    row.get::<_, i64>(0)
                })?,
                conn.query_row("SELECT COUNT(*) FROM publication_log", [], |row| {
                    row.get::<_, i64>(0)
                })?,
            ))
        })
        .unwrap()
    };
    let before = state(&db);

    let report = collector.validate(request).await.unwrap();

    assert!(!report.valid);
    assert_eq!(report.envelope_id, "validate-non-finite-unknown");
    assert!(matches!(&report.issues[..], [ValidationIssue {
            item_index: Some(1),
            reason_code: ReasonCode::ValueTypeMismatch,
            field_path: Some(path),
            schema_hint: Some(_),
            ..
        }] if path == "/items/1/values"));
    assert_eq!(state(&db), before);
}

#[test]
fn staged_serialization_failure_propagates_without_a_stored_outcome() {
    let request = env(
        "serialization-failure",
        "unknown-serialization-failure",
        "temperature_c",
    );
    let mut pending_sightings = Vec::new();

    let result = stage_unknown_item_with(
        &request.envelope.items[0],
        "unknown-serialization-failure",
        &mut pending_sightings,
        |_| Err::<String, _>("injected serialization failure"),
    );

    assert_eq!(result, Err("injected serialization failure".into()));
    assert!(pending_sightings.is_empty());
}

#[tokio::test]
async fn finite_guard_precedes_registry_lookup_but_not_measurement_key_grammar() {
    let db = test_db();
    register_active(&db, "ble:finite-precheck");
    let (collector, _h) = Collector::spawn(db, Arc::new(FailingPolicy), 16);
    let mut non_finite = env(
        "finite-before-registry",
        "ble:finite-precheck",
        "custom.unknown",
    );
    non_finite.envelope.items[0].values = vec![f64::INFINITY];

    let ack = collector.submit(non_finite).await.unwrap();

    assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(&items[..], [ItemStatus::ItemRejected {
                reason_code: ReasonCode::ValueTypeMismatch,
                field_path: Some(path),
                ..
            }] if path == "/items/0/values")));

    let mut malformed = env("grammar-before-finite", "ble:finite-precheck", "not valid!");
    malformed.envelope.items[0].values = vec![f64::NAN];
    let ack = collector.submit(malformed).await.unwrap();
    assert!(matches!(ack.status,
            AckStatus::Accepted { ref items }
            if matches!(&items[..], [ItemStatus::ItemRejected {
                reason_code: ReasonCode::MalformedMeasurementKey,
                field_path: Some(path),
                ..
            }] if path == "/items/0/measurement_key")));
}

#[tokio::test]
async fn multi_item_staging_over_reserve_is_no_ack_and_rolls_back_every_sibling() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let mut insert = conn.prepare(
            "INSERT INTO staged_readings
                 (hardware_id, received_at, payload_json, principal_id, payload_bytes, pinned)
                 VALUES (?1, 0, '{}', 'principal:test-adapter', 11186, 1)",
        )?;
        for index in 0..744 {
            insert.execute([format!("pinned-{index}")])?;
        }
        Ok(())
    })
    .unwrap();

    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
    let mut request = env(
        "staging-envelope-over-reserve",
        "incoming-1",
        "temperature_c",
    );
    request.envelope.items[0].series_variant = Some("x".repeat(40 * 1024));
    let mut second = request.envelope.items[0].clone();
    second.subject_hint = Some("incoming-2".into());
    request.envelope.items.push(second);
    let staged_sizes = request
        .envelope
        .items
        .iter()
        .map(|item| serde_json::to_string(item).unwrap().len())
        .collect::<Vec<_>>();
    assert!(staged_sizes.iter().all(|size| *size < 64 * 1024));
    assert!(staged_sizes.iter().sum::<usize>() > 64 * 1024);

    let result = collector.submit(request).await;

    assert!(matches!(result, Err(SubmitError::NoAck)));
    let state: (i64, i64, i64, i64, i64) = db
            .with_conn_sync(|conn| {
                Ok((
                    conn.query_row(
                        "SELECT COUNT(*) FROM staged_readings WHERE pinned=1",
                        [],
                        |row| row.get(0),
                    )?,
                    conn.query_row(
                        "SELECT COALESCE(SUM(payload_bytes),0) FROM staged_readings WHERE pinned=1",
                        [],
                        |row| row.get(0),
                    )?,
                    conn.query_row(
                        "SELECT COUNT(*) FROM staged_readings WHERE hardware_id IN ('incoming-1','incoming-2')",
                        [],
                        |row| row.get(0),
                    )?,
                    conn.query_row(
                        "SELECT COUNT(*) FROM sightings WHERE hardware_id IN ('incoming-1','incoming-2')",
                        [],
                        |row| row.get(0),
                    )?,
                    conn.query_row(
                        "SELECT COUNT(*) FROM ingest_dedup WHERE envelope_id='staging-envelope-over-reserve'",
                        [],
                        |row| row.get(0),
                    )?,
                ))
            })
            .unwrap();
    assert_eq!(state, (744, 8_322_384, 0, 0, 0));
}

#[tokio::test]
async fn multi_item_staging_at_reserve_is_acknowledged_and_fully_durable() {
    let db = test_db();
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
    let mut request = env(
        "staging-envelope-within-reserve",
        "incoming-1",
        "temperature_c",
    );
    request.envelope.items[0].series_variant = Some("x".repeat(30 * 1024));
    let mut second = request.envelope.items[0].clone();
    second.subject_hint = Some("incoming-2".into());
    request.envelope.items.push(second);
    let staged_bytes = request
        .envelope
        .items
        .iter()
        .map(|item| serde_json::to_string(item).unwrap().len())
        .sum::<usize>();
    assert!(staged_bytes <= 64 * 1024);

    let ack = collector.submit(request).await.unwrap();

    assert!(matches!(ack.status, AckStatus::Accepted { ref items }
    if items.len() == 2 && items.iter().all(|item| matches!(
        item,
        ItemStatus::Stored { disposition: Disposition::Staged, .. }
    ))));
    let state: (i64, i64, i64) = db
            .with_conn_sync(|conn| {
                Ok((
                    conn.query_row(
                        "SELECT COUNT(*) FROM staged_readings WHERE hardware_id IN ('incoming-1','incoming-2')",
                        [],
                        |row| row.get(0),
                    )?,
                    conn.query_row(
                        "SELECT COUNT(*) FROM sightings WHERE hardware_id IN ('incoming-1','incoming-2')",
                        [],
                        |row| row.get(0),
                    )?,
                    conn.query_row(
                        "SELECT COUNT(*) FROM ingest_dedup WHERE envelope_id='staging-envelope-within-reserve'",
                        [],
                        |row| row.get(0),
                    )?,
                ))
            })
            .unwrap();
    assert_eq!(state, (2, 2, 1));
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
async fn sqlite_capacity_exhaustion_is_atomic_and_retryable_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("capacity.db");
    let db = iotkit_core_storage::init_db(&db_path, &migration_set()).unwrap();
    register_active(&db, "ble:capacity");
    db.with_conn_sync(|conn| {
        iotkit_core_publish::store::target_insert(
            conn,
            &iotkit_core_publish::store::TargetRow {
                target_id: "edge".into(),
                endpoint_url: "mqtt://edge".into(),
                credential_token: String::new(),
                archive_responsible: true,
                schema_version: 1,
                cursor_epoch: None,
                cursor_pub_seq: 0,
            },
            1,
        )
        .unwrap();
        Ok(())
    })
    .unwrap();

    let (collector, handle) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);
    collector
        .submit(env("capacity-000000", "ble:capacity", "temperature_c"))
        .await
        .unwrap();

    let page_limit: i64 = db
        .with_conn_sync(|conn| {
            let page_count: i64 = conn.query_row("PRAGMA page_count", [], |row| row.get(0))?;
            let requested = page_count + 8;
            conn.pragma_update(None, "max_page_count", requested)?;
            conn.query_row("PRAGMA max_page_count", [], |row| row.get::<_, i64>(0))
                .map_err(iotkit_core_storage::StorageError::from)
        })
        .unwrap();

    let mut accepted = 1_i64;
    let failed_envelope_id = loop {
        let envelope_id = format!("capacity-{accepted:06}");
        match collector
            .submit(env(&envelope_id, "ble:capacity", "temperature_c"))
            .await
        {
            Ok(ack) => {
                assert!(matches!(ack.status, AckStatus::Accepted { .. }));
                accepted += 1;
                assert!(accepted < 10_000, "capacity limit was not reached");
            }
            Err(SubmitError::NoAck) => break envelope_id,
            Err(other) => panic!("unexpected submit error at capacity: {other:?}"),
        }
    };

    let state_before_reopen = db
        .with_conn_sync(|conn| {
            let readings: i64 =
                conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?;
            let outbox: i64 = conn.query_row(
                "SELECT COUNT(*) FROM publication_log WHERE kind='measurement'",
                [],
                |row| row.get(0),
            )?;
            let dedup: i64 = conn.query_row(
                "SELECT COUNT(*) FROM ingest_dedup WHERE sender_id='principal:test-adapter'",
                [],
                |row| row.get(0),
            )?;
            let target = iotkit_core_publish::store::target_get(conn)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?
                .unwrap();
            let check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
            let page_count: i64 = conn.query_row("PRAGMA page_count", [], |row| row.get(0))?;
            Ok((
                readings,
                outbox,
                dedup,
                target.cursor_pub_seq,
                check,
                page_count,
            ))
        })
        .unwrap();
    let (readings, outbox, dedup, cursor, check, page_count) = state_before_reopen;
    assert!(accepted > 1, "capacity test must retain a real backlog");
    assert!(
        page_count <= page_limit,
        "SQLite must not grow beyond the configured page limit"
    );
    assert_eq!(
        (readings, outbox, dedup, cursor, check),
        (accepted, accepted, accepted, 0, "ok".to_string()),
        "the failed envelope must leave no partial reading, outbox, or dedup claim"
    );

    db.with_conn_sync(|conn| {
        conn.pragma_update(None, "max_page_count", page_limit + 1_024)?;
        Ok(())
    })
    .unwrap();
    drop(collector);
    handle.await.unwrap();
    drop(db);

    let reopened = iotkit_core_storage::init_db(&db_path, &migration_set()).unwrap();
    let state_after_reopen = reopened
        .with_conn_sync(|conn| {
            let readings: i64 =
                conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?;
            let outbox: i64 = conn.query_row(
                "SELECT COUNT(*) FROM publication_log WHERE kind='measurement'",
                [],
                |row| row.get(0),
            )?;
            let check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
            Ok((readings, outbox, check))
        })
        .unwrap();
    assert_eq!(state_after_reopen, (accepted, accepted, "ok".to_string()));

    let (restarted_collector, restarted_handle) =
        Collector::spawn(reopened.clone(), Arc::new(PermissiveRegistry), 16);
    let retry = restarted_collector
        .submit(env(&failed_envelope_id, "ble:capacity", "temperature_c"))
        .await
        .unwrap();
    assert!(matches!(retry.status, AckStatus::Accepted { .. }));

    let recovered_state = reopened
        .with_conn_sync(|conn| {
            let readings: i64 =
                conn.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?;
            let outbox: i64 = conn.query_row(
                "SELECT COUNT(*) FROM publication_log WHERE kind='measurement'",
                [],
                |row| row.get(0),
            )?;
            let dedup: i64 = conn.query_row(
                "SELECT COUNT(*) FROM ingest_dedup WHERE sender_id='principal:test-adapter'",
                [],
                |row| row.get(0),
            )?;
            Ok((readings, outbox, dedup))
        })
        .unwrap();
    assert_eq!(recovered_state, (accepted + 1, accepted + 1, accepted + 1));

    drop(restarted_collector);
    restarted_handle.await.unwrap();
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
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ingest_dedup (sender_id, envelope_id, received_at) VALUES (?1, ?2, ?3)",
            rusqlite::params!["keep-sender", "keep-env", keep_at],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();

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
async fn dedup_purge_failure_degrades_once_without_invalidating_committed_ack() {
    let db = test_db();
    register_active(&db, "ble:aa");
    db.with_conn_sync(|conn| {
        conn.execute(
            "INSERT INTO ingest_dedup(sender_id,envelope_id,received_at) VALUES('old','old',0)",
            [],
        )?;
        conn.execute_batch(
            "CREATE TRIGGER fail_dedup_purge BEFORE DELETE ON ingest_dedup
                 BEGIN SELECT RAISE(FAIL, 'injected purge failure'); END;",
        )?;
        Ok(())
    })
    .unwrap();
    let (collector, _h) =
        Collector::spawn_with_purge_interval(db.clone(), Arc::new(PermissiveRegistry), 16, 0);

    let first = collector
        .submit(env("purge-failure-1", "ble:aa", "temperature_c"))
        .await
        .unwrap();
    assert!(matches!(first.status, AckStatus::Accepted { .. }));
    let second = collector
        .submit(env("purge-failure-2", "ble:aa", "humidity_pct"))
        .await
        .unwrap();
    assert!(matches!(second.status, AckStatus::Accepted { .. }));

    db.with_conn_sync(|conn| {
        let degraded: bool = conn.query_row(
            "SELECT degraded FROM ingest_dedup_maintenance WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        let starts: i64 = conn.query_row(
            "SELECT COUNT(*) FROM ledger_events WHERE kind='dedup_window_degraded'",
            [],
            |row| row.get(0),
        )?;
        assert!(degraded);
        assert_eq!(starts, 1);
        Ok(())
    })
    .unwrap();

    db.with_conn_sync(|conn| {
        conn.execute_batch("DROP TRIGGER fail_dedup_purge")?;
        Ok(())
    })
    .unwrap();
    collector
        .submit(env("purge-recovery-1", "ble:aa", "pressure_pa"))
        .await
        .unwrap();
    collector
        .submit(env("purge-recovery-2", "ble:aa", "voltage_v"))
        .await
        .unwrap();
    db.with_conn_sync(|conn| {
        let degraded: bool = conn.query_row(
            "SELECT degraded FROM ingest_dedup_maintenance WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        let recoveries: i64 = conn.query_row(
            "SELECT COUNT(*) FROM ledger_events WHERE kind='dedup_window_recovered'",
            [],
            |row| row.get(0),
        )?;
        assert!(!degraded);
        assert_eq!(recoveries, 1);
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn dedup_degradation_transition_retries_after_maintenance_update_failure() {
    let db = test_db();
    register_active(&db, "ble:aa");
    db.with_conn_sync(|conn| {
        conn.execute(
            "INSERT INTO ingest_dedup(sender_id,envelope_id,received_at) VALUES('old','old',0)",
            [],
        )?;
        conn.execute_batch(
            "CREATE TRIGGER fail_dedup_purge BEFORE DELETE ON ingest_dedup
                 BEGIN SELECT RAISE(FAIL, 'injected purge failure'); END;
                 CREATE TRIGGER fail_maintenance_update BEFORE UPDATE ON ingest_dedup_maintenance
                 BEGIN SELECT RAISE(FAIL, 'injected maintenance update failure'); END;",
        )?;
        Ok(())
    })
    .unwrap();
    let mut last_purge_ms = 0;
    let mut latch = DedupMaintenanceLatch::default();
    maybe_purge_dedup(
        &db,
        DEFAULT_PURGE_INTERVAL_MS,
        &mut last_purge_ms,
        &mut latch,
    )
    .await;
    db.with_conn_sync(|conn| {
        let degraded: bool = conn.query_row(
            "SELECT degraded FROM ingest_dedup_maintenance WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        assert!(!degraded, "the injected transition really must have failed");
        conn.execute_batch("DROP TRIGGER fail_maintenance_update")?;
        Ok(())
    })
    .unwrap();

    // Still inside the normal purge interval: only a retained transition latch can retry.
    maybe_purge_dedup(
        &db,
        DEFAULT_PURGE_INTERVAL_MS,
        &mut last_purge_ms,
        &mut latch,
    )
    .await;
    db.with_conn_sync(|conn| {
        let degraded: bool = conn.query_row(
            "SELECT degraded FROM ingest_dedup_maintenance WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        let starts: i64 = conn.query_row(
            "SELECT COUNT(*) FROM ledger_events WHERE kind='dedup_window_degraded'",
            [],
            |row| row.get(0),
        )?;
        assert!(degraded);
        assert_eq!(starts, 1);
        Ok(())
    })
    .unwrap();
}

async fn prove_dedup_transition_retry_after_fault(fault_sql: &str, remove_fault_sql: &str) {
    let db = test_db();
    register_active(&db, "ble:aa");
    db.with_conn_sync(|conn| {
        conn.execute(
            "INSERT INTO ingest_dedup(sender_id,envelope_id,received_at) VALUES('old','old',0)",
            [],
        )?;
        conn.execute_batch(
            "CREATE TRIGGER fail_dedup_purge BEFORE DELETE ON ingest_dedup
                 BEGIN SELECT RAISE(FAIL, 'injected purge failure'); END;",
        )?;
        conn.execute_batch(fault_sql)?;
        Ok(())
    })
    .unwrap();
    let mut last_purge_ms = 0;
    let mut latch = DedupMaintenanceLatch::default();
    maybe_purge_dedup(
        &db,
        DEFAULT_PURGE_INTERVAL_MS,
        &mut last_purge_ms,
        &mut latch,
    )
    .await;
    assert!(latch.pending.is_some());

    db.with_conn_sync(|conn| {
        conn.execute_batch(remove_fault_sql)?;
        Ok(())
    })
    .unwrap();
    maybe_purge_dedup(
        &db,
        DEFAULT_PURGE_INTERVAL_MS,
        &mut last_purge_ms,
        &mut latch,
    )
    .await;
    assert!(latch.pending.is_none());
    db.with_conn_sync(|conn| {
        let state: (bool, i64) = conn.query_row(
            "SELECT degraded,
                    (SELECT COUNT(*) FROM ledger_events WHERE kind='dedup_window_degraded')
                 FROM ingest_dedup_maintenance WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(state, (true, 1));
        conn.execute_batch("DROP TRIGGER fail_dedup_purge")?;
        Ok(())
    })
    .unwrap();
    last_purge_ms = 0;
    maybe_purge_dedup(
        &db,
        DEFAULT_PURGE_INTERVAL_MS,
        &mut last_purge_ms,
        &mut latch,
    )
    .await;
    db.with_conn_sync(|conn| {
        let state: (bool, i64, i64) = conn.query_row(
            "SELECT degraded,
                    (SELECT COUNT(*) FROM ledger_events WHERE kind='dedup_window_degraded'),
                    (SELECT COUNT(*) FROM ledger_events WHERE kind='dedup_window_recovered')
                 FROM ingest_dedup_maintenance WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(state, (false, 1, 1));
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn dedup_degradation_transition_retries_after_ledger_insert_failure() {
    prove_dedup_transition_retry_after_fault(
        "CREATE TRIGGER fail_maintenance_ledger BEFORE INSERT ON ledger_events
             WHEN new.kind='dedup_window_degraded'
             BEGIN SELECT RAISE(FAIL, 'injected ledger insert failure'); END;",
        "DROP TRIGGER fail_maintenance_ledger",
    )
    .await;
}

#[tokio::test]
async fn dedup_degradation_transition_retries_after_commit_failure() {
    prove_dedup_transition_retry_after_fault(
        "PRAGMA foreign_keys=ON;
             CREATE TABLE maintenance_fault_parent(id INTEGER PRIMARY KEY);
             CREATE TABLE maintenance_fault_child(
                 parent_id INTEGER REFERENCES maintenance_fault_parent(id)
                     DEFERRABLE INITIALLY DEFERRED
             );
             CREATE TRIGGER fail_maintenance_commit AFTER INSERT ON ledger_events
             WHEN new.kind='dedup_window_degraded'
             BEGIN INSERT INTO maintenance_fault_child(parent_id) VALUES(999); END;",
        "DROP TRIGGER fail_maintenance_commit",
    )
    .await;
}

#[tokio::test]
async fn validation_never_triggers_opportunistic_dedup_purge() {
    let db = test_db();
    register_active(&db, "ble:aa");
    let old_at = now_ms() - 100 * 60 * 60 * 1000;
    db.with_conn_sync(|conn| {
        conn.execute(
            "INSERT INTO ingest_dedup (sender_id, envelope_id, received_at)
                 VALUES ('validation-purge-sentinel', 'old-env', ?1)",
            [old_at],
        )?;
        Ok(())
    })
    .unwrap();

    let (collector, _h) =
        Collector::spawn_with_purge_interval(db.clone(), Arc::new(PermissiveRegistry), 16, 0);
    collector
        .validate(env("validate-purge-1", "ble:aa", "temperature_c"))
        .await
        .unwrap();
    collector
        .validate(env("validate-purge-2", "ble:aa", "humidity_pct"))
        .await
        .unwrap();

    let sentinel_count: i64 = db
        .with_conn_sync(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM ingest_dedup
                     WHERE sender_id='validation-purge-sentinel'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(sentinel_count, 1, "validation must not mutate dedup state");
}

#[tokio::test]
async fn cache_is_reset_after_storage_failure() {
    // 回帰テスト: process_itemはensure_seriesのINSERT直後(コミット前)にcache.seriesへ
    // 書き込む。同一envelope内の後続itemがストレージ失敗すると全体がロールバックされ、
    // series行はDBから消えるが、修正前はcacheに幻のseries_idが残っていた。次のenvelopeが
    // 同キーを使うとFK違反→ackなしの無限ループ(再送しても回復しない=D1違反)になる。
    //
    let db = test_db();
    register_active(&db, "ble:aa");
    db.with_conn_sync(|conn| {
        conn.execute_batch(
            "CREATE TRIGGER fail_second_reading BEFORE INSERT ON readings
                 WHEN new.values_json='[2.0]'
                 BEGIN SELECT RAISE(FAIL, 'injected second reading failure'); END;",
        )?;
        Ok(())
    })
    .unwrap();
    let (collector, _h) = Collector::spawn(db.clone(), Arc::new(PermissiveRegistry), 16);

    let make_item = |value: f64| ReadingItem {
        subject_hint: Some("ble:aa".into()),
        measurement_key: "temp_a".into(),
        channel_index: None,
        series_variant: None,
        values: vec![value],
        device_time_ms: None,
        time_source: TimeSource::EdgeNode,
        age_ms: None,
        rssi: None,
        battery_pct: None,
    };

    // 1件目: 新規series(=temp_a)を作成しキャッシュに載せる。2件目: triggerで書き込み失敗。
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
            items: vec![make_item(1.0), make_item(2.0)],
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

    db.with_conn_sync(|conn| {
        conn.execute_batch("DROP TRIGGER fail_second_reading")?;
        Ok(())
    })
    .unwrap();

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
        principal: IngestPrincipal::test_authenticated_device(
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
