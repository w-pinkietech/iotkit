use super::*;
use iotkit_core_storage::init_db_memory;

fn test_db() -> iotkit_core_storage::DbHandle {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(crate::MIGRATIONS);
    init_db_memory(&all).expect("in-memory db")
}

#[test]
fn series_key_round_trips_na_and_numeric_channels() {
    let sid = SystemId::from_bytes([0x01_u8; 16]);
    let na_key = series_key_of(&sid, "temperature_c", CHANNEL_NA, DEFAULT_VARIANT);
    assert_eq!(
        na_key,
        format!("{}:temperature_c:na:primary", sid.to_text())
    );
    let parsed = parse_series_key(&na_key).unwrap();
    assert_eq!(parsed.system_id, sid);
    assert_eq!(parsed.measurement_key, "temperature_c");
    assert_eq!(parsed.channel_index, CHANNEL_NA);
    assert_eq!(parsed.variant, DEFAULT_VARIANT);

    let count_key = series_key_of(&sid, "contact_state", 2, "count");
    assert_eq!(
        count_key,
        format!("{}:contact_state:2:count", sid.to_text())
    );
    let parsed = parse_series_key(&count_key).unwrap();
    assert_eq!(parsed.channel_index, 2);
    assert_eq!(parsed.variant, "count");
}

#[test]
fn parse_series_key_rejects_non_four_part_keys() {
    assert!(matches!(
        parse_series_key("not-enough-parts"),
        Err(LedgerError::InvalidId(_))
    ));
    assert!(matches!(
        parse_series_key("00000000-0000-0000-0000-000000000000:bad:1:too:many"),
        Err(LedgerError::InvalidId(_))
    ));
}

#[test]
fn find_and_list_series_use_derived_series_key() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = insert_device(
            conn,
            &NewDevice {
                hardware_id: "rpi-local:default:i2c:0x60".into(),
                user_label: Some("Rack sensor".into()),
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        let na_id = ensure_series(
            conn,
            &sid,
            "temperature_c",
            CHANNEL_NA,
            DEFAULT_VARIANT,
            false,
            None,
        )
        .unwrap();
        let count_id = ensure_series(conn, &sid, "contact_state", 2, "count", false, None).unwrap();

        let na_key = series_key_of(&sid, "temperature_c", CHANNEL_NA, DEFAULT_VARIANT);
        assert_eq!(find_series_by_key(conn, &na_key).unwrap(), Some(na_id));
        assert_eq!(
            find_series_by_key(conn, &series_key_of(&sid, "temperature_c", 1, "primary")).unwrap(),
            None
        );

        let rows = list_series(conn).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].series_id, na_id);
        assert_eq!(rows[0].series_key, na_key);
        assert_eq!(rows[0].system_id, sid.to_text());
        assert_eq!(rows[0].user_label.as_deref(), Some("Rack sensor"));
        assert_eq!(rows[1].series_id, count_id);
        assert_eq!(
            rows[1].series_key,
            series_key_of(&sid, "contact_state", 2, "count")
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn insert_and_resolve_device_by_hardware_id() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:00000000000000ab".into(),
                user_label: Some("炉1温度".into()),
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        let row = find_alive_by_hardware_id(conn, "ble:00000000000000ab")
            .unwrap()
            .unwrap();
        assert_eq!(row.system_id, sid);
        assert_eq!(row.kind, DeviceKind::Individual);
        Ok(())
    })
    .unwrap();
}

