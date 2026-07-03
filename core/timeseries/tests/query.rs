use iotkit_core_timeseries::{
    NewReading, insert_reading_v3,
    insert_staged_reading,
    query::{
        aggregate_readings_v3, export_csv, latest_by_series, list_staged_for_hardware,
        mark_readings_quarantined, query_readings_v3,
    },
};
use rusqlite::params;

fn test_db() -> iotkit_core_storage::DbHandle {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.sort_by_key(|m| m.version);
    iotkit_core_storage::init_db_memory(&all).unwrap()
}

fn seed_series(conn: &rusqlite::Connection) -> i64 {
    let sid = iotkit_core_ledger::insert_device(
        conn,
        &iotkit_core_ledger::NewDevice {
            hardware_id: "ble:q".into(),
            user_label: None,
            parent: None,
            kind: iotkit_core_ledger::DeviceKind::Individual,
            initial_state: iotkit_core_ledger::DeviceState::Active,
        },
    )
    .unwrap();
    iotkit_core_ledger::ensure_series(
        conn,
        &sid,
        "temperature_c",
        iotkit_core_ledger::CHANNEL_NA,
        iotkit_core_ledger::DEFAULT_VARIANT,
        false,
        None,
    )
    .unwrap()
}

fn insert_row(
    conn: &rusqlite::Connection,
    series_id: i64,
    seq: i64,
    event_time: i64,
    values_json: &str,
    quarantined: bool,
) {
    conn.execute(
        "INSERT INTO readings
            (seq, series_id, received_at, device_time, time_source, time_quality,
             event_time, event_time_source, values_json, rssi, battery_pct, quarantined)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            seq,
            series_id,
            event_time + 100,
            event_time,
            "device_ntp",
            "unsynced",
            event_time,
            "device",
            values_json,
            -70i16,
            90u8,
            quarantined as i32
        ],
    )
    .unwrap();
}

#[test]
fn query_readings_uses_event_time_range_order_limit_and_quarantine_switch() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let series_id = seed_series(conn);
        insert_row(conn, series_id, 1, 900, "[9.0]", false);
        insert_row(conn, series_id, 3, 1100, "[11.0]", false);
        insert_row(conn, series_id, 2, 1100, "[10.0]", true);
        insert_row(conn, series_id, 4, 1200, "[12.0]", false);
        insert_row(conn, series_id, 5, 1300, "[13.0]", false);
        insert_row(conn, series_id, 6, 1250, "[12.5]", false);

        let rows = query_readings_v3(conn, series_id, 1000, 1300, 2, false).unwrap();
        assert_eq!(rows.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![3, 4]);
        assert_eq!(
            rows.iter().map(|r| r.event_time).collect::<Vec<_>>(),
            vec![1100, 1200]
        );
        assert!(rows.iter().all(|r| !r.quarantined));

        let rows = query_readings_v3(conn, series_id, 1000, 1300, 10, true).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![2, 3, 4, 6]
        );
        assert_eq!(rows[0].values, vec![10.0]);
        Ok(())
    })
    .unwrap();
}

#[test]
fn aggregate_readings_buckets_scalars_and_rejects_multivalue_rows() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let series_id = seed_series(conn);
        insert_row(conn, series_id, 1, 1000, "[1.0]", false);
        insert_row(conn, series_id, 2, 1499, "[3.0]", false);
        insert_row(conn, series_id, 3, 1500, "[5.0]", false);
        insert_row(conn, series_id, 4, 2500, "[7.0]", false);

        let buckets = aggregate_readings_v3(conn, series_id, 1000, 3000, 500, false).unwrap();
        assert_eq!(buckets.len(), 3);
        assert_eq!((buckets[0].bucket_start, buckets[0].count), (1000, 2));
        assert_eq!((buckets[0].min, buckets[0].max, buckets[0].avg), (1.0, 3.0, 2.0));
        assert_eq!((buckets[1].bucket_start, buckets[1].count), (1500, 1));
        assert_eq!((buckets[2].bucket_start, buckets[2].count), (2500, 1));

        assert!(aggregate_readings_v3(conn, series_id, 1000, 3000, 0, false).is_err());
        assert!(aggregate_readings_v3(conn, series_id, 1000, 3000, -1, false).is_err());

        insert_row(conn, series_id, 5, 2600, "[1.0,2.0]", false);
        assert!(aggregate_readings_v3(conn, series_id, 1000, 3000, 500, false).is_err());
        Ok(())
    })
    .unwrap();
}

#[test]
fn aggregate_readings_rejects_empty_value_rows() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let series_id = seed_series(conn);
        insert_row(conn, series_id, 1, 1000, "[1.0]", false);
        insert_row(conn, series_id, 2, 1100, "[]", false);

        assert!(aggregate_readings_v3(conn, series_id, 1000, 1200, 500, false).is_err());
        Ok(())
    })
    .unwrap();
}

