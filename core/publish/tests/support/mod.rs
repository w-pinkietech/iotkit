use super::*;

pub fn open() -> rusqlite::Connection {
    let mut all: Vec<Migration> = Vec::new();
    all.extend_from_slice(iotkit_core_storage::MIGRATIONS);
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.extend_from_slice(MIGRATIONS);
    all.sort_by_key(|migration| migration.version);
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    iotkit_core_storage::run_migrations(&conn, &all).unwrap();
    conn
}