#[test]
fn duplicate_alive_hardware_id_is_rejected() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let nd = NewDevice {
            hardware_id: "i2c:0x60".into(),
            user_label: None,
            parent: None,
            kind: DeviceKind::Positional,
            initial_state: DeviceState::Active,
        };
        insert_device(conn, &nd).unwrap();
        assert!(matches!(
            insert_device(conn, &nd),
            Err(LedgerError::HardwareIdInUse(_))
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn ensure_series_is_idempotent_and_monotonic() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:cc".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        let s1 = ensure_series(
            conn,
            &sid,
            "temperature_c",
            CHANNEL_NA,
            DEFAULT_VARIANT,
            false,
            None,
        )
        .unwrap();
        let s2 = ensure_series(
            conn,
            &sid,
            "temperature_c",
            CHANNEL_NA,
            DEFAULT_VARIANT,
            false,
            None,
        )
        .unwrap();
        let s3 = ensure_series(conn, &sid, "voltage_mv", 0, DEFAULT_VARIANT, false, None).unwrap();
        assert_eq!(s1, s2);
        assert!(s3 > s1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn ensure_series_stores_quarantine_reason() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:qr".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        let id = ensure_series(
            conn,
            &sid,
            "custom.mystery",
            CHANNEL_NA,
            DEFAULT_VARIANT,
            true,
            Some("unknown_key"),
        )
        .unwrap();
        let meta = find_series_meta(conn, &sid, "custom.mystery", CHANNEL_NA, DEFAULT_VARIANT)
            .unwrap()
            .unwrap();
        assert_eq!(meta.series_id, id);
        assert!(meta.quarantined);
        assert_eq!(meta.quarantine_reason.as_deref(), Some("unknown_key"));
        assert_eq!(meta.range_min, None);
        Ok(())
    })
    .unwrap();
}

#[test]
fn series_calibration_review_defaults_to_zero() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:cal".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        let series_id = ensure_series(
            conn,
            &sid,
            "temperature_c",
            CHANNEL_NA,
            DEFAULT_VARIANT,
            false,
            None,
        )
        .unwrap();
        let calibration_review: i64 = conn
            .query_row(
                "SELECT calibration_review FROM series WHERE series_id = ?1",
                params![series_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(calibration_review, 0);
        Ok(())
    })
    .unwrap();
}

#[test]
fn series_exists_for_key_ignores_channel_and_variant() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:ex".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        assert!(!series_exists_for_key(conn, &sid, "temp_old").unwrap());
        ensure_series(conn, &sid, "temp_old", 2, "count", false, None).unwrap();
        assert!(series_exists_for_key(conn, &sid, "temp_old").unwrap());
        Ok(())
    })
    .unwrap();
}

#[test]
fn record_event_appends_to_ledger_events() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        record_event(
            conn,
            "registry_entry_enabled",
            None,
            r#"{"key":"temperature_c"}"#,
        )
        .unwrap();
        let (kind, detail): (String, String) = conn
            .query_row(
                "SELECT kind, detail FROM ledger_events ORDER BY event_id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "registry_entry_enabled");
        assert!(detail.contains("temperature_c"));
        Ok(())
    })
    .unwrap();
}

#[test]
fn release_series_quarantine_checked_clears_matching_channels_and_relabels_mismatches() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:rel".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        let a = ensure_series(
            conn,
            &sid,
            "temp_old",
            CHANNEL_NA,
            DEFAULT_VARIANT,
            true,
            Some("unknown_key"),
        )
        .unwrap();
        let bad = ensure_series(
            conn,
            &sid,
            "temp_old",
            3,
            DEFAULT_VARIANT,
            true,
            Some("unknown_key"),
        )
        .unwrap();
        ensure_series(
            conn,
            &sid,
            "other_key",
            CHANNEL_NA,
            DEFAULT_VARIANT,
            true,
            Some("undeclared_channel"),
        )
        .unwrap();
        let (released, mismatch) =
            release_series_quarantine_for_key_checked(conn, "temp_old", "unknown_key", &|ch| {
                ch == CHANNEL_NA
            })
            .unwrap();
        assert_eq!(released, vec![a]);
        assert_eq!(mismatch, vec![bad]);
        let meta = find_series_meta(conn, &sid, "temp_old", CHANNEL_NA, DEFAULT_VARIANT)
            .unwrap()
            .unwrap();
        assert!(!meta.quarantined);
        assert_eq!(meta.quarantine_reason, None);
        let bad_meta = find_series_meta(conn, &sid, "temp_old", 3, DEFAULT_VARIANT)
            .unwrap()
            .unwrap();
        assert!(bad_meta.quarantined);
        assert_eq!(
            bad_meta.quarantine_reason.as_deref(),
            Some("undeclared_channel")
        );
        // キーも理由も異なるseriesは対象外
        let other = find_series_meta(conn, &sid, "other_key", CHANNEL_NA, DEFAULT_VARIANT)
            .unwrap()
            .unwrap();
        assert!(other.quarantined);
        // 対象なしの冪等呼び出し
        let (released2, mismatch2) =
            release_series_quarantine_for_key_checked(conn, "temp_old", "unknown_key", &|_| true)
                .unwrap();
        assert!(released2.is_empty());
        assert!(mismatch2.is_empty());
        Ok(())
    })
    .unwrap();
}