#[test]
fn export_csv_quotes_string_columns_and_preserves_empty_optional_fields() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let series_id = seed_series(conn);
        insert_reading_v3(
            conn,
            &NewReading {
                series_id,
                received_at_ms: 1000,
                device_time_ms: None,
                time_source: "gateway,edge".into(),
                values: vec![1.25],
                rssi: None,
                battery_pct: None,
                quarantined: false,
            },
        )
        .unwrap();
        insert_row(conn, series_id, 2, 1100, "[2.0,3.0]", false);
        let rows = query_readings_v3(conn, series_id, 1000, 1300, 10, false).unwrap();

        let mut out = Vec::new();
        export_csv(&mut out, &rows).unwrap();
        let csv = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = csv.trim_end().split('\n').collect();
        assert_eq!(
            lines[0],
            "seq,event_time,event_time_source,received_at,device_time,time_source,time_quality,quarantined,rssi,battery_pct,v0,v1"
        );
        assert_eq!(
            lines[1],
            "1,1000,received_at,1000,,\"gateway,edge\",unsynced,0,,,1.25,"
        );
        assert_eq!(
            lines[2],
            "2,1100,device,1200,1100,device_ntp,unsynced,0,-70,90,2,3"
        );
        assert_eq!(lines.len(), 3);
        Ok(())
    })
    .unwrap();
}

#[test]
fn latest_by_series_uses_event_time_then_seq_tiebreaker() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let series_id = seed_series(conn);
        insert_row(conn, series_id, 1, 1000, "[1.0]", false);
        insert_row(conn, series_id, 2, 2000, "[2.0]", false);
        insert_row(conn, series_id, 3, 2000, "[3.0]", false);
        insert_row(conn, series_id, 4, 1500, "[4.0]", false);

        let row = latest_by_series(conn, series_id).unwrap().unwrap();
        assert_eq!(row.seq, 3);
        assert_eq!(row.values, vec![3.0]);
        Ok(())
    })
    .unwrap();
}

#[test]
fn list_staged_for_hardware_returns_newest_rows_for_that_hardware() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        insert_staged_reading(conn, "ble:target", 1000, r#"{"v":1}"#).unwrap();
        insert_staged_reading(conn, "ble:other", 1500, r#"{"v":9}"#).unwrap();
        insert_staged_reading(conn, "ble:target", 2000, r#"{"v":2}"#).unwrap();

        let rows = list_staged_for_hardware(conn, "ble:target", 1).unwrap();
        assert_eq!(rows, vec![(2000, r#"{"v":2}"#.to_string())]);
        Ok(())
    })
    .unwrap();
}

#[test]
fn mark_readings_quarantined_uses_received_at_range_for_multiple_series() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let first = seed_series(conn);
        let sid = iotkit_core_ledger::insert_device(
            conn,
            &iotkit_core_ledger::NewDevice {
                hardware_id: "ble:q2".into(),
                user_label: None,
                parent: None,
                kind: iotkit_core_ledger::DeviceKind::Individual,
                initial_state: iotkit_core_ledger::DeviceState::Active,
            },
        )
        .unwrap();
        let second = iotkit_core_ledger::ensure_series(
            conn,
            &sid,
            "voltage_mv",
            0,
            iotkit_core_ledger::DEFAULT_VARIANT,
            false,
            None,
        )
        .unwrap();

        conn.execute(
            "INSERT INTO readings
                (seq, series_id, received_at, device_time, time_source, time_quality,
                 event_time, event_time_source, values_json, rssi, battery_pct, quarantined)
             VALUES
                (1, ?1, 5000, 1000, 'device_ntp', 'unsynced', 1000, 'device', '[1.0]', NULL, NULL, 0),
                (2, ?1, 3500, 9000, 'device_ntp', 'unsynced', 9000, 'device', '[2.0]', NULL, NULL, 0),
                (3, ?2, 7000, 1100, 'device_ntp', 'unsynced', 1100, 'device', '[3.0]', NULL, NULL, 0),
                (4, ?2, 9000, 1200, 'device_ntp', 'unsynced', 1200, 'device', '[4.0]', NULL, NULL, 0)",
            params![first, second],
        )
        .unwrap();

        let updated = mark_readings_quarantined(conn, &[first, second], 4000, 8000).unwrap();

        assert_eq!(updated, 2);
        let rows: Vec<(i64, i64)> = conn
            .prepare("SELECT seq, quarantined FROM readings ORDER BY seq")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, vec![(1, 1), (2, 0), (3, 1), (4, 0)]);
        Ok(())
    })
    .unwrap();
}
