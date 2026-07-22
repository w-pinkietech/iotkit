use super::*;
use iotkit_core_ledger as ledger;
use iotkit_core_storage::init_db_memory;

fn v3_db() -> iotkit_core_storage::DbHandle {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(ledger::MIGRATIONS);
    all.extend_from_slice(crate::MIGRATIONS); // v4, v7, v8
    // 昇順必須: 1(ledgerなし), 3, 4, 5, 7, 8 の順に並べ替え
    all.sort_by_key(|m| m.version);
    init_db_memory(&all).unwrap()
}

fn db_before_v8() -> iotkit_core_storage::DbHandle {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend(ledger::MIGRATIONS.iter().copied().filter(|m| m.version < 8));
    all.extend(crate::MIGRATIONS.iter().copied().filter(|m| m.version < 8));
    all.sort_by_key(|m| m.version);
    init_db_memory(&all).unwrap()
}

fn seed_series(conn: &rusqlite::Connection) -> i64 {
    let sid = ledger::insert_device(
        conn,
        &ledger::NewDevice {
            hardware_id: "ble:aa".into(),
            user_label: None,
            parent: None,
            kind: ledger::DeviceKind::Individual,
            initial_state: ledger::DeviceState::Active,
        },
    )
    .unwrap();
    ledger::ensure_series(
        conn,
        &sid,
        "temperature_c",
        ledger::CHANNEL_NA,
        ledger::DEFAULT_VARIANT,
        false,
        None,
    )
    .unwrap()
}

fn seed_series_before_v8(conn: &rusqlite::Connection) -> i64 {
    conn.execute(
        "INSERT INTO devices (
                system_id, hardware_id, kind, state, created_at
             ) VALUES (?1, 'ble:aa', 'individual', 'active', 0)",
        rusqlite::params![vec![1_u8; 16]],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO series (
                system_id, measurement_key, channel_index, variant, created_at
             ) VALUES (?1, 'temperature_c', -1, 'primary', 0)",
        rusqlite::params![vec![1_u8; 16]],
    )
    .unwrap();
    conn.last_insert_rowid()
}

#[test]
fn claim_envelope_detects_duplicates() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
        assert!(try_claim_envelope(conn, "adapterA", "e-1").unwrap());
        assert!(!try_claim_envelope(conn, "adapterA", "e-1").unwrap());
        assert!(try_claim_envelope(conn, "adapterB", "e-1").unwrap()); // 送信者スコープ(D1)
        Ok(())
    })
    .unwrap();
}

#[test]
fn insert_reading_v3_returns_monotonic_seq() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
        let series_id = seed_series(conn);
        let r = NewReading {
            series_id,
            received_at_ms: 1000,
            device_time_ms: None,
            time_source: "edge_node".into(),
            values: vec![21.5],
            rssi: None,
            battery_pct: None,
            quarantined: false,
        };
        let s1 = insert_reading_v3(conn, &r).unwrap();
        let s2 = insert_reading_v3(conn, &r).unwrap(); // 同時刻・同値でも別行(v2の暗黙dedup廃止)
        assert!(s2 > s1);
        Ok(())
    })
    .unwrap();
}

fn insert_and_read_event_time(
    conn: &rusqlite::Connection,
    received_at_ms: i64,
    device_time_ms: Option<i64>,
    time_source: &str,
) -> (i64, String) {
    let series_id = seed_series(conn);
    let r = NewReading {
        series_id,
        received_at_ms,
        device_time_ms,
        time_source: time_source.into(),
        values: vec![21.5],
        rssi: None,
        battery_pct: None,
        quarantined: false,
    };
    let seq = insert_reading_v3(conn, &r).unwrap();
    conn.query_row(
        "SELECT event_time, event_time_source FROM readings WHERE seq = ?1",
        rusqlite::params![seq],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

#[test]
fn event_time_prefers_device_time_within_tolerance() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
        let received_at = 10_000_000;
        let (event_time, source) = insert_and_read_event_time(
            conn,
            received_at,
            Some(received_at - 3 * 60 * 60 * 1000),
            "device_ntp",
        );
        assert_eq!(event_time, received_at - 3 * 60 * 60 * 1000);
        assert_eq!(source, "device");
        Ok(())
    })
    .unwrap();
}