#[test]
fn find_series_meta_returns_range_override() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:rng".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        ensure_series(
            conn,
            &sid,
            "temperature_c",
            CHANNEL_NA,
            DEFAULT_VARIANT,
            false,
            None,
        )
        .unwrap();
        // Wave 0にはseries値域の設定APIがない(R14=計画4)ため、直接SQLで個別上書きを模擬
        conn.execute(
            "UPDATE series SET range_min = -10.0, range_max = 50.0
                 WHERE system_id = ?1 AND measurement_key = 'temperature_c'",
            params![sid.as_bytes().to_vec()],
        )
        .unwrap();
        let meta = find_series_meta(conn, &sid, "temperature_c", CHANNEL_NA, DEFAULT_VARIANT)
            .unwrap()
            .unwrap();
        assert_eq!(meta.range_min, Some(-10.0));
        assert_eq!(meta.range_max, Some(50.0));
        Ok(())
    })
    .unwrap();
}

#[test]
fn sighting_then_approve_creates_quarantined_device() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        record_sighting(conn, "ble:ff", "bravepi-mainboard").unwrap();
        record_sighting(conn, "ble:ff", "bravepi-mainboard").unwrap();
        let sid =
            approve_sighting(conn, "ble:ff", Some("新センサー"), DeviceKind::Individual).unwrap();
        let row = find_alive_by_hardware_id(conn, "ble:ff").unwrap().unwrap();
        assert_eq!(row.system_id, sid);
        assert_eq!(row.state, DeviceState::Quarantined);
        activate_device(conn, &sid).unwrap();
        assert_eq!(
            find_alive_by_hardware_id(conn, "ble:ff")
                .unwrap()
                .unwrap()
                .state,
            DeviceState::Active
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn purge_sightings_drops_stale_and_keeps_fresh_rows() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        const DAY_MS: i64 = 24 * 60 * 60 * 1000;
        let now = 1_700_000_000_000_i64;
        conn.execute(
            "INSERT INTO sightings (hardware_id, source, first_seen, last_seen, observations)
                 VALUES (?1, 'adapter-a', ?2, ?2, 1)",
            params!["ble:stale", now - 31 * DAY_MS],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sightings (hardware_id, source, first_seen, last_seen, observations)
                 VALUES (?1, 'adapter-a', ?2, ?2, 1)",
            params!["ble:fresh", now - DAY_MS],
        )
        .unwrap();

        let purged = purge_sightings(conn, now).unwrap();

        assert_eq!(purged, 1);
        let remaining = list_sightings(conn).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].hardware_id, "ble:fresh");
        Ok(())
    })
    .unwrap();
}

