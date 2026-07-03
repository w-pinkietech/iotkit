use std::process::{Command, Output};

use rusqlite::params;

fn gatewayctl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_iotkit-gatewayctl"))
}

fn all_migrations() -> Vec<iotkit_core_storage::Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.sort_by_key(|m| m.version);
    all
}

fn run(args: &[&str]) -> Output {
    gatewayctl().args(args).output().expect("run gatewayctl")
}

fn assert_success(output: Output) -> String {
    assert!(
        output.status.success(),
        "status: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn assert_failure(output: Output) -> String {
    assert!(
        !output.status.success(),
        "expected failure\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stderr).unwrap()
}

fn seed_replace_target(conn: &rusqlite::Connection) -> iotkit_core_ledger::SystemId {
    let sid = iotkit_core_ledger::insert_device(
        conn,
        &iotkit_core_ledger::NewDevice {
            hardware_id: "ble:old".into(),
            user_label: Some("Target Sensor".into()),
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
    .unwrap();
    iotkit_core_ledger::ensure_series(
        conn,
        &sid,
        "voltage_mv",
        0,
        iotkit_core_ledger::DEFAULT_VARIANT,
        false,
        None,
    )
    .unwrap();
    sid
}

fn stage_item(
    conn: &rusqlite::Connection,
    hardware_id: &str,
    measurement_key: &str,
    channel_index: Option<u16>,
) {
    let payload = match channel_index {
        Some(ch) => format!(
            r#"{{"measurement_key":"{measurement_key}","channel_index":{ch},"values":[1.0],"time_source":"gateway"}}"#
        ),
        None => format!(
            r#"{{"measurement_key":"{measurement_key}","values":[1.0],"time_source":"gateway"}}"#
        ),
    };
    iotkit_core_timeseries::insert_staged_reading(conn, hardware_id, 1000, &payload).unwrap();
}

fn prepare_replace_db() -> (tempfile::TempDir, std::path::PathBuf, String) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let sid = db
        .with_conn_sync(|conn| Ok(seed_replace_target(conn).to_text()))
        .unwrap();
    (dir, db_path, sid)
}

#[test]
fn missing_db_path_is_error_and_does_not_create_empty_db() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("missing.db");

    let output = run(&["--db", db_path.to_str().unwrap(), "device", "list"]);

    assert!(!output.status.success());
    assert!(!db_path.exists());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("database file does not exist"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn existing_empty_db_gets_gateway_migration_version_set() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    std::fs::File::create(&db_path).unwrap();

    assert_success(run(&["--db", db_path.to_str().unwrap(), "device", "list"]));

    let conn = rusqlite::Connection::open(db_path).unwrap();
    let versions: Vec<u32> = conn
        .prepare("SELECT version FROM _schema_version ORDER BY version")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(versions, vec![1, 3, 4, 5, 6, 7, 8, 9]);
}

#[test]
fn device_lifecycle_commands_round_trip_and_bump_generation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        iotkit_core_ledger::record_sighting(conn, "ble:cli", "cli-test").unwrap();
        Ok(())
    })
    .unwrap();
    let db_arg = db_path.to_str().unwrap();

    let sid = assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "approve",
        "ble:cli",
        "--label",
        "CLI Sensor",
        "--kind",
        "individual",
    ]))
    .trim()
    .to_string();
    assert!(!sid.is_empty());

    assert_success(run(&["--db", db_arg, "device", "activate", &sid]));

    let listed = assert_success(run(&["--db", db_arg, "device", "list"]));
    assert!(listed.contains("ble:cli"));
    assert!(listed.contains("active"));

    assert_success(run(&["--db", db_arg, "device", "retire", &sid, "--yes"]));

    let live = assert_success(run(&["--db", db_arg, "device", "list"]));
    assert!(!live.contains("ble:cli"));
    let all = assert_success(run(&["--db", db_arg, "device", "list", "--all"]));
    assert!(all.contains("ble:cli"));
    assert!(all.contains("retired"));

    db.with_conn_sync(|conn| {
        assert_eq!(iotkit_core_ledger::current_generation(conn).unwrap(), 3);
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_hardware_allows_exact_observed_profile_from_staged_readings() {
    let (_dir, db_path, sid) = prepare_replace_db();
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        stage_item(conn, "ble:new", "temperature_c", None);
        stage_item(conn, "ble:new", "voltage_mv", Some(0));
        Ok(())
    })
    .unwrap();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid,
        "--new-hardware-id",
        "ble:new",
        "--yes",
    ]));

    db.with_conn_sync(|conn| {
        let row = iotkit_core_ledger::get_device(
            conn,
            &iotkit_core_ledger::SystemId::from_text(&sid).unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(row.hardware_id, "ble:new");
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_hardware_normalizes_single_channel_zero_in_observed_profile() {
    let (_dir, db_path, sid) = prepare_replace_db();
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let catalog = iotkit_core_registry::standard_catalog();
        let temperature = catalog.find("temperature_c").unwrap();
        iotkit_core_registry::enable_entry(
            conn,
            temperature,
            &catalog.catalog_version,
            "test",
        )
        .unwrap();
        stage_item(conn, "ble:new", "temperature_c", Some(0));
        stage_item(conn, "ble:new", "voltage_mv", Some(0));
        Ok(())
    })
    .unwrap();

    assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "replace",
        &sid,
        "--new-hardware-id",
        "ble:new",
        "--yes",
    ]));
}

