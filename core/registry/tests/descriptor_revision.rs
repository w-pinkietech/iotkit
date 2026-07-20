use iotkit_core_ledger::descriptor_revision;
use iotkit_core_registry::{AliasKind, MIGRATIONS, define_alias, enable_entry, standard_catalog};

#[test]
fn registry_entry_change_bumps_descriptor_revision_in_its_transaction() {
    let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    migrations.extend_from_slice(MIGRATIONS);
    migrations.sort_by_key(|m| m.version);
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    iotkit_core_storage::run_migrations(&conn, &migrations).unwrap();
    let before = descriptor_revision(&conn).unwrap();

    let tx = conn.unchecked_transaction().unwrap();
    let catalog = standard_catalog();
    enable_entry(
        &tx,
        catalog.find("contact_state").unwrap(),
        &catalog.catalog_version,
        "test",
    )
    .unwrap();
    assert_eq!(descriptor_revision(&tx).unwrap(), before + 1);
    tx.rollback().unwrap();

    assert_eq!(descriptor_revision(&conn).unwrap(), before);
}

#[test]
fn registry_alias_change_bumps_descriptor_revision_in_its_transaction() {
    let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    migrations.extend_from_slice(MIGRATIONS);
    migrations.sort_by_key(|m| m.version);
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    iotkit_core_storage::run_migrations(&conn, &migrations).unwrap();
    let before = descriptor_revision(&conn).unwrap();

    let catalog = standard_catalog();
    enable_entry(
        &conn,
        catalog.find("contact_state").unwrap(),
        &catalog.catalog_version,
        "test",
    )
    .unwrap();
    let after_entry = descriptor_revision(&conn).unwrap();
    assert_eq!(after_entry, before + 1);

    let tx = conn.unchecked_transaction().unwrap();
    define_alias(
        &tx,
        "contact_state_legacy",
        "contact_state",
        AliasKind::Rename,
    )
    .unwrap();
    assert_eq!(descriptor_revision(&tx).unwrap(), after_entry + 1);
    tx.rollback().unwrap();

    assert_eq!(descriptor_revision(&conn).unwrap(), after_entry);
}
