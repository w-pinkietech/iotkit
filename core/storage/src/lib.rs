//! iotkit-core-storage: SQLite persistence infrastructure for the IoT gateway.

mod error;
mod handle;
mod migrate;

use std::path::Path;

use rusqlite::Connection;

pub use error::StorageError;
pub use handle::DbHandle;
pub use migrate::{MIGRATIONS, Migration, run_migrations};

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
mod tests {
    use super::*;

    #[test]
    fn init_db_creates_and_migrates() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = init_db(&db_path, MIGRATIONS).unwrap();

        db.with_conn_sync(|conn| {
            let version: u32 = conn
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM _schema_version",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(version >= 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn init_db_idempotent_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let _db1 = init_db(&db_path, MIGRATIONS).unwrap();
        drop(_db1);
        let _db2 = init_db(&db_path, MIGRATIONS).unwrap();
    }

    #[test]
    fn init_db_missing_parent_returns_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let bad_path = dir.path().join("nonexistent_subdir").join("test.db");
        let result = init_db(&bad_path, MIGRATIONS);
        assert!(
            matches!(result, Err(StorageError::Io(_))),
            "expected StorageError::Io for missing parent, got {result:?}"
        );
    }

    #[test]
    fn init_db_memory_succeeds() {
        let db = init_db_memory(MIGRATIONS).unwrap();
        db.with_conn_sync(|conn| {
            let version: u32 = conn
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM _schema_version",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(version >= 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn pragmas_use_wal_and_full_sync() {
        let db = init_db_memory(&[]).unwrap();
        db.with_conn_sync(|conn| {
            let sync: i64 = conn
                .query_row("PRAGMA synchronous", [], |r| r.get(0))
                .unwrap();
            assert_eq!(sync, 2, "synchronous must be FULL (D8 amendment)");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn pragma_verification_file_backed() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = init_db(&db_path, MIGRATIONS).unwrap();

        db.with_conn_sync(|conn| {
            let journal_mode: String = conn
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .unwrap();
            assert_eq!(journal_mode, "wal");

            let synchronous: i32 = conn
                .query_row("PRAGMA synchronous", [], |row| row.get(0))
                .unwrap();
            assert_eq!(synchronous, 2); // FULL (D8 amendment)

            let foreign_keys: i32 = conn
                .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                .unwrap();
            assert_eq!(foreign_keys, 1);

            let busy_timeout: i32 = conn
                .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
                .unwrap();
            assert_eq!(busy_timeout, 5000);

            let cache_size: i32 = conn
                .query_row("PRAGMA cache_size", [], |row| row.get(0))
                .unwrap();
            assert_eq!(cache_size, -8000);

            Ok(())
        })
        .unwrap();
    }
}