#[test]
fn event_time_edge_node_adjusted_source() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
        let received_at = 10_000_000;
        let (event_time, source) = insert_and_read_event_time(
            conn,
            received_at,
            Some(received_at - 5000),
            "edge_node_adjusted",
        );
        assert_eq!(event_time, received_at - 5000);
        assert_eq!(source, "edge_node_adjusted");
        Ok(())
    })
    .unwrap();
}

#[test]
fn event_time_ignores_device_time_when_source_is_edge() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
        let received_at = 10_000_000;
        let (event_time, source) =
            insert_and_read_event_time(conn, received_at, Some(received_at - 5000), "edge_node");
        assert_eq!(event_time, received_at);
        assert_eq!(source, "received_at");
        Ok(())
    })
    .unwrap();
}

#[test]
fn event_time_accepts_device_time_at_future_tolerance_boundary() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
        let received_at = 10_000_000;
        let (event_time, source) = insert_and_read_event_time(
            conn,
            received_at,
            Some(received_at + FUTURE_TOLERANCE_MS),
            "device_ntp",
        );
        assert_eq!(event_time, received_at + FUTURE_TOLERANCE_MS);
        assert_eq!(source, "device");
        Ok(())
    })
    .unwrap();
}

#[test]
fn event_time_demotes_device_time_beyond_future_tolerance() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
        let received_at = 10_000_000;
        let (event_time, source) = insert_and_read_event_time(
            conn,
            received_at,
            Some(received_at + FUTURE_TOLERANCE_MS + 1),
            "device_ntp",
        );
        assert_eq!(event_time, received_at);
        assert_eq!(source, "received_at");
        Ok(())
    })
    .unwrap();
}

#[test]
fn event_time_falls_back_to_received_at() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
        let received_at = 10_000_000;
        let (event_time, source) = insert_and_read_event_time(conn, received_at, None, "edge_node");
        assert_eq!(event_time, received_at);
        assert_eq!(source, "received_at");
        Ok(())
    })
    .unwrap();
}

#[test]
fn staged_readings_are_bounded_per_hardware_id() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
        for i in 0..1005 {
            insert_staged_reading(conn, "ble:new", i, "{}").unwrap();
        }
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM staged_readings WHERE hardware_id='ble:new'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1000);
        let oldest: i64 = conn
            .query_row(
                "SELECT MIN(received_at) FROM staged_readings WHERE hardware_id='ble:new'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(oldest, 5); // 最古削除
        Ok(())
    })
    .unwrap();
}

#[test]
fn purge_dedup_before_removes_old_entries() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
        try_claim_envelope(conn, "a", "old").unwrap();
        conn.execute("UPDATE ingest_dedup SET received_at = 0", [])
            .unwrap();
        try_claim_envelope(conn, "a", "new").unwrap();
        assert_eq!(purge_dedup_before(conn, 1).unwrap(), 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn bounded_staging_enforces_principal_global_rows_bytes_age_and_pin_reserve() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
        let limits = StagingLimits::new(4, 3, 40, 30, 100, 2, 20).unwrap();
        stage_sighting_at(conn, "p1", "old", 0, "1234567890", limits).unwrap();
        stage_sighting_at(conn, "p1", "pinned", 90, "1234567890", limits).unwrap();
        set_sighting_pin(conn, "p1", "pinned", true, limits).unwrap();

        let outcome = stage_sighting_at(conn, "p1", "new", 101, "1234567890", limits).unwrap();
        assert_eq!(outcome.expired_subjects, 1);
        assert!(staging_subject_exists(conn, "p1", "pinned").unwrap());
        assert!(staging_subject_exists(conn, "p1", "new").unwrap());

        stage_sighting_at(conn, "p2", "g1", 102, "1234567890", limits).unwrap();
        stage_sighting_at(conn, "p2", "g2", 103, "1234567890", limits).unwrap();
        let outcome = stage_sighting_at(conn, "p2", "g3", 104, "1234567890", limits).unwrap();
        assert_eq!(outcome.evicted_subjects, 1);
        assert!(staging_subject_exists(conn, "p1", "pinned").unwrap());
        let health = staging_health(conn, limits).unwrap();
        assert!(health.rows <= 4);
        assert!(health.bytes <= 40);
        assert!(health.principals <= 2);

        let err = set_sighting_pin(conn, "p1", "new", true, limits).unwrap_err();
        assert!(matches!(err, TimeseriesError::Limit(_)));
        Ok(())
    })
    .unwrap();
}

