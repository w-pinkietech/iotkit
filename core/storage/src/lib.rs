//! iotkit-core-storage: SQLite persistence infrastructure for IoTKit Edge Node.

mod error;
mod handle;
mod migrate;

use std::path::Path;

use rusqlite::Connection;

pub use error::StorageError;
pub use handle::DbHandle;
pub use migrate::{MIGRATIONS, Migration, run_migrations};

/// Read-only cutover guard for an on-disk IoTKit Edge Node database.
///
/// Missing and zero-length paths are fresh creation targets. A non-empty database is current
/// only when it already has an `edge_node_id`, or when migration v1 from the post-cutover code
/// created the private format marker before identity initialization completed. Everything else
/// is an unsupported pre-release database and is never migrated in place.
pub fn preflight_edge_node_database(db_path: &Path) -> Result<(), StorageError> {
    let metadata = match std::fs::metadata(db_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.len() == 0 {
        return Ok(());
    }

    let conn = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let has_ledger_meta = table_exists(&conn, "ledger_meta")?;
    if has_ledger_meta && ledger_meta_key_exists(&conn, "gateway_identity")? {
        return Err(StorageError::UnsupportedPreReleaseEdgeDatabase);
    }
    if has_ledger_meta && ledger_meta_key_exists(&conn, "edge_node_id")? {
        return Ok(());
    }

    if table_exists(&conn, "_iotkit_edge_format")? && edge_format_marker_is_current(&conn)? {
        return Ok(());
    }

    Err(StorageError::UnsupportedPreReleaseEdgeDatabase)
}

fn edge_format_marker_is_current(conn: &Connection) -> Result<bool, StorageError> {
    let schema_is_exact = conn.query_row(
        "SELECT COUNT(*) = 2
             AND SUM(name = 'singleton' AND type = 'INTEGER' AND \"notnull\" = 1 AND pk = 1) = 1
             AND SUM(name = 'format_version' AND type = 'INTEGER' AND \"notnull\" = 1 AND pk = 0) = 1
         FROM pragma_table_info('_iotkit_edge_format')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if !schema_is_exact {
        return Ok(false);
    }
    Ok(conn.query_row(
        "SELECT COUNT(*) = 1
             AND MIN(singleton) = 1
             AND MIN(format_version) = 1
         FROM _iotkit_edge_format",
        [],
        |row| row.get(0),
    )?)
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool, StorageError> {
    Ok(conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1
        )",
        [name],
        |row| row.get(0),
    )?)
}

fn ledger_meta_key_exists(conn: &Connection, key: &str) -> Result<bool, StorageError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM ledger_meta WHERE key = ?1)",
        [key],
        |row| row.get(0),
    )?)
}

/// Configure SQLite pragmas for production use.
fn configure_pragmas(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         PRAGMA cache_size = -8000;",
    )?;
    Ok(())
}

/// Open (or create) the database, configure pragmas, run migrations.
/// Synchronous -- call before entering the async runtime.
pub fn init_db(db_path: &Path, migrations: &[Migration]) -> Result<DbHandle, StorageError> {
    // Check parent directory exists -- surface as StorageError::Io, not Sqlite.
    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        return Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("parent directory does not exist: {}", parent.display()),
        )));
    }
    let conn = Connection::open(db_path)?;
    configure_pragmas(&conn)?;
    run_migrations(&conn, migrations)?;
    Ok(DbHandle::new(conn))
}

/// Open an in-memory database with all migrations applied.
/// For tests only.
#[cfg(any(test, feature = "test-util"))]
pub fn init_db_memory(migrations: &[Migration]) -> Result<DbHandle, StorageError> {
    let conn = Connection::open_in_memory()?;
    configure_pragmas(&conn)?;
    run_migrations(&conn, migrations)?;
    Ok(DbHandle::new(conn))
}

#[cfg(test)]
#[path = "../tests/unit/lib_tests.rs"]
mod tests;