#[test]
fn replace_hardware_normalizes_unenabled_standard_single_channel_zero_in_observed_profile() {
    let (_dir, db_path, sid) = prepare_replace_db();
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        stage_item(conn, "ble:new", "temperature_c", Some(0));
        stage_item(conn, "ble:new", "voltage_mv", Some(0));
        Ok(())
    })
    .unwrap();

    assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "replace",
        &sid,
        "--new-hardware-id",
        "ble:new",
        "--yes",
    ]));
}

#[test]
fn replace_hardware_rejects_same_hardware_id() {
    let (_dir, db_path, sid) = prepare_replace_db();

    let stderr = assert_failure(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "replace",
        &sid,
        "--new-hardware-id",
        "ble:old",
        "--force",
        "--yes",
    ]));
    assert!(
        stderr.contains("invalid replace") || stderr.contains("same hardware_id"),
        "stderr did not explain same-hardware replace:\n{stderr}"
    );

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
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
fn replace_hardware_rejects_retired_target() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let sid = iotkit_core_ledger::SystemId::from_text(&sid_text).unwrap();
        iotkit_core_ledger::retire_device(conn, &sid).unwrap();
        Ok(())
    })
    .unwrap();

    let stderr = assert_failure(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));
    assert!(
        stderr.contains("non-retired device"),
        "stderr did not explain retired target:\n{stderr}"
    );
}

#[test]
fn replace_hardware_blocks_missing_extra_and_empty_observed_profiles_unless_forced() {
    for (case, staged) in [
        ("missing", vec![("temperature_c", None)]),
        (
            "extra",
            vec![
                ("temperature_c", None),
                ("voltage_mv", Some(0)),
                ("humidity_pct", None),
            ],
        ),
        ("empty", vec![]),
    ] {
        let (_dir, db_path, sid) = prepare_replace_db();
        let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            for (measurement_key, channel_index) in staged {
                stage_item(conn, "ble:new", measurement_key, channel_index);
            }
            Ok(())
        })
        .unwrap();

        let stderr = assert_failure(run(&[
            "--db",
            db_path.to_str().unwrap(),
            "device",
            "replace",
            &sid,
            "--new-hardware-id",
            "ble:new",
            "--yes",
        ]));
        assert!(
            stderr.contains("observed profile"),
            "{case} stderr did not explain profile mismatch:\n{stderr}"
        );
    }

    let (_dir, db_path, sid) = prepare_replace_db();
    let db_arg = db_path.to_str().unwrap();
    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));
}

