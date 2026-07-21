use iotkit_core_publish::descriptor::DESCRIPTOR_SCHEMA_VERSION;

#[test]
fn builds_adapter_neutral_snapshot_with_resolved_metadata() {
    let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_publish::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    migrations.sort_by_key(|m| m.version);
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    iotkit_core_storage::run_migrations(&conn, &migrations).unwrap();
    conn.execute(
        "INSERT INTO ledger_meta(key, value) VALUES
            ('edge_node_id', 'edge-node-01'), ('epoch', 'epoch-01')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO devices
            (system_id, hardware_id, presentation_identifier, kind, state, created_at)
         VALUES (?1, 'input:secret-provider-id:i2c:0x60', '01234567', 'positional', 'active', 1)",
        [
            iotkit_core_ledger::SystemId::from_text("018f0000-0000-7000-8000-000000000001")
                .unwrap()
                .as_bytes()
                .to_vec(),
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO positional_device_models(system_id, model_id)
         SELECT system_id, 'mcp9600' FROM devices",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO registry_entries (
            measurement_key, origin, entry_revision, value_type, semantic_class,
            channel_mode, enabled_at
         ) VALUES ('contact_state', 'custom', 'r1', 'bool', 'contact', 'single', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO series
            (system_id, measurement_key, channel_index, variant, created_at)
         SELECT system_id, 'contact_state', -1, 'primary', 1 FROM devices",
        [],
    )
    .unwrap();

    let snapshot =
        iotkit_edge_node::descriptor_snapshot::build_descriptor_snapshot(&conn, "edge-node-01")
            .unwrap();
    assert_eq!(snapshot.schema_version, DESCRIPTOR_SCHEMA_VERSION);
    assert!(snapshot.complete);
    assert_eq!(snapshot.descriptor_revision, 5);
    assert_eq!(snapshot.devices[0].identifier.as_deref(), Some("01234567"));
    assert_eq!(snapshot.devices[0].model_id.as_deref(), Some("mcp9600"));
    assert_eq!(snapshot.signals[0].value_type, "bool");
    assert_eq!(snapshot.signals[0].channel_index, None);

    let text = String::from_utf8(snapshot.encode_bounded().unwrap()).unwrap();
    assert!(!text.contains("secret-provider-id"));
    let fixture =
        std::fs::read_to_string("../testdata/egress/v2/descriptor-snapshot.json").unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap(),
        serde_json::from_str::<serde_json::Value>(&fixture).unwrap()
    );
}
