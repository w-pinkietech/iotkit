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
