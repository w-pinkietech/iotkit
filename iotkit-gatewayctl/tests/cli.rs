use std::process::{Command, Output};

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
