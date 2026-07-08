use std::io::Write;
use std::process::{Command, Output, Stdio};

use rusqlite::{params, types::ValueRef};
use serde_json::{Map, Value};

fn gatewayctl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_iotkit-gatewayctl"))
}

fn all_migrations() -> Vec<iotkit_core_storage::Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    all.sort_by_key(|m| m.version);
    all
}

fn run(args: &[&str]) -> Output {
    gatewayctl().args(args).output().expect("run gatewayctl")
}

fn run_with_stdin(args: &[&str], stdin: &str) -> Output {
    let mut child = gatewayctl()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn gatewayctl");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().expect("run gatewayctl")
}

fn run_in_dir_without_db_env(args: &[&str], cwd: &std::path::Path) -> Output {
    gatewayctl()
        .args(args)
        .current_dir(cwd)
        .env_remove("IOTKIT_DB_PATH")
        .output()
        .expect("run gatewayctl")
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

fn json_rows(conn: &rusqlite::Connection, table: &str, order_by: &str) -> Vec<Value> {
    let mut stmt = conn
        .prepare(&format!("SELECT * FROM {table} ORDER BY {order_by}"))
        .unwrap();
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    stmt.query_map([], |row| {
        let mut object = Map::new();
        for (idx, name) in names.iter().enumerate() {
            let value = match row.get_ref(idx)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(v) => Value::from(v),
                ValueRef::Real(v) => Value::from(v),
                ValueRef::Text(v) => Value::from(String::from_utf8_lossy(v).into_owned()),
                ValueRef::Blob(v) => {
                    Value::from(v.iter().map(|b| format!("{b:02x}")).collect::<String>())
                }
            };
            object.insert(name.clone(), value);
        }
        Ok(Value::Object(object))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

fn snapshot_sections(conn: &rusqlite::Connection) -> Map<String, Value> {
    [
        ("devices", "created_at, hardware_id"),
        ("series", "series_id"),
        ("registry_entries", "measurement_key"),
        ("registry_aliases", "alias"),
        ("legacy_sensor_type_map", "sensor_type"),
    ]
    .into_iter()
    .map(|(table, order)| {
        (
            table.to_string(),
            Value::Array(json_rows(conn, table, order)),
        )
    })
    .collect()
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

fn seed_archive_target(conn: &rusqlite::Connection) {
    iotkit_core_publish::store::target_insert(
        conn,
        &iotkit_core_publish::store::TargetRow {
            target_id: "archive".into(),
            endpoint_url: "https://archive.example/publish".into(),
            credential_token: "token".into(),
            archive_responsible: true,
            schema_version: 1,
            cursor_epoch: None,
            cursor_pub_seq: 0,
        },
        1_000,
    )
    .unwrap();
}

fn token_id_from_list(output: &str) -> String {
    output
        .split_whitespace()
        .find(|part| part.starts_with("tok_"))
        .expect("token id in list output")
        .to_string()
}

fn token_id_from_labeled_stderr(output: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(output);
    stderr
        .lines()
        .find_map(|line| line.strip_prefix("token_id: "))
        .expect("labeled token id in stderr")
        .to_string()
}

#[test]
fn token_issue_list_and_revoke_do_not_expose_token_material() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let db_arg = db_path.to_str().unwrap();

    let issued = run(&[
        "--db",
        db_arg,
        "token",
        "issue",
        "--name",
        "routine-human",
        "--kind",
        "human",
        "--tier",
        "routine",
    ]);
    assert!(
        issued.status.success(),
        "status: {:?}\nstdout:\n{}\nstderr:\n{}",
        issued.status.code(),
        String::from_utf8_lossy(&issued.stdout),
        String::from_utf8_lossy(&issued.stderr)
    );
    let plaintext = String::from_utf8(issued.stdout).unwrap().trim().to_string();
    assert!(plaintext.starts_with("iko_"), "stdout:\n{plaintext}");
    let issued_token_id = token_id_from_labeled_stderr(&issued.stderr);

    let listed = assert_success(run(&["--db", db_arg, "token", "list"]));
    let token_id = token_id_from_list(&listed);
    assert_eq!(issued_token_id, token_id);
    assert!(listed.contains("routine-human"), "stdout:\n{listed}");
    assert!(
        !listed.contains(&plaintext),
        "stdout leaked plaintext:\n{listed}"
    );
    assert!(
        !listed.contains("token_hash"),
        "stdout leaked hash header:\n{listed}"
    );
    assert!(
        !listed.contains("$argon2"),
        "stdout leaked hash material:\n{listed}"
    );

    assert_success(run(&["--db", db_arg, "token", "revoke", "--id", &token_id]));

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        assert!(
            iotkit_core_ops::authenticate(conn, &plaintext, i64::MAX)
                .unwrap()
                .is_none()
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn token_issue_rejects_ai_daily() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let db_arg = db_path.to_str().unwrap();

    let stderr = assert_failure(run(&[
        "--db", db_arg, "token", "issue", "--name", "ai-daily", "--kind", "ai", "--tier", "daily",
    ]));

    assert!(
        stderr.contains("ai token tier ceiling cannot exceed routine")
            || stderr.contains("validation"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn passphrase_reset_validates_strength_and_replaces_existing_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let db_arg = db_path.to_str().unwrap();

    let empty = assert_failure(run_with_stdin(
        &["--db", db_arg, "passphrase", "reset"],
        "\n\n",
    ));
    assert!(empty.contains("at least 8"), "stderr:\n{empty}");

    let short_output = run_with_stdin(&["--db", db_arg, "passphrase", "reset"], "1234567\n");
    assert!(
        !String::from_utf8_lossy(&short_output.stdout).contains("confirm passphrase"),
        "stdout:\n{}",
        String::from_utf8_lossy(&short_output.stdout)
    );
    let short = assert_failure(short_output);
    assert!(short.contains("at least 8"), "stderr:\n{short}");

    assert_success(run_with_stdin(
        &["--db", db_arg, "passphrase", "reset"],
        "old-pass\nold-pass\n",
    ));
    assert_success(run_with_stdin(
        &["--db", db_arg, "passphrase", "reset"],
        "new-pass\nnew-pass\n",
    ));

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let hash = iotkit_core_ops::load_passphrase_hash(conn)
            .unwrap()
            .unwrap();
        assert!(!iotkit_core_ops::verify_passphrase(&hash, "old-pass"));
        assert!(iotkit_core_ops::verify_passphrase(&hash, "new-pass"));
        Ok(())
    })
    .unwrap();
}

#[test]
fn passphrase_reset_rejects_mismatched_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let db_arg = db_path.to_str().unwrap();

    let stderr = assert_failure(run_with_stdin(
        &["--db", db_arg, "passphrase", "reset"],
        "first-pass\nsecond-pass\n",
    ));

    assert!(
        stderr.contains("passphrases do not match"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn fingerprint_reads_cert_next_to_db_and_reports_missing_material() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let db_arg = db_path.to_str().unwrap();

    let missing = run(&["--db", db_arg, "fingerprint"]);
    assert!(
        !missing.status.success(),
        "expected missing fingerprint material to fail\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&missing.stdout),
        String::from_utf8_lossy(&missing.stderr)
    );
    assert!(
        missing.stdout.is_empty(),
        "stdout should be empty when fingerprint is unavailable:\n{}",
        String::from_utf8_lossy(&missing.stdout)
    );
    let missing_stderr = String::from_utf8(missing.stderr).unwrap();
    assert!(
        missing_stderr.contains("未生成") && missing_stderr.contains("gateway 未起動"),
        "stderr:\n{missing_stderr}"
    );

    let tls_dir = dir.path().join("tls");
    std::fs::create_dir(&tls_dir).unwrap();
    let cert_pem = "-----BEGIN CERTIFICATE-----\nAQIDBAU=\n-----END CERTIFICATE-----\n";
    std::fs::write(tls_dir.join("cert.pem"), cert_pem).unwrap();
    let expected = iotkit_core_ops::fingerprint_of_pem(cert_pem).unwrap();

    let out = assert_success(run(&["--db", db_arg, "fingerprint"]));

    assert_eq!(out.trim(), expected);
}

#[test]
fn target_add_is_rejected_in_setup_mode_and_allowed_after_passphrase_reset() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let db_arg = db_path.to_str().unwrap();

    let setup_stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "target",
        "add",
        "https://archive.example/publish",
        "token",
    ]));
    assert!(
        setup_stderr.contains("setupモード中は出口target登録不可"),
        "stderr:\n{setup_stderr}"
    );

    assert_success(run_with_stdin(
        &["--db", db_arg, "passphrase", "reset"],
        "admin-pass\nadmin-pass\n",
    ));

    let post_reset_stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "target",
        "add",
        "http://archive.example/publish",
        "token",
    ]));
    assert!(
        !post_reset_stderr.contains("setupモード中は出口target登録不可"),
        "stderr:\n{post_reset_stderr}"
    );
}