#[test]
fn purge_sightings_evicts_oldest_rows_beyond_cap() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let now = 1_700_000_000_000_i64;
        let first_seen = now - SIGHTINGS_TTL_MS + 1;
        let mut insert = conn
            .prepare(
                "INSERT INTO sightings
                     (hardware_id, source, first_seen, last_seen, observations)
                     VALUES (?1, 'adapter-a', ?2, ?2, 1)",
            )
            .unwrap();
        for i in 0..(SIGHTINGS_CAP + 5) {
            insert
                .execute(params![format!("ble:{i:05}"), first_seen + i])
                .unwrap();
        }
        drop(insert);

        let purged = purge_sightings(conn, now).unwrap();

        assert_eq!(purged, 5);
        let remaining_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sightings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining_count, SIGHTINGS_CAP);
        for i in 0..5 {
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sightings WHERE hardware_id = ?1)",
                    params![format!("ble:{i:05}")],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(!exists, "oldest sighting ble:{i:05} should be evicted");
        }
        let newest_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sightings WHERE hardware_id = ?1)",
                params![format!("ble:{:05}", SIGHTINGS_CAP + 4)],
                |row| row.get(0),
            )
            .unwrap();
        assert!(newest_exists);
        Ok(())
    })
    .unwrap();
}

#[test]
fn purge_sightings_keeps_fresh_sighting_approval_flow_working() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let now = now_ms();
        record_sighting(conn, "ble:approval", "bravepi-mainboard").unwrap();

        let purged = purge_sightings(conn, now).unwrap();
        let sid = approve_sighting(
            conn,
            "ble:approval",
            Some("new sensor"),
            DeviceKind::Individual,
        )
        .unwrap();

        assert_eq!(purged, 0);
        let row = find_alive_by_hardware_id(conn, "ble:approval")
            .unwrap()
            .unwrap();
        assert_eq!(row.system_id, sid);
        assert_eq!(row.state, DeviceState::Quarantined);
        assert!(list_sightings(conn).unwrap().is_empty());
        Ok(())
    })
    .unwrap();
}

#[test]
fn purge_sightings_never_deletes_metadata_while_staged_rows_survive() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        conn.execute_batch(
            "CREATE TABLE staged_readings (
                    id INTEGER PRIMARY KEY,
                    hardware_id TEXT NOT NULL,
                    received_at INTEGER NOT NULL,
                    payload_json TEXT NOT NULL
                );",
        )?;
        record_sighting(conn, "ble:staged", "official-a").unwrap();
        conn.execute(
            "INSERT INTO staged_readings(hardware_id, received_at, payload_json)
                 VALUES('ble:staged', 0, '{}')",
            [],
        )?;
        conn.execute(
            "UPDATE sightings SET last_seen=0 WHERE hardware_id='ble:staged'",
            [],
        )?;

        purge_sightings(conn, SIGHTINGS_TTL_MS + 1).unwrap();
        assert_eq!(list_sightings(conn).unwrap().len(), 1);

        conn.execute(
            "DELETE FROM staged_readings WHERE hardware_id='ble:staged'",
            [],
        )?;
        purge_sightings(conn, SIGHTINGS_TTL_MS + 1).unwrap();
        assert!(list_sightings(conn).unwrap().is_empty());
        Ok(())
    })
    .unwrap();
}