#[test]
fn bounded_staging_rejects_oversize_without_evicting_protected_data() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
        let limits = StagingLimits::new(3, 2, 24, 16, 1_000, 1, 8).unwrap();
        stage_sighting_at(conn, "p1", "protected", 10, "12345678", limits).unwrap();
        set_sighting_pin(conn, "p1", "protected", true, limits).unwrap();
        let err = stage_sighting_at(conn, "p1", "hostile", 11, "123456789", limits).unwrap_err();
        assert!(matches!(err, TimeseriesError::Limit(_)));
        assert!(staging_subject_exists(conn, "p1", "protected").unwrap());
        assert!(!staging_subject_exists(conn, "p1", "hostile").unwrap());
        Ok(())
    })
    .unwrap();
}

#[test]
fn staging_batch_never_evicts_a_current_envelope_subject() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
            let limits = StagingLimits::new(3, 3, 30, 30, 1_000, 2, 20).unwrap();
            stage_sighting_at(conn, "p1", "incoming", 10, "1234567890", limits).unwrap();
            stage_sighting_at(conn, "p1", "older-sibling", 11, "1234567890", limits).unwrap();

            let outcome = stage_sightings_at(
                conn,
                "p1",
                12,
                &[
                    StagedSighting {
                        hardware_id: "incoming",
                        payload_json: "1234567890",
                    },
                    StagedSighting {
                        hardware_id: "new-sibling",
                        payload_json: "1234567890",
                    },
                ],
                limits,
            )
            .unwrap();

            assert_eq!(outcome.evicted_subjects, 1);
            let incoming_rows: i64 = conn.query_row(
                "SELECT COUNT(*) FROM staged_readings WHERE principal_id='p1' AND hardware_id='incoming'",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(incoming_rows, 2);
            assert!(!staging_subject_exists(conn, "p1", "older-sibling").unwrap());
            assert!(staging_subject_exists(conn, "p1", "new-sibling").unwrap());
            Ok(())
        })
        .unwrap();
}

#[test]
fn pinned_subject_inherits_pin_and_survives_row_byte_and_age_pressure() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
            let limits = StagingLimits::new(4, 4, 40, 40, 100, 1, 10).unwrap();
            stage_sighting_at(conn, "p1", "protected", 0, "1234567890", limits).unwrap();
            set_sighting_pin(conn, "p1", "protected", true, limits).unwrap();

            // A later observation of a pinned subject must be protected too.
            stage_sighting_at(conn, "p1", "protected", 200, "1234567890", limits).unwrap();
            let protected_unpinned: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM staged_readings WHERE principal_id='p1' AND hardware_id='protected' AND pinned=0",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(protected_unpinned, 0);

            stage_sighting_at(conn, "p1", "oldest-eligible", 201, "1234567890", limits)
                .unwrap();
            stage_sighting_at(conn, "p1", "newer-eligible", 202, "1234567890", limits)
                .unwrap();
            let outcome =
                stage_sighting_at(conn, "p1", "incoming", 203, "1234567890", limits).unwrap();

            assert_eq!(outcome.evicted_subjects, 1);
            assert!(staging_subject_exists(conn, "p1", "protected").unwrap());
            assert!(!staging_subject_exists(conn, "p1", "oldest-eligible").unwrap());
            assert!(staging_subject_exists(conn, "p1", "newer-eligible").unwrap());
            assert!(staging_subject_exists(conn, "p1", "incoming").unwrap());

            // Age cleanup may delete eligible rows, but never any row in a pinned subject.
            let outcome =
                stage_sighting_at(conn, "p1", "after-age", 400, "1234567890", limits).unwrap();
            assert!(outcome.expired_subjects >= 1);
            let protected_rows: (i64, i64) = conn
                .query_row(
                    "SELECT COUNT(*), SUM(pinned) FROM staged_readings WHERE principal_id='p1' AND hardware_id='protected'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(protected_rows, (2, 2));
            stage_sighting_at(conn, "p1", "protected", 401, "1234567890", limits).unwrap();
            let reserve_error =
                stage_sighting_at(conn, "p1", "protected", 402, "1234567890", limits)
                    .unwrap_err();
            assert!(matches!(reserve_error, TimeseriesError::Limit(_)));
            let protected_rows: (i64, i64) = conn
                .query_row(
                    "SELECT COUNT(*), SUM(pinned) FROM staged_readings WHERE principal_id='p1' AND hardware_id='protected'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(protected_rows, (3, 3));
            Ok(())
        })
        .unwrap();
}