#[test]
fn replace_hardware_uses_alive_candidate_series_as_observed_profile_and_retires_candidate() {
    let (_dir, db_path, sid) = prepare_replace_db();
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let candidate = db
        .with_conn_sync(|conn| {
            let candidate = iotkit_core_ledger::insert_device(
                conn,
                &iotkit_core_ledger::NewDevice {
                    hardware_id: "ble:new".into(),
                    user_label: None,
                    parent: None,
                    kind: iotkit_core_ledger::DeviceKind::Individual,
                    initial_state: iotkit_core_ledger::DeviceState::Quarantined,
                },
            )
            .unwrap();
            iotkit_core_ledger::ensure_series(
                conn,
                &candidate,
                "temperature_c",
                iotkit_core_ledger::CHANNEL_NA,
                iotkit_core_ledger::DEFAULT_VARIANT,
                false,
                None,
            )
            .unwrap();
            iotkit_core_ledger::ensure_series(
                conn,
                &candidate,
                "voltage_mv",
                0,
                iotkit_core_ledger::DEFAULT_VARIANT,
                false,
                None,
            )
            .unwrap();
            Ok(candidate)
        })
        .unwrap();

    assert_success(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "replace",
        &sid,
        "--new-hardware-id",
        "ble:new",
        "--yes",
    ]));

    db.with_conn_sync(|conn| {
        let candidate_row = iotkit_core_ledger::get_device(conn, &candidate)
            .unwrap()
            .unwrap();
        assert_eq!(
            candidate_row.state,
            iotkit_core_ledger::DeviceState::Retired
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_undo_restores_hardware_id_marks_since_range_and_records_event() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        stage_item(conn, "ble:new", "temperature_c", None);
        stage_item(conn, "ble:new", "voltage_mv", Some(0));
        Ok(())
    })
    .unwrap();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--yes",
    ]));

    db.with_conn_sync(|conn| {
        let sid = iotkit_core_ledger::SystemId::from_text(&sid_text).unwrap();
        let series = iotkit_core_ledger::list_series_for_device(conn, &sid).unwrap();
        let first = series[0].series_id;
        let second = series[1].series_id;
        conn.execute(
            "UPDATE ledger_events
             SET at = 1000, detail = '{\"old_hw\":\"ble:old\",\"new_hw\":\"ble:new\",\"at\":1000}'
             WHERE kind = 'hardware_replaced'",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO readings
                (seq, series_id, received_at, device_time, time_source, time_quality,
                 event_time, event_time_source, values_json, rssi, battery_pct, quarantined)
             VALUES
                (1, ?1, 500, 500, 'gateway', 'unsynced', 500, 'received_at', '[1.0]', NULL, NULL, 0),
                (2, ?1, 1500, 100, 'device_ntp', 'unsynced', 100, 'device', '[2.0]', NULL, NULL, 0),
                (3, ?2, 1600, 200, 'device_ntp', 'unsynced', 200, 'device', '[3.0]', NULL, NULL, 0)",
            params![first, second],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:old",
    ]));

    db.with_conn_sync(|conn| {
        let sid = iotkit_core_ledger::SystemId::from_text(&sid_text).unwrap();
        let row = iotkit_core_ledger::get_device(conn, &sid).unwrap().unwrap();
        assert_eq!(row.hardware_id, "ble:old");
        let rows: Vec<(i64, i64)> = conn
            .prepare("SELECT seq, quarantined FROM readings ORDER BY seq")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, vec![(1, 0), (2, 1), (3, 1)]);
        let (kind, detail): (String, String) = conn
            .query_row(
                "SELECT kind, detail FROM ledger_events ORDER BY event_id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "hardware_replace_undone");
        assert!(detail.contains("\"rows\":2"));
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_undo_rejects_old_hardware_id_mismatch() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));

    let stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:wrong",
    ]));
    assert!(
        stderr.contains("old_hw") || stderr.contains("old hardware"),
        "stderr did not explain old_hw mismatch:\n{stderr}"
    );

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let sid = iotkit_core_ledger::SystemId::from_text(&sid_text).unwrap();
        let row = iotkit_core_ledger::get_device(conn, &sid).unwrap().unwrap();
        assert_eq!(row.hardware_id, "ble:new");
        let undone: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind = 'hardware_replace_undone'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(undone, 0);
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_undo_rejects_old_hardware_id_used_by_other_alive_device() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        iotkit_core_ledger::insert_device(
            conn,
            &iotkit_core_ledger::NewDevice {
                hardware_id: "ble:old".into(),
                user_label: Some("Other".into()),
                parent: None,
                kind: iotkit_core_ledger::DeviceKind::Individual,
                initial_state: iotkit_core_ledger::DeviceState::Active,
            },
        )
        .unwrap();
        Ok(())
    })
    .unwrap();

    let stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:old",
    ]));
    assert!(
        stderr.contains("hardware_id already in use"),
        "stderr did not explain hardware conflict:\n{stderr}"
    );
}

#[test]
fn replace_undo_without_since_requires_matching_replace_event() {
    let (_dir, db_path, sid_text) = prepare_replace_db();

    let stderr = assert_failure(run(&[
        "--db",
        db_path.to_str().unwrap(),
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:previous",
    ]));
    assert!(
        stderr.contains("no hardware_replaced event"),
        "stderr did not explain missing replace event:\n{stderr}"
    );
}

#[test]
fn replace_undo_rejects_future_since_even_when_event_matches() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));

    let stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:old",
        "--since",
        "9223372036854775807",
    ]));
    assert!(
        stderr.contains("--since") && stderr.contains("future"),
        "stderr did not explain future --since:\n{stderr}"
    );
}

#[test]
fn replace_undo_rejects_since_after_replace_event() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE ledger_events
             SET at = 1000, detail = '{\"old_hw\":\"ble:old\",\"new_hw\":\"ble:new\",\"at\":1000}'
             WHERE kind = 'hardware_replaced'",
            [],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();

    let stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:old",
        "--since",
        "1500",
    ]));
    assert!(
        stderr.contains("--since") && stderr.contains("replace event"),
        "stderr did not explain --since lower bound:\n{stderr}"
    );
}

#[test]
fn replace_undo_allows_since_before_replace_event() {
    let (_dir, db_path, sid_text) = prepare_replace_db();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace",
        &sid_text,
        "--new-hardware-id",
        "ble:new",
        "--force",
        "--yes",
    ]));

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE ledger_events
             SET at = 1000, detail = '{\"old_hw\":\"ble:old\",\"new_hw\":\"ble:new\",\"at\":1000}'
             WHERE kind = 'hardware_replaced'",
            [],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:old",
        "--since",
        "500",
    ]));
}