#[test]
fn retired_hardware_id_becomes_reusable() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let hardware_id = "ble:retire01";
        let sid1 = insert_device(
            conn,
            &NewDevice {
                hardware_id: hardware_id.into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();

        retire_device(conn, &sid1).unwrap();

        // partial unique index はstate != 'retired'のみ対象のため、同一hardware_idの再登録が成功するはず
        let sid2 = insert_device(
            conn,
            &NewDevice {
                hardware_id: hardware_id.into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        assert_ne!(sid1, sid2);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM devices WHERE hardware_id = ?1",
                params![hardware_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2, "retired行と新規active行がDB上に共存するはず");
        Ok(())
    })
    .unwrap();
}

#[test]
fn retire_device_marks_tombstone_and_records_event() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let sid = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:tombstone".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();

        retire_device(conn, &sid).unwrap();

        let row = get_device(conn, &sid).unwrap().unwrap();
        assert_eq!(row.state, DeviceState::Retired);
        let retired_at: Option<i64> = conn
            .query_row(
                "SELECT retired_at FROM devices WHERE system_id = ?1",
                params![sid.as_bytes().to_vec()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(retired_at.is_some());
        assert!(
            find_alive_by_hardware_id(conn, "ble:tombstone")
                .unwrap()
                .is_none()
        );

        let (kind, event_sid): (String, Vec<u8>) = conn
            .query_row(
                "SELECT kind, system_id FROM ledger_events ORDER BY event_id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "device_retired");
        assert_eq!(event_sid, sid.as_bytes().to_vec());
        Ok(())
    })
    .unwrap();
}

#[test]
fn retire_device_returns_not_found_for_missing_or_already_retired_device() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let missing = SystemId::generate();
        assert!(matches!(
            retire_device(conn, &missing),
            Err(LedgerError::NotFound(_))
        ));

        let sid = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:already-retired".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        retire_device(conn, &sid).unwrap();
        assert!(matches!(
            retire_device(conn, &sid),
            Err(LedgerError::NotFound(_))
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn expire_quarantined_devices_activates_only_ttl_expired_rows_and_records_events() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let expired = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:expired".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Quarantined,
            },
        )
        .unwrap();
        let fresh = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:fresh".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Quarantined,
            },
        )
        .unwrap();
        let active = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:active".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        conn.execute(
            "UPDATE devices SET created_at = 0 WHERE system_id = ?1",
            params![expired.as_bytes().to_vec()],
        )
        .unwrap();
        conn.execute(
            "UPDATE devices SET created_at = ?1 WHERE system_id IN (?2, ?3)",
            params![
                i64::MAX / 2,
                fresh.as_bytes().to_vec(),
                active.as_bytes().to_vec()
            ],
        )
        .unwrap();

        let expired_ids = expire_quarantined_devices(conn, 1).unwrap();

        assert_eq!(expired_ids, vec![expired]);
        assert_eq!(
            get_device(conn, &expired).unwrap().unwrap().state,
            DeviceState::Active
        );
        assert_eq!(
            get_device(conn, &fresh).unwrap().unwrap().state,
            DeviceState::Quarantined
        );
        assert_eq!(
            get_device(conn, &active).unwrap().unwrap().state,
            DeviceState::Active
        );
        let (kind, event_sid): (String, Vec<u8>) = conn
            .query_row(
                "SELECT kind, system_id FROM ledger_events
                     WHERE kind = 'quarantine_expired'
                     ORDER BY event_id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "quarantine_expired");
        assert_eq!(event_sid, expired.as_bytes().to_vec());
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_hardware_retires_candidate_before_rebinding_and_records_event() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let target = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:old".into(),
                user_label: Some("target".into()),
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        let candidate = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:new".into(),
                user_label: Some("candidate".into()),
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Quarantined,
            },
        )
        .unwrap();
        ensure_series(
            conn,
            &target,
            "temperature_c",
            CHANNEL_NA,
            DEFAULT_VARIANT,
            false,
            None,
        )
        .unwrap();
        ensure_series(conn, &target, "voltage_mv", 0, DEFAULT_VARIANT, false, None).unwrap();

        let outcome = replace_hardware(conn, &target, "ble:new").unwrap();

        assert_eq!(outcome.replaced, target);
        assert_eq!(outcome.old_hardware_id, "ble:old");
        assert_eq!(outcome.retired_candidates, vec![candidate]);

        let target_row = get_device(conn, &target).unwrap().unwrap();
        assert_eq!(target_row.hardware_id, "ble:new");
        assert_eq!(target_row.state, DeviceState::Active);

        let candidate_row = get_device(conn, &candidate).unwrap().unwrap();
        assert_eq!(candidate_row.state, DeviceState::Retired);
        let (superseded_by, retired_at): (Vec<u8>, Option<i64>) = conn
            .query_row(
                "SELECT superseded_by, retired_at FROM devices WHERE system_id = ?1",
                params![candidate.as_bytes().to_vec()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(superseded_by, target.as_bytes().to_vec());
        assert!(retired_at.is_some());

        assert!(
            list_series_for_device(conn, &target)
                .unwrap()
                .iter()
                .all(|series| series.calibration_review)
        );

        let (kind, event_sid, detail): (String, Vec<u8>, String) = conn
            .query_row(
                "SELECT kind, system_id, detail FROM ledger_events ORDER BY event_id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(kind, "hardware_replaced");
        assert_eq!(event_sid, target.as_bytes().to_vec());
        let detail: serde_json::Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(detail["old_hw"], "ble:old");
        assert_eq!(detail["new_hw"], "ble:new");
        assert!(detail["at"].as_i64().is_some());
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_hardware_rejects_same_hardware_id_without_recording_event() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let target = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:same".into(),
                user_label: Some("target".into()),
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();

        assert!(matches!(
            replace_hardware(conn, &target, "ble:same"),
            Err(LedgerError::InvalidReplace(_))
        ));
        let row = get_device(conn, &target).unwrap().unwrap();
        assert_eq!(row.hardware_id, "ble:same");
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind = 'hardware_replaced'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events, 0);
        Ok(())
    })
    .unwrap();
}

