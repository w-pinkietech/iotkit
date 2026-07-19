use iotkit_core_ledger::MIGRATIONS;

fn migrations_through(version: u32) -> Vec<iotkit_core_storage::Migration> {
    let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    migrations.extend(MIGRATIONS.iter().copied().filter(|m| m.version <= version));
    migrations.sort_by_key(|migration| migration.version);
    migrations
}

#[test]
fn model_migration_fences_the_previous_fixed_rpi_local_inventory() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    iotkit_core_storage::run_migrations(&conn, &migrations_through(18)).unwrap();
    conn.execute_batch(
        "
        INSERT INTO devices(
            system_id, hardware_id, user_label, kind, state, created_at
        ) VALUES
            (x'00000000000000000000000000000001',
             'rpi-local:default:i2c:0x60',
             'MCP9600 thermocouple', 'positional', 'active', 1),
            (x'00000000000000000000000000000002',
             'rpi-local:default:i2c:0x44',
             'OPT3001 illuminance', 'positional', 'active', 1),
            (x'00000000000000000000000000000003',
             'other:position',
             'Operator label', 'positional', 'active', 1),
            (x'00000000000000000000000000000004',
             'other:position:i2c:0x61',
             'MCP9600 thermocouple', 'positional', 'active', 1),
            (x'00000000000000000000000000000005',
             'rpi-local:default:i2c:0x45',
             'OPT3001 illuminance', 'positional', 'active', 1);
        ",
    )
    .unwrap();

    iotkit_core_storage::run_migrations(&conn, &migrations_through(21)).unwrap();

    let models = {
        let mut statement = conn
            .prepare(
                "SELECT hex(system_id), model_id
                 FROM positional_device_models
                 ORDER BY system_id",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(
        models,
        [
            ("00000000000000000000000000000001".into(), "mcp9600".into()),
            ("00000000000000000000000000000002".into(), "opt3001".into()),
        ]
    );
}