#[test]
fn staging_cleanup_preserves_refreshed_and_legacy_sightings_while_rows_survive() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
            let limits = StagingLimits::new(8, 6, 80, 60, 100, 1, 10).unwrap();

            ledger::record_sighting(conn, "ble:shared", "official-a").unwrap();
            stage_sighting_at(conn, "official-a", "ble:shared", 0, "1234567890", limits)
                .unwrap();
            stage_sighting_at(conn, "official-a", "ble:shared", 150, "1234567890", limits)
                .unwrap();
            set_sighting_pin(conn, "official-a", "ble:shared", true, limits).unwrap();

            // A different authenticated official principal becomes the canonical owner.
            ledger::record_sighting(conn, "ble:shared", "official-b").unwrap();
            stage_sighting_at(conn, "official-b", "ble:shared", 0, "1234567890", limits)
                .unwrap();
            stage_sighting_at(conn, "official-b", "ble:shared", 150, "1234567890", limits)
                .unwrap();

            // Legacy rows have unknown ownership. Their safe policy is retention: they may
            // keep approval metadata alive, but never grant staging authority.
            ledger::record_sighting(conn, "ble:legacy", "historical-adapter").unwrap();
            conn.execute(
                "INSERT INTO staged_readings (hardware_id, received_at, payload_json, principal_id, payload_bytes, pinned) VALUES ('ble:legacy', 0, '{}', 'legacy:unknown', 2, 0)",
                [],
            )
            .unwrap();

            stage_sighting_at(conn, "official-c", "trigger", 200, "1234567890", limits)
                .unwrap();

            let shared: (String, i64) = conn
                .query_row(
                    "SELECT source, observations FROM sightings WHERE hardware_id='ble:shared'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(shared.0, "official-b");
            assert!(shared.1 >= 2);
            assert!(staging_subject_exists(conn, "official-a", "ble:shared").unwrap());
            assert!(staging_subject_exists(conn, "official-b", "ble:shared").unwrap());
            let legacy_sighting: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sightings WHERE hardware_id='ble:legacy')",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(legacy_sighting);

            // Surviving acknowledged staging must remain approvable.
            ledger::approve_sighting(
                conn,
                "ble:shared",
                None,
                ledger::DeviceKind::Individual,
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
}

#[test]
fn bounded_dedup_enforces_principal_global_count_and_age() {
    let db = v3_db();
    db.with_conn_sync(|conn| {
        let limits = DedupLimits::new(3, 2, 100).unwrap();
        assert!(try_claim_envelope_bounded_at(conn, "p1", "old", 0, limits).unwrap());
        assert!(try_claim_envelope_bounded_at(conn, "p1", "a", 90, limits).unwrap());
        assert!(try_claim_envelope_bounded_at(conn, "p1", "b", 101, limits).unwrap());
        assert!(try_claim_envelope_bounded_at(conn, "p2", "c", 102, limits).unwrap());
        assert!(try_claim_envelope_bounded_at(conn, "p2", "d", 103, limits).unwrap());
        assert!(!try_claim_envelope_bounded_at(conn, "p2", "d", 104, limits).unwrap());

        let health = dedup_health(conn, limits).unwrap();
        assert_eq!(health.rows, 3);
        assert!(health.oldest_age_ms <= 100);
        assert!(health.max_principal_rows <= 2);
        Ok(())
    })
    .unwrap();
}