#[test]
fn generation_counter_defaults_to_zero_and_bumps_monotonically() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        assert_eq!(current_generation(conn).unwrap(), 0);
        assert_eq!(bump_generation(conn).unwrap(), 1);
        assert_eq!(current_generation(conn).unwrap(), 1);
        assert_eq!(bump_generation(conn).unwrap(), 2);
        assert_eq!(current_generation(conn).unwrap(), 2);
        Ok(())
    })
    .unwrap();
}

#[test]
fn db_level_unique_constraint_rejects_alive_duplicate() {
    let db = test_db();
    db.with_conn_sync(|conn| {
            let hardware_id = "ble:dbunique01";
            insert_device(conn, &NewDevice {
                hardware_id: hardware_id.into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            }).unwrap();

            // アプリ層の事前チェック(find_alive_by_hardware_id)をバイパスし、
            // DB側のpartial unique index (idx_devices_hardware_alive) 単独で
            // alive重複を弾けることを検証する。
            let other_sid = SystemId::generate();
            let err = conn
                .execute(
                    "INSERT INTO devices (system_id, hardware_id, user_label, parent_system_id, kind, state, created_at)
                     VALUES (?1, ?2, NULL, NULL, ?3, 'active', ?4)",
                    params![
                        other_sid.as_bytes().to_vec(),
                        hardware_id,
                        DeviceKind::Individual.as_db(),
                        now_ms()
                    ],
                )
                .expect_err("DB-level partial unique indexがalive重複を拒否するはず");

            match err {
                rusqlite::Error::SqliteFailure(e, _) => {
                    assert_eq!(
                        e.code,
                        rusqlite::ErrorCode::ConstraintViolation,
                        "unique制約違反であるはず: {e:?}"
                    );
                }
                other => panic!("unique制約違反(SqliteFailure)を期待したが別のエラー: {other:?}"),
            }
            Ok(())
        })
        .unwrap();
}

#[test]
fn ledger_epoch_is_generated_once_and_stable() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let e1 = ledger_epoch(conn).unwrap();
        let e2 = ledger_epoch(conn).unwrap();
        assert_eq!(e1, e2);
        assert!(!e1.is_empty());
        Ok(())
    })
    .unwrap();
}

