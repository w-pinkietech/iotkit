use iotkit_core_ledger::{
    DeviceKind, DeviceState, MIGRATIONS, NewDevice, bind_positional_model, descriptor_revision,
    ensure_series, insert_device, set_presentation_identifier,
};

fn migrations_through(version: u32) -> Vec<iotkit_core_storage::Migration> {
    let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    migrations.extend(MIGRATIONS.iter().copied().filter(|m| m.version <= version));
    migrations.sort_by_key(|m| m.version);
    migrations
}

#[test]
fn descriptor_migration_preserves_existing_devices_and_initializes_revision() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    iotkit_core_storage::run_migrations(&conn, &migrations_through(11)).unwrap();
    conn.execute(
        "INSERT INTO devices (system_id, hardware_id, kind, state, created_at)
         VALUES (?1, 'ble:existing', 'individual', 'active', 1)",
        [vec![1_u8; 16]],
    )
    .unwrap();

    iotkit_core_storage::run_migrations(&conn, &migrations_through(18)).unwrap();

    let hardware_id: String = conn
        .query_row("SELECT hardware_id FROM devices", [], |row| row.get(0))
        .unwrap();
    assert_eq!(hardware_id, "ble:existing");
    assert_eq!(descriptor_revision(&conn).unwrap(), 1);
}

#[test]
fn descriptor_revision_tracks_device_series_and_identifier_transactions() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    iotkit_core_storage::run_migrations(&conn, &migrations_through(18)).unwrap();
    assert_eq!(descriptor_revision(&conn).unwrap(), 1);

    let tx = conn.unchecked_transaction().unwrap();
    let system_id = insert_device(
        &tx,
        &NewDevice {
            hardware_id: "ble:device-01".into(),
            user_label: None,
            parent: None,
            kind: DeviceKind::Individual,
            initial_state: DeviceState::Active,
        },
    )
    .unwrap();
    set_presentation_identifier(&tx, &system_id, Some("01234567")).unwrap();
    ensure_series(&tx, &system_id, "contact_state", -1, "primary", false, None).unwrap();
    assert_eq!(descriptor_revision(&tx).unwrap(), 4);
    tx.rollback().unwrap();

    assert_eq!(descriptor_revision(&conn).unwrap(), 1);
}

#[test]
fn presentation_identifier_rejects_control_and_overlong_values() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    iotkit_core_storage::run_migrations(&conn, &migrations_through(18)).unwrap();
    let system_id = insert_device(
        &conn,
        &NewDevice {
            hardware_id: "ble:device-02".into(),
            user_label: None,
            parent: None,
            kind: DeviceKind::Individual,
            initial_state: DeviceState::Active,
        },
    )
    .unwrap();

    assert!(set_presentation_identifier(&conn, &system_id, Some("bad\nvalue")).is_err());
    assert!(set_presentation_identifier(&conn, &system_id, Some(&"x".repeat(65))).is_err());
}

#[test]
fn descriptor_model_migration_preserves_revision_and_binding_advances_it() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    iotkit_core_storage::run_migrations(&conn, &migrations_through(21)).unwrap();
    conn.execute(
        "INSERT INTO ledger_meta(key, value) VALUES ('edge_node_id', 'edge-node-01')",
        [],
    )
    .unwrap();
    let before_migration = descriptor_revision(&conn).unwrap();

    iotkit_core_storage::run_migrations(&conn, &migrations_through(22)).unwrap();
    assert_eq!(descriptor_revision(&conn).unwrap(), before_migration);

    let system_id = insert_device(
        &conn,
        &NewDevice {
            hardware_id: "input:test:line-a:i2c:0x60".into(),
            user_label: Some("MCP9600 thermocouple".into()),
            parent: None,
            kind: DeviceKind::Positional,
            initial_state: DeviceState::Active,
        },
    )
    .unwrap();
    let before_binding = descriptor_revision(&conn).unwrap();
    bind_positional_model(&conn, &system_id, "mcp9600").unwrap();
    assert_eq!(descriptor_revision(&conn).unwrap(), before_binding + 1);
}

#[test]
fn positional_model_binding_rejects_non_canonical_model_id() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    iotkit_core_storage::run_migrations(&conn, &migrations_through(22)).unwrap();
    let system_id = insert_device(
        &conn,
        &NewDevice {
            hardware_id: "input:test:line-a:i2c:0x60".into(),
            user_label: None,
            parent: None,
            kind: DeviceKind::Positional,
            initial_state: DeviceState::Active,
        },
    )
    .unwrap();

    let error = bind_positional_model(&conn, &system_id, "Model ID").unwrap_err();
    assert!(matches!(
        error,
        iotkit_core_ledger::LedgerError::InvalidModelId(_)
    ));
    assert_eq!(
        iotkit_core_ledger::positional_model_id(&conn, &system_id).unwrap(),
        None
    );
}
