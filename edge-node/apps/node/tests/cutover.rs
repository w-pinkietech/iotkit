use std::path::Path;
use std::process::Command;

fn create_gateway_database(path: &std::path::Path) {
    let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    migrations.sort_by_key(|migration| migration.version);
    let db = iotkit_core_storage::init_db(path, &migrations).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute_batch(
            "DROP TABLE _iotkit_edge_format;
             INSERT INTO ledger_meta VALUES ('gateway_identity', 'legacy-edge');
             DELETE FROM _schema_version WHERE version = 7;",
        )
        .unwrap();
        conn.pragma_update(None, "journal_mode", "DELETE").unwrap();
        Ok(())
    })
    .unwrap();
    drop(db);
}

fn create_edge_node_database(path: &Path, include_recovery_migration: bool) {
    let mut migrations = iotkit_core_recovery::all_edge_node_migrations();
    if !include_recovery_migration {
        migrations.retain(|migration| migration.version < 23);
    }
    let db = iotkit_core_storage::init_db(path, &migrations).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "INSERT INTO ledger_meta(key, value) VALUES
                 ('edge_node_id', 'normal-node'), ('epoch', 'epoch-normal'), ('generation', '1')",
            [],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    drop(db);
    std::fs::write(
        iotkit_core_ops::database_initialization_marker_path(path),
        b"iotkit-database-initialized-v1\n",
    )
    .unwrap();
}

#[test]
fn daemon_rejects_gateway_database_before_migration_or_other_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("gateway.db");
    let health_path = dir.path().join("health.json");
    let config_path = dir.path().join("iotkit.toml");
    create_gateway_database(&db_path);
    std::fs::write(
        &config_path,
        format!(
            "[edge_node]\ndb_path = {:?}\nhealth_json_path = {:?}\n\
             [adapters.bravepi]\nenabled = false\n\
             [api]\nenabled = true\nbind = \"127.0.0.1:0\"\n",
            db_path.to_str().unwrap(),
            health_path.to_str().unwrap(),
        ),
    )
    .unwrap();
    let bytes_before = std::fs::read(&db_path).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_iotkit-edge-node"))
        .args(["--config", config_path.to_str().unwrap()])
        .env_remove("IOTKIT_DB_PATH")
        .env_remove("IOTKIT_CONFIG_PATH")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stderr.contains(
            "unsupported pre-release Edge Node database; recreate the Edge Node database"
        ) || stdout.contains(
            "unsupported pre-release Edge Node database; recreate the Edge Node database"
        ),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(std::fs::read(&db_path).unwrap(), bytes_before);
}

#[test]
fn normal_pre_recovery_and_current_databases_keep_migration_startup_behavior() {
    let directory = tempfile::tempdir().unwrap();
    for (name, include_recovery_migration) in [("pre-recovery", false), ("current", true)] {
        let db_path = directory.path().join(format!("{name}.db"));
        let config_path = directory.path().join(format!("{name}.toml"));
        create_edge_node_database(&db_path, include_recovery_migration);
        std::fs::write(
            &config_path,
            format!(
                "[edge_node]\n db_path = {:?}\n\
                 [api]\n enabled = false\n\
                 [exit.mqtt]\n enabled = true\n host = \"127.0.0.1\"\n port = 1883\n\
                 password_file = {:?}\n allow_insecure = true\n",
                db_path,
                directory.path().join(format!("{name}-missing-password")),
            ),
        )
        .unwrap();

        let output = Command::new(env!("CARGO_BIN_EXE_iotkit-edge-node"))
            .args(["--config", config_path.to_str().unwrap()])
            .env_remove("IOTKIT_DB_PATH")
            .env_remove("IOTKIT_CONFIG_PATH")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "stderr={stderr}");
        assert!(
            !stderr.contains("fenced recovery candidate"),
            "stderr={stderr}"
        );

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        assert!(
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM _schema_version WHERE version = 23)",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap(),
            "{name} database did not reach the recovery migration",
        );
    }
}