#[test]
fn edge_node_id_is_generated_once_and_stable() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let first = edge_node_id(conn).unwrap();
        let second = edge_node_id(conn).unwrap();
        assert_eq!(first, second);
        assert_eq!(uuid::Uuid::parse_str(&first).unwrap().get_version_num(), 7);
        Ok(())
    })
    .unwrap();
}

#[test]
fn load_edge_node_identity_returns_existing_values_without_writing() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let edge_node_id = edge_node_id(conn).unwrap();
        let ledger_epoch = ledger_epoch(conn).unwrap();
        let changes_before = conn.total_changes();

        let identity = load_edge_node_identity(conn).unwrap();

        assert_eq!(identity.edge_node_id, edge_node_id);
        assert_eq!(identity.ledger_epoch, ledger_epoch);
        assert_eq!(conn.total_changes(), changes_before);
        Ok(())
    })
    .unwrap();
}

#[test]
fn load_edge_node_identity_does_not_generate_missing_values() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let changes_before = conn.total_changes();

        let error = load_edge_node_identity(conn).unwrap_err();

        assert!(matches!(error, LedgerError::NotFound(_)));
        assert_eq!(conn.total_changes(), changes_before);
        assert_eq!(
            conn.query_row::<i64, _, _>(
                "SELECT COUNT(*) FROM ledger_meta WHERE key IN ('edge_node_id', 'epoch')",
                [],
                |row| row.get(0),
            )
            .unwrap(),
            0
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn legacy_gateway_identity_is_rejected() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        conn.execute(
            "INSERT INTO ledger_meta (key, value) VALUES ('gateway_identity', 'legacy-id')",
            [],
        )
        .unwrap();

        let error = edge_node_id(conn).unwrap_err();
        assert!(matches!(error, LedgerError::UnsupportedPreReleaseSchema));
        assert_eq!(
            error.to_string(),
            "unsupported pre-release Edge Node database; recreate the Edge Node database"
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM ledger_meta WHERE key = 'edge_node_id'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn renew_epoch_replaces_or_inserts_epoch_and_records_old_epoch() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let fresh = renew_epoch(conn).unwrap();
        assert_eq!(ledger_epoch(conn).unwrap(), fresh);
        let first_detail: String = conn
            .query_row(
                "SELECT detail FROM ledger_events WHERE kind = 'epoch_renewed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&first_detail).unwrap()["old_epoch"],
            serde_json::Value::Null
        );

        let renewed = renew_epoch(conn).unwrap();
        assert_ne!(renewed, fresh);
        assert_eq!(ledger_epoch(conn).unwrap(), renewed);
        let latest_detail: String = conn
            .query_row(
                "SELECT detail FROM ledger_events
                     WHERE kind = 'epoch_renewed' ORDER BY event_id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&latest_detail).unwrap()["old_epoch"],
            fresh
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn listing_apis_return_inserted_devices_series_sightings_and_events() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let parent = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:parent".into(),
                user_label: Some("parent".into()),
                parent: None,
                kind: DeviceKind::Positional,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        let child = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:child".into(),
                user_label: Some("child".into()),
                parent: Some(parent),
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        let retired = insert_device(
            conn,
            &NewDevice {
                hardware_id: "ble:retired".into(),
                user_label: None,
                parent: None,
                kind: DeviceKind::Individual,
                initial_state: DeviceState::Active,
            },
        )
        .unwrap();
        conn.execute(
            "UPDATE devices SET state = 'retired' WHERE system_id = ?1",
            params![retired.as_bytes().to_vec()],
        )
        .unwrap();

        let child_row = get_device(conn, &child).unwrap().unwrap();
        assert_eq!(child_row.hardware_id, "ble:child");
        assert_eq!(child_row.parent, Some(parent));
        let parent_row = get_device(conn, &parent).unwrap().unwrap();
        assert_eq!(parent_row.hardware_id, "ble:parent");
        assert_eq!(parent_row.parent, None);

        let alive = list_devices(conn, false).unwrap();
        assert_eq!(alive.len(), 2);
        assert!(alive.iter().all(|d| d.state != DeviceState::Retired));
        let all = list_devices(conn, true).unwrap();
        assert_eq!(all.len(), 3);
        let listed_parent = all.iter().find(|d| d.system_id == parent).unwrap();
        assert_eq!(listed_parent.parent, None);

        let series_id = ensure_series(
            conn,
            &child,
            "temperature_c",
            7,
            "backup",
            true,
            Some("unknown_key"),
        )
        .unwrap();
        conn.execute(
            "UPDATE series SET unit = 'Cel', range_min = -10.0, range_max = 50.0,
                    calibration_review = 1 WHERE series_id = ?1",
            params![series_id],
        )
        .unwrap();
        let series = list_series_for_device(conn, &child).unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].series_id, series_id);
        assert_eq!(series[0].system_id, child);
        assert_eq!(series[0].measurement_key, "temperature_c");
        assert_eq!(series[0].channel_index, 7);
        assert_eq!(series[0].variant, "backup");
        assert!(series[0].quarantined);
        assert_eq!(series[0].quarantine_reason.as_deref(), Some("unknown_key"));
        assert_eq!(series[0].value_semantics, "calibrated");
        assert_eq!(series[0].unit.as_deref(), Some("Cel"));
        assert_eq!(series[0].range_min, Some(-10.0));
        assert_eq!(series[0].range_max, Some(50.0));
        assert!(series[0].calibration_review);

        record_sighting(conn, "ble:seen", "adapter-a").unwrap();
        let sightings = list_sightings(conn).unwrap();
        assert_eq!(sightings.len(), 1);
        assert_eq!(sightings[0].hardware_id, "ble:seen");
        assert_eq!(sightings[0].source, "adapter-a");
        assert_eq!(sightings[0].observations, 1);

        record_event(conn, "manual", Some(&child), r#"{"ok":true}"#).unwrap();
        record_event(conn, "system_note", None, r#"{"ok":false}"#).unwrap();
        let events = list_recent_events(conn, 2).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "system_note");
        assert_eq!(events[0].system_id, None);
        assert_eq!(events[1].kind, "manual");
        assert_eq!(events[1].system_id, Some(child));
        Ok(())
    })
    .unwrap();
}