#[test]
fn health_command_reads_default_health_json_next_to_db_and_marks_stale() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let health_path = dir.path().join("health.json");
    let stale_written_at = 1_i64;
    std::fs::write(
        &health_path,
        format!(
            r#"{{
                "schema":1,
                "written_at":{stale_written_at},
                "epoch":"epoch-1",
                "uptime_s":12,
                "collector_alive":true,
                "adapters":[],
                "publish":[{{"target_id":"archive","cursor_pub_seq":7,"backlog":3,"last_push_at":1234,"last_error":null}}],
                "db":{{"size_bytes":10,"disk_available_bytes":20,"watermark_exceeded":false}},
                "retention":{{"days":90,"last_purge_at":null,"last_purged_rows":0}}
            }}"#
        ),
    )
    .unwrap();

    let out = assert_success(run(&["--db", db_path.to_str().unwrap(), "health"]));

    assert!(out.contains("STALE (daemon down?)"), "stdout:\n{out}");
    assert!(out.contains("epoch-1"), "stdout:\n{out}");
    assert!(out.contains("collector_alive=true"), "stdout:\n{out}");
    assert!(
        out.contains("publish target=archive cursor=7 backlog=3 last_push_at=1234 last_error=-"),
        "stdout:\n{out}"
    );
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
fn mutate_command_without_db_argument_or_env_is_error() {
    let dir = tempfile::tempdir().unwrap();
    let fallback_path = dir.path().join("iotkit.db");

    let output =
        run_in_dir_without_db_env(&["device", "add", "--hardware-id", "ble:no-db"], dir.path());

    assert!(!output.status.success());
    assert!(!fallback_path.exists());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no database specified"),
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
    assert_eq!(versions, vec![1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
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
fn registry_and_series_commands_round_trip_and_bump_generation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    let sid = db
        .with_conn_sync(|conn| {
            let sid = iotkit_core_ledger::insert_device(
                conn,
                &iotkit_core_ledger::NewDevice {
                    hardware_id: "ble:series".into(),
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
                "distance_mm",
                iotkit_core_ledger::CHANNEL_NA,
                iotkit_core_ledger::DEFAULT_VARIANT,
                false,
                None,
            )
            .unwrap();
            Ok(sid.to_text())
        })
        .unwrap();
    let db_arg = db_path.to_str().unwrap();

    assert_success(run(&["--db", db_arg, "registry", "enable", "distance_mm"]));
    let entries = assert_success(run(&["--db", db_arg, "registry", "list"]));
    assert!(entries.contains("distance_mm"));
    assert!(entries.contains("single"));

    assert_success(run(&[
        "--db",
        db_arg,
        "registry",
        "alias",
        "range_mm",
        "distance_mm",
    ]));
    let aliases = assert_success(run(&["--db", db_arg, "registry", "list", "--aliases"]));
    assert!(aliases.contains("range_mm"));
    assert!(aliases.contains("distance_mm"));

    let series = assert_success(run(&["--db", db_arg, "series", "list", &sid]));
    assert!(series.contains("distance_mm"));
    assert!(series.contains("-1"));

    db.with_conn_sync(|conn| {
        assert_eq!(iotkit_core_ledger::current_generation(conn).unwrap(), 2);
        Ok(())
    })
    .unwrap();
}

#[test]
fn release_rejected_while_archive_target_registered_without_override() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("iotkit.db");
    let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let catalog = iotkit_core_registry::standard_catalog();
        let entry = catalog.find("temperature_c").unwrap();
        iotkit_core_registry::enable_entry(conn, entry, &catalog.catalog_version, "test").unwrap();
        let sid = iotkit_core_ledger::insert_device(
            conn,
            &iotkit_core_ledger::NewDevice {
                hardware_id: "ble:quarantined-alias".into(),
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
            "temp_alias",
            iotkit_core_ledger::CHANNEL_NA,
            iotkit_core_ledger::DEFAULT_VARIANT,
            true,
            Some("unknown_key"),
        )
        .unwrap();
        seed_archive_target(conn);
        Ok(())
    })
    .unwrap();
    let db_arg = db_path.to_str().unwrap();

    let stderr = assert_failure(run(&[
        "--db",
        db_arg,
        "registry",
        "alias",
        "temp_alias",
        "temperature_c",
    ]));
    assert!(
        stderr.contains("refused") && stderr.contains("--release-abandon-past"),
        "stderr did not explain archive custody refusal:\n{stderr}"
    );
    db.with_conn_sync(|conn| {
        let quarantined: i64 = conn
            .query_row(
                "SELECT quarantined FROM series WHERE measurement_key = 'temp_alias'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let aliases: i64 = conn
            .query_row("SELECT COUNT(*) FROM registry_aliases", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(quarantined, 1);
        assert_eq!(aliases, 0);
        Ok(())
    })
    .unwrap();

    assert_success(run(&[
        "--db",
        db_arg,
        "registry",
        "alias",
        "temp_alias",
        "temperature_c",
        "--release-abandon-past",
    ]));
    db.with_conn_sync(|conn| {
        let quarantined: i64 = conn
            .query_row(
                "SELECT quarantined FROM series WHERE measurement_key = 'temp_alias'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let event_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events
                 WHERE kind = 'quarantine_release_abandon_past'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(quarantined, 0);
        assert_eq!(event_count, 1);
        Ok(())
    })
    .unwrap();

    let no_archive_dir = tempfile::tempdir().unwrap();
    let no_archive_path = no_archive_dir.path().join("iotkit.db");
    let no_archive_db = iotkit_core_storage::init_db(&no_archive_path, &all_migrations()).unwrap();
    no_archive_db
        .with_conn_sync(|conn| {
            let catalog = iotkit_core_registry::standard_catalog();
            let entry = catalog.find("temperature_c").unwrap();
            iotkit_core_registry::enable_entry(conn, entry, &catalog.catalog_version, "test")
                .unwrap();
            let sid = iotkit_core_ledger::insert_device(
                conn,
                &iotkit_core_ledger::NewDevice {
                    hardware_id: "ble:no-archive-alias".into(),
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
                "temp_alias",
                iotkit_core_ledger::CHANNEL_NA,
                iotkit_core_ledger::DEFAULT_VARIANT,
                true,
                Some("unknown_key"),
            )
            .unwrap();
            Ok(())
        })
        .unwrap();

    assert_success(run(&[
        "--db",
        no_archive_path.to_str().unwrap(),
        "registry",
        "alias",
        "temp_alias",
        "temperature_c",
    ]));
    no_archive_db
        .with_conn_sync(|conn| {
            let quarantined: i64 = conn
                .query_row(
                    "SELECT quarantined FROM series WHERE measurement_key = 'temp_alias'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(quarantined, 0);
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
        iotkit_core_registry::enable_entry(conn, temperature, &catalog.catalog_version, "test")
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
fn replace_undo_rejected_while_archive_target_registered_without_abandon() {
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
        seed_archive_target(conn);
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
        stderr.contains("refused") && stderr.contains("--abandon-custody"),
        "stderr did not explain archive custody refusal:\n{stderr}"
    );
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

    assert_success(run(&[
        "--db",
        db_arg,
        "device",
        "replace-undo",
        &sid_text,
        "--old-hardware-id",
        "ble:old",
        "--abandon-custody",
    ]));
    db.with_conn_sync(|conn| {
        let sid = iotkit_core_ledger::SystemId::from_text(&sid_text).unwrap();
        let row = iotkit_core_ledger::get_device(conn, &sid).unwrap().unwrap();
        assert_eq!(row.hardware_id, "ble:old");
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_undo_prunes_outbox_for_retroactively_quarantined_rows_same_tx() {
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
    let (before_seq, in_range_seq, second_in_range_seq, outbox_annotation_seq) = db
        .with_conn_sync(|conn| {
            let sid = iotkit_core_ledger::SystemId::from_text(&sid_text).unwrap();
            let series = iotkit_core_ledger::list_series_for_device(conn, &sid).unwrap();
            let first_series = series[0].series_id;
            let second_series = series[1].series_id;
            conn.execute(
                "UPDATE ledger_events
                 SET at = 1000, detail = '{\"old_hw\":\"ble:old\",\"new_hw\":\"ble:new\",\"at\":1000}'
                 WHERE kind = 'hardware_replaced'",
                [],
            )
            .unwrap();
            let before_seq = iotkit_core_timeseries::insert_reading_v3(
                conn,
                &iotkit_core_timeseries::NewReading {
                    series_id: first_series,
                    received_at_ms: 500,
                    device_time_ms: None,
                    time_source: "gateway".into(),
                    values: vec![1.0],
                    rssi: None,
                    battery_pct: None,
                    quarantined: false,
                },
            )
            .unwrap();
            let in_range_seq = iotkit_core_timeseries::insert_reading_v3(
                conn,
                &iotkit_core_timeseries::NewReading {
                    series_id: first_series,
                    received_at_ms: 1_500,
                    device_time_ms: None,
                    time_source: "gateway".into(),
                    values: vec![2.0],
                    rssi: None,
                    battery_pct: None,
                    quarantined: false,
                },
            )
            .unwrap();
            let second_in_range_seq = iotkit_core_timeseries::insert_reading_v3(
                conn,
                &iotkit_core_timeseries::NewReading {
                    series_id: second_series,
                    received_at_ms: 1_600,
                    device_time_ms: None,
                    time_source: "gateway".into(),
                    values: vec![3.0],
                    rssi: None,
                    battery_pct: None,
                    quarantined: false,
                },
            )
            .unwrap();
            let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
            iotkit_core_publish::store::enqueue_measurement(conn, &epoch, before_seq, 501)
                .unwrap();
            iotkit_core_publish::store::enqueue_measurement(conn, &epoch, in_range_seq, 1_501)
                .unwrap();
            iotkit_core_publish::store::enqueue_measurement(
                conn,
                &epoch,
                second_in_range_seq,
                1_601,
            )
                .unwrap();
            let outbox_annotation_seq =
                iotkit_core_publish::store::enqueue_annotation(conn, &epoch, "test", "{}", 1_700)
                    .unwrap()
                    .unwrap();
            Ok((
                before_seq,
                in_range_seq,
                second_in_range_seq,
                outbox_annotation_seq,
            ))
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
        let rows: Vec<(i64, i64)> = conn
            .prepare("SELECT seq, quarantined FROM readings ORDER BY seq")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![(before_seq, 0), (in_range_seq, 1), (second_in_range_seq, 1)]
        );
        let outbox: Vec<(String, Option<i64>)> = conn
            .prepare("SELECT kind, reading_seq FROM publication_log ORDER BY pub_seq")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            outbox,
            vec![
                ("measurement".to_string(), Some(before_seq)),
                ("annotation".to_string(), None)
            ]
        );
        let annotation_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM publication_log WHERE pub_seq = ?1",
                params![outbox_annotation_seq],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(annotation_exists, 1);
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

#[test]
fn snapshot_export_restore_round_trips_full_columns_and_renews_epoch() {
    let dir = tempfile::tempdir().unwrap();
    let source_db_path = dir.path().join("source.db");
    let restore_db_path = dir.path().join("restore.db");
    let snapshot_path = dir.path().join("snapshot.json");
    let db = iotkit_core_storage::init_db(&source_db_path, &all_migrations()).unwrap();
    let source_epoch = db
        .with_conn_sync(|conn| {
            let parent = iotkit_core_ledger::insert_device(
                conn,
                &iotkit_core_ledger::NewDevice {
                    hardware_id: "ble:parent".into(),
                    user_label: Some("Parent".into()),
                    parent: None,
                    kind: iotkit_core_ledger::DeviceKind::Positional,
                    initial_state: iotkit_core_ledger::DeviceState::Active,
                },
            )
            .unwrap();
            let target = iotkit_core_ledger::insert_device(
                conn,
                &iotkit_core_ledger::NewDevice {
                    hardware_id: "ble:target".into(),
                    user_label: Some("Target".into()),
                    parent: Some(parent),
                    kind: iotkit_core_ledger::DeviceKind::Individual,
                    initial_state: iotkit_core_ledger::DeviceState::Active,
                },
            )
            .unwrap();
            let candidate = iotkit_core_ledger::insert_device(
                conn,
                &iotkit_core_ledger::NewDevice {
                    hardware_id: "ble:new".into(),
                    user_label: Some("Retired Candidate".into()),
                    parent: Some(parent),
                    kind: iotkit_core_ledger::DeviceKind::Individual,
                    initial_state: iotkit_core_ledger::DeviceState::Quarantined,
                },
            )
            .unwrap();
            for sid in [target, candidate] {
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
                    true,
                    Some("unknown_key"),
                )
                .unwrap();
            }
            iotkit_core_ledger::replace_hardware(conn, &target, "ble:new").unwrap();
            conn.execute(
                "UPDATE series
                 SET unit = 'degC', range_min = -10.5, range_max = 85.25, legacy_sensor_type = 42,
                     calibration_review = 1
                 WHERE measurement_key = 'temperature_c'",
                [],
            )
            .unwrap();
            let catalog = iotkit_core_registry::standard_catalog();
            let temperature = catalog.find("temperature_c").unwrap();
            iotkit_core_registry::enable_entry(
                conn,
                temperature,
                &catalog.catalog_version,
                "snapshot-test",
            )
            .unwrap();
            iotkit_core_registry::define_alias(
                conn,
                "temp_old",
                "temperature_c",
                iotkit_core_registry::AliasKind::SiteMapping,
            )
            .unwrap();
            conn.execute(
                "INSERT INTO legacy_sensor_type_map (sensor_type, measurement_key, created_at)
                 VALUES (42, 'temperature_c', 123456)",
                [],
            )
            .unwrap();
            Ok(iotkit_core_ledger::ledger_epoch(conn).unwrap())
        })
        .unwrap();

    let source_sections = db
        .with_conn_sync(|conn| Ok(snapshot_sections(conn)))
        .unwrap();

    assert_success(run(&[
        "--db",
        source_db_path.to_str().unwrap(),
        "snapshot",
        "export",
        snapshot_path.to_str().unwrap(),
    ]));
    let snapshot: Value = serde_json::from_slice(&std::fs::read(&snapshot_path).unwrap()).unwrap();
    assert_eq!(snapshot["manifest"]["format_version"], 1);
    assert_eq!(snapshot["manifest"]["epoch"], source_epoch);
    assert_eq!(
        snapshot["manifest"]["sections"],
        serde_json::json!([
            "devices",
            "series",
            "registry_entries",
            "registry_aliases",
            "legacy_sensor_type_map"
        ])
    );
    assert!(snapshot["secrets"].is_null());
    assert!(snapshot["calibration"].is_null());
    assert!(snapshot["desired_config"].is_null());
    assert!(
        snapshot["devices"][0]["system_id"]
            .as_str()
            .unwrap()
            .contains('-')
    );

    assert_success(run(&[
        "snapshot",
        "restore",
        snapshot_path.to_str().unwrap(),
        "--db",
        restore_db_path.to_str().unwrap(),
        "--create",
        "--yes",
    ]));

    let restored_db = iotkit_core_storage::init_db(&restore_db_path, &all_migrations()).unwrap();
    restored_db
        .with_conn_sync(|conn| {
            assert_eq!(snapshot_sections(conn), source_sections);
            let restored_epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
            assert_ne!(restored_epoch, source_epoch);
            let (kind, detail): (String, String) = conn
                .query_row(
                    "SELECT kind, detail FROM ledger_events ORDER BY event_id DESC LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(kind, "epoch_renewed");
            let detail: Value = serde_json::from_str(&detail).unwrap();
            assert!(detail["old_epoch"].is_null());
            let blob_columns: Vec<(String, String)> = conn
                .prepare(
                    "SELECT 'devices.system_id', typeof(system_id) FROM devices
                     UNION ALL SELECT 'devices.parent_system_id', typeof(parent_system_id)
                         FROM devices WHERE parent_system_id IS NOT NULL
                     UNION ALL SELECT 'devices.superseded_by', typeof(superseded_by)
                         FROM devices WHERE superseded_by IS NOT NULL
                     UNION ALL SELECT 'series.system_id', typeof(system_id) FROM series",
                )
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert!(!blob_columns.is_empty());
            assert!(
                blob_columns.iter().all(|(_, typ)| typ == "blob"),
                "{blob_columns:?}"
            );
            let broken_refs: i64 = conn
                .query_row(
                    "SELECT
                        (SELECT COUNT(*) FROM devices d
                         WHERE d.parent_system_id IS NOT NULL
                           AND NOT EXISTS (SELECT 1 FROM devices p WHERE p.system_id = d.parent_system_id))
                      + (SELECT COUNT(*) FROM devices d
                         WHERE d.superseded_by IS NOT NULL
                           AND NOT EXISTS (SELECT 1 FROM devices s WHERE s.system_id = d.superseded_by))
                      + (SELECT COUNT(*) FROM series s
                         WHERE NOT EXISTS (SELECT 1 FROM devices d WHERE d.system_id = s.system_id))",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(broken_refs, 0);
            let next_series_id = conn
                .execute(
                    "INSERT INTO series
                        (system_id, measurement_key, channel_index, variant, quarantined,
                         value_semantics, created_at)
                     SELECT system_id, 'after_restore_key', -1, 'primary', 0, 'calibrated', 999999
                     FROM devices ORDER BY created_at LIMIT 1",
                    [],
                )
                .unwrap();
            assert_eq!(next_series_id, 1);
            let (max_existing, inserted): (i64, i64) = conn
                .query_row(
                    "SELECT
                        (SELECT MAX(series_id) FROM series WHERE measurement_key != 'after_restore_key'),
                        (SELECT series_id FROM series WHERE measurement_key = 'after_restore_key')",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(inserted, max_existing + 1);
            Ok(())
        })
        .unwrap();
}

#[test]
fn snapshot_restore_rejects_non_empty_device_table() {
    let dir = tempfile::tempdir().unwrap();
    let source_db_path = dir.path().join("source.db");
    let target_db_path = dir.path().join("target.db");
    let snapshot_path = dir.path().join("snapshot.json");
    let source_db = iotkit_core_storage::init_db(&source_db_path, &all_migrations()).unwrap();
    source_db
        .with_conn_sync(|conn| {
            iotkit_core_ledger::insert_device(
                conn,
                &iotkit_core_ledger::NewDevice {
                    hardware_id: "ble:source".into(),
                    user_label: None,
                    parent: None,
                    kind: iotkit_core_ledger::DeviceKind::Individual,
                    initial_state: iotkit_core_ledger::DeviceState::Active,
                },
            )
            .unwrap();
            iotkit_core_ledger::ledger_epoch(conn).unwrap();
            Ok(())
        })
        .unwrap();
    assert_success(run(&[
        "--db",
        source_db_path.to_str().unwrap(),
        "snapshot",
        "export",
        snapshot_path.to_str().unwrap(),
    ]));

    let target_db = iotkit_core_storage::init_db(&target_db_path, &all_migrations()).unwrap();
    target_db
        .with_conn_sync(|conn| {
            iotkit_core_ledger::insert_device(
                conn,
                &iotkit_core_ledger::NewDevice {
                    hardware_id: "ble:existing".into(),
                    user_label: None,
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
        "snapshot",
        "restore",
        snapshot_path.to_str().unwrap(),
        "--db",
        target_db_path.to_str().unwrap(),
        "--yes",
    ]));
    assert!(
        stderr.contains("restore target is not empty"),
        "stderr did not explain non-empty target:\n{stderr}"
    );
    target_db
        .with_conn_sync(|conn| {
            let devices: i64 = conn
                .query_row("SELECT COUNT(*) FROM devices", [], |row| row.get(0))
                .unwrap();
            assert_eq!(devices, 1);
            Ok(())
        })
        .unwrap();
}

#[test]
fn snapshot_restore_rejects_non_empty_registry_entries_table() {
    let dir = tempfile::tempdir().unwrap();
    let source_db_path = dir.path().join("source.db");
    let target_db_path = dir.path().join("target.db");
    let snapshot_path = dir.path().join("snapshot.json");
    let source_db = iotkit_core_storage::init_db(&source_db_path, &all_migrations()).unwrap();
    source_db
        .with_conn_sync(|conn| {
            iotkit_core_ledger::insert_device(
                conn,
                &iotkit_core_ledger::NewDevice {
                    hardware_id: "ble:source".into(),
                    user_label: None,
                    parent: None,
                    kind: iotkit_core_ledger::DeviceKind::Individual,
                    initial_state: iotkit_core_ledger::DeviceState::Active,
                },
            )
            .unwrap();
            iotkit_core_ledger::ledger_epoch(conn).unwrap();
            Ok(())
        })
        .unwrap();
    assert_success(run(&[
        "--db",
        source_db_path.to_str().unwrap(),
        "snapshot",
        "export",
        snapshot_path.to_str().unwrap(),
    ]));

    let target_db = iotkit_core_storage::init_db(&target_db_path, &all_migrations()).unwrap();
    target_db
        .with_conn_sync(|conn| {
            let catalog = iotkit_core_registry::standard_catalog();
            let entry = catalog.find("temperature_c").unwrap();
            iotkit_core_registry::enable_entry(
                conn,
                entry,
                &catalog.catalog_version,
                "restore-test",
            )
            .unwrap();
            let devices: i64 = conn
                .query_row("SELECT COUNT(*) FROM devices", [], |row| row.get(0))
                .unwrap();
            assert_eq!(devices, 0);
            Ok(())
        })
        .unwrap();

    let stderr = assert_failure(run(&[
        "snapshot",
        "restore",
        snapshot_path.to_str().unwrap(),
        "--db",
        target_db_path.to_str().unwrap(),
        "--yes",
    ]));
    assert!(
        stderr.contains("restore target is not empty"),
        "stderr did not explain non-empty target:\n{stderr}"
    );
    target_db
        .with_conn_sync(|conn| {
            let registry_entries: i64 = conn
                .query_row("SELECT COUNT(*) FROM registry_entries", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(registry_entries, 1);
            Ok(())
        })
        .unwrap();
}

#[test]
fn snapshot_restore_rejects_unknown_columns_before_insert() {
    let dir = tempfile::tempdir().unwrap();
    let source_db_path = dir.path().join("source.db");
    let target_db_path = dir.path().join("target.db");
    let snapshot_path = dir.path().join("snapshot.json");
    let source_db = iotkit_core_storage::init_db(&source_db_path, &all_migrations()).unwrap();
    source_db
        .with_conn_sync(|conn| {
            iotkit_core_ledger::insert_device(
                conn,
                &iotkit_core_ledger::NewDevice {
                    hardware_id: "ble:source".into(),
                    user_label: None,
                    parent: None,
                    kind: iotkit_core_ledger::DeviceKind::Individual,
                    initial_state: iotkit_core_ledger::DeviceState::Active,
                },
            )
            .unwrap();
            iotkit_core_ledger::ledger_epoch(conn).unwrap();
            Ok(())
        })
        .unwrap();
    assert_success(run(&[
        "--db",
        source_db_path.to_str().unwrap(),
        "snapshot",
        "export",
        snapshot_path.to_str().unwrap(),
    ]));

    let mut snapshot: Value =
        serde_json::from_slice(&std::fs::read(&snapshot_path).unwrap()).unwrap();
    snapshot["devices"][0]["unknown_snapshot_column"] = Value::from("blocked");
    std::fs::write(
        &snapshot_path,
        serde_json::to_vec_pretty(&snapshot).unwrap(),
    )
    .unwrap();
    iotkit_core_storage::init_db(&target_db_path, &all_migrations()).unwrap();

    let stderr = assert_failure(run(&[
        "snapshot",
        "restore",
        snapshot_path.to_str().unwrap(),
        "--db",
        target_db_path.to_str().unwrap(),
        "--yes",
    ]));
    assert!(
        stderr.contains("unknown snapshot column: devices.unknown_snapshot_column"),
        "stderr did not explain unknown column:\n{stderr}"
    );

    let target_db = iotkit_core_storage::init_db(&target_db_path, &all_migrations()).unwrap();
    target_db
        .with_conn_sync(|conn| {
            let devices: i64 = conn
                .query_row("SELECT COUNT(*) FROM devices", [], |row| row.get(0))
                .unwrap();
            assert_eq!(devices, 0);
            Ok(())
        })
        .unwrap();
}