#[test]
fn concurrent_staging_admission_serializes_eviction_and_preserves_pinned_subject() {
    use std::sync::{Arc, Barrier};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("concurrent-staging.db");
    let db = iotkit_core_storage::init_db(&path, &{
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(ledger::MIGRATIONS);
        all.extend_from_slice(crate::MIGRATIONS);
        all.sort_by_key(|migration| migration.version);
        all
    })
    .unwrap();
    let limits = StagingLimits::new(6, 4, 60, 40, 10_000, 1, 10).unwrap();
    db.with_conn_sync(|conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
        stage_sighting_at(&tx, "protected", "subject", 1, "1234567890", limits).unwrap();
        set_sighting_pin(&tx, "protected", "subject", true, limits).unwrap();
        tx.commit()?;
        Ok(())
    })
    .unwrap();

    let barrier = Arc::new(Barrier::new(12));
    let handles = (0..12)
        .map(|index| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let mut conn = rusqlite::Connection::open(path).unwrap();
                conn.busy_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                barrier.wait();
                let tx = conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .unwrap();
                let principal = format!("p{}", index % 3);
                let subject = format!("s{index}");
                stage_sighting_at(&tx, &principal, &subject, 100 + index, "1234567890", limits)
                    .unwrap();
                assert!(staging_subject_exists(&tx, &principal, &subject).unwrap());
                tx.commit().unwrap();
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }
    db.with_conn_sync(|conn| {
        let health = staging_health(conn, limits).unwrap();
        assert!(health.rows <= 6);
        assert!(health.bytes <= 60);
        assert!(staging_subject_exists(conn, "protected", "subject").unwrap());
        Ok(())
    })
    .unwrap();
}

#[test]
fn migration_v7_drops_sensor_readings_from_legacy_db() {
    let db = iotkit_core_storage::init_db_memory(iotkit_core_storage::MIGRATIONS).unwrap();
    db.with_conn_sync(|conn| {
            conn.execute_batch(
                "CREATE TABLE sensor_readings (
                    adapter_id  TEXT NOT NULL,
                    device_key  TEXT NOT NULL,
                    ingested_at INTEGER NOT NULL,
                    sensor_type TEXT NOT NULL,
                    values_json TEXT NOT NULL,
                    rssi        INTEGER,
                    battery_pct INTEGER,
                    PRIMARY KEY (adapter_id, device_key, ingested_at, sensor_type)
                );
                INSERT INTO sensor_readings
                    (adapter_id, device_key, ingested_at, sensor_type, values_json)
                VALUES ('a1', 'd1', 1000, 'temperature', '[21.0]');
                INSERT INTO _schema_version (version, label, applied_at)
                VALUES (2, 'timeseries', 0);",
            )
            .unwrap();

            let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
            all.extend_from_slice(ledger::MIGRATIONS);
            all.extend_from_slice(crate::MIGRATIONS);
            all.sort_by_key(|m| m.version);
            iotkit_core_storage::run_migrations(conn, &all).unwrap();

            let table_exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='sensor_readings'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(!table_exists, "sensor_readings must be dropped by v7");
            Ok(())
        })
        .unwrap();
}

#[test]
fn migration_v8_backfills_event_time_from_real_rows() {
    let db = db_before_v8();
    db.with_conn_sync(|conn| {
        // Reproduce an actual pre-v8 schema. Current ledger helpers select columns
        // introduced by later migrations and must not be used to seed old schemas.
        let series_id = seed_series_before_v8(conn);
        let rows = [
            (1, 10_000_000, None, "edge_node"),
            (2, 10_000_000, Some(9_990_000), "device_ntp"),
            (3, 10_000_000, Some(10_300_001), "device_ntp"),
            (4, 10_000_000, Some(9_995_000), "edge_node"),
        ];
        for (seq, received_at, device_time, time_source) in rows {
            conn.execute(
                "INSERT INTO readings
                        (seq, series_id, received_at, device_time, time_source, values_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, '[21.5]')",
                rusqlite::params![seq, series_id, received_at, device_time, time_source],
            )
            .unwrap();
        }

        let v8 = *crate::MIGRATIONS.iter().find(|m| m.version == 8).unwrap();
        iotkit_core_storage::run_migrations(conn, &[v8]).unwrap();

        let actual: Vec<(i64, i64, String)> = conn
            .prepare("SELECT seq, event_time, event_time_source FROM readings ORDER BY seq")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            actual,
            vec![
                (1, 10_000_000, "received_at".to_string()),
                (2, 9_990_000, "device".to_string()),
                (3, 10_000_000, "received_at".to_string()),
                (4, 10_000_000, "received_at".to_string()),
            ]
        );
        Ok(())
    })
    .unwrap();
}