#[test]
fn invalid_system_id_blob_returns_error_instead_of_panicking() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        conn.execute(
            "INSERT INTO devices (system_id, hardware_id, kind, state, created_at)
                 VALUES (?1, 'ble:bad-system-id', 'individual', 'active', 1)",
            params![vec![1_u8, 2, 3]],
        )
        .unwrap();

        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| list_devices(conn, true)));

        assert!(result.is_ok(), "invalid blob length should not panic");
        assert!(result.unwrap().is_err());
        Ok(())
    })
    .unwrap();
}

#[test]
fn install_recovery_epoch_requires_the_exact_old_epoch_and_records_transition() {
    let db = test_db();
    db.with_conn_sync(|conn| {
        let old_epoch = ledger_epoch(conn).unwrap();
        let new_epoch = "01JRECOVERYNEW";

        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .unwrap();
        install_recovery_epoch(&tx, &old_epoch, new_epoch).unwrap();
        tx.commit().unwrap();
        assert_eq!(ledger_epoch(conn).unwrap(), new_epoch);

        let detail: String = conn
            .query_row(
                "SELECT detail FROM ledger_events
                 WHERE kind='epoch_recovered'
                 ORDER BY event_id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&detail).unwrap(),
            serde_json::json!({"old_epoch": old_epoch, "new_epoch": new_epoch})
        );
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .unwrap();
        assert!(install_recovery_epoch(&tx, "wrong-old", "another-new").is_err());
        tx.rollback().unwrap();
        assert_eq!(ledger_epoch(conn).unwrap(), new_epoch);
        Ok(())
    })
    .unwrap();
}
