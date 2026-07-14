//! iotkit-core-storage: SQLite persistence infrastructure for IoTKit Edge.

mod error;
mod handle;
mod migrate;

use std::path::Path;

use rusqlite::Connection;

pub use error::StorageError;
pub use handle::DbHandle;
pub use migrate::{MIGRATIONS, Migration, run_migrations};

/// Read-only cutover guard for an on-disk IoTKit Edge database.
///
/// Missing and zero-length paths are fresh creation targets. A non-empty database is current
/// only when it already has an `edge_node_id`, or when migration v1 from the post-cutover code
/// created the private format marker before identity initialization completed. Everything else
/// is an unsupported pre-release database and is never migrated in place.
pub fn preflight_edge_database(db_path: &Path) -> Result<(), StorageError> {
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
mod tests {
    use super::*;

    fn create_identity_database(db_path: &Path, key: Option<&str>) {
        let conn = Connection::open(db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE _schema_version (
                version INTEGER NOT NULL PRIMARY KEY,
                label TEXT NOT NULL,
                applied_at INTEGER NOT NULL
            );
            INSERT INTO _schema_version VALUES (1, 'init', 0);
            CREATE TABLE ledger_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE preserved_data (
                id INTEGER PRIMARY KEY,
                value TEXT NOT NULL
            );
            INSERT INTO preserved_data VALUES (1, 'must-not-change');",
        )
        .unwrap();
        if let Some(key) = key {
            conn.execute(
                "INSERT INTO ledger_meta (key, value) VALUES (?1, 'identity')",
                [key],
            )
            .unwrap();
        }
    }

    fn assert_preflight_rejects_without_mutation(db_path: &Path) {
        let bytes_before = std::fs::read(db_path).unwrap();
        let error = preflight_edge_database(db_path).unwrap_err();
        assert_eq!(
            error.to_string(),
            "unsupported pre-release Edge database; recreate the Edge database"
        );
        assert_eq!(std::fs::read(db_path).unwrap(), bytes_before);

        let conn = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
        assert_eq!(
            conn.query_row("SELECT value FROM preserved_data WHERE id = 1", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
            "must-not-change"
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM sqlite_schema", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            4
        );
    }

    #[test]
    fn cutover_preflight_rejects_gateway_identity_database_without_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("gateway.db");
        create_identity_database(&db_path, Some("gateway_identity"));

        assert_preflight_rejects_without_mutation(&db_path);
    }

    #[test]
    fn cutover_preflight_rejects_pre_cutover_database_without_identity_or_marker() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("pre-cutover.db");
        create_identity_database(&db_path, None);

        assert_preflight_rejects_without_mutation(&db_path);
    }

    #[test]
    fn cutover_preflight_accepts_current_edge_node_identity() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("current.db");
        create_identity_database(&db_path, Some("edge_node_id"));

        preflight_edge_database(&db_path).unwrap();
    }

    #[test]
    fn cutover_preflight_rejects_ambiguous_marker_with_cutover_error() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("ambiguous-marker.db");
        create_identity_database(&db_path, None);
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE _iotkit_edge_format (unknown INTEGER);")
            .unwrap();
        drop(conn);
        let bytes_before = std::fs::read(&db_path).unwrap();

        let error = preflight_edge_database(&db_path).unwrap_err();

        assert_eq!(
            error.to_string(),
            "unsupported pre-release Edge database; recreate the Edge database"
        );
        assert_eq!(std::fs::read(db_path).unwrap(), bytes_before);
    }

    #[test]
    fn cutover_preflight_accepts_absent_and_zero_length_fresh_targets() {
        let dir = tempfile::tempdir().unwrap();
        let absent = dir.path().join("absent.db");
        preflight_edge_database(&absent).unwrap();

        let empty = dir.path().join("empty.db");
        std::fs::File::create(&empty).unwrap();
        preflight_edge_database(&empty).unwrap();
        assert_eq!(std::fs::metadata(empty).unwrap().len(), 0);
    }

    #[test]
    fn baseline_migration_marks_new_format_before_edge_identity_exists() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("fresh.db");
        drop(init_db(&db_path, MIGRATIONS).unwrap());

        preflight_edge_database(&db_path).unwrap();
        let conn =
            Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let marker: i64 = conn
            .query_row(
                "SELECT format_version FROM _iotkit_edge_format WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker, 1);
    }

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
