use super::*;

/// Open an in-memory database with all migrations applied.
pub fn init_db_memory(migrations: &[Migration]) -> Result<DbHandle, StorageError> {
    let conn = Connection::open_in_memory()?;
    configure_pragmas(&conn)?;
    run_migrations(&conn, migrations)?;
    Ok(DbHandle::new(conn))
}
