use rusqlite::Connection;

use crate::StorageError;

pub(crate) struct Migration {
    version: u32,
    label: &'static str,
    sql: &'static str,
}

pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    label: "init",
    sql: include_str!("../migrations/0001_init.sql"),
}];

/// Internal trait to abstract over Migration and TestMigration.
/// Allows the same apply loop to be used for production and tests.
trait MigrationEntry {
    fn version(&self) -> u32;
    fn label(&self) -> &str;
    fn sql(&self) -> &str;
}

impl MigrationEntry for Migration {
    fn version(&self) -> u32 {
        self.version
    }
    fn label(&self) -> &str {
        self.label
    }
    fn sql(&self) -> &str {
        self.sql
    }
}

/// Full migration runner: bootstrap table, schema-ahead check, apply pending.
/// Used by both run_migrations() (production) and run_migrations_with() (test).
fn run_migrations_inner<M: MigrationEntry>(
    conn: &Connection,
    migrations: &[M],
) -> Result<(), StorageError> {
    // Bootstrap: create version table if absent.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _schema_version (
            version    INTEGER NOT NULL,
            label      TEXT    NOT NULL,
            applied_at INTEGER NOT NULL,
            PRIMARY KEY (version)
        );",
    )?;

    let current_version: u32 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM _schema_version",
        [],
        |row| row.get(0),
    )?;

    // Schema-ahead guard
    let latest_known = migrations.last().map_or(0, |m| m.version());
    if current_version > latest_known {
        return Err(StorageError::SchemaVersionAhead {
            on_disk: current_version,
            latest_known,
        });
    }

    // Apply each pending migration in its own transaction
    for m in migrations.iter().filter(|m| m.version() > current_version) {
        tracing::info!(
            version = m.version(),
            label = m.label(),
            "applying migration"
        );
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| StorageError::MigrationFailed {
                version: m.version(),
                source: Box::new(e.into()),
            })?;
        tx.execute_batch(m.sql())
            .map_err(|e| StorageError::MigrationFailed {
                version: m.version(),
                source: Box::new(e.into()),
            })?;
        let applied_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_secs();
        tx.execute(
            "INSERT INTO _schema_version (version, label, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![m.version(), m.label(), applied_at as i64],
        )
        .map_err(|e| StorageError::MigrationFailed {
            version: m.version(),
            source: Box::new(e.into()),
        })?;
        tx.commit()
            .map_err(|e| StorageError::MigrationFailed {
                version: m.version(),
                source: Box::new(e.into()),
            })?;
    }

    Ok(())
}

/// Production entry point: runs MIGRATIONS through the shared inner runner.
pub(crate) fn run_migrations(conn: &Connection) -> Result<(), StorageError> {
    run_migrations_inner(conn, MIGRATIONS)
}

/// Test helper: runs a custom migration list through the same inner runner.
#[cfg(test)]
pub(crate) fn run_migrations_with(
    conn: &Connection,
    migrations: &[TestMigration],
) -> Result<(), StorageError> {
    run_migrations_inner(conn, migrations)
}

#[cfg(test)]
pub(crate) struct TestMigration {
    pub version: u32,
    pub label: &'static str,
    pub sql: &'static str,
}

#[cfg(test)]
impl MigrationEntry for TestMigration {
    fn version(&self) -> u32 {
        self.version
    }
    fn label(&self) -> &str {
        self.label
    }
    fn sql(&self) -> &str {
        self.sql
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrations_array_invariants() {
        assert!(!MIGRATIONS.is_empty(), "MIGRATIONS must not be empty");
        for (i, m) in MIGRATIONS.iter().enumerate() {
            let expected_version = (i as u32) + 1;
            assert_eq!(
                m.version, expected_version,
                "migration at index {i} has version {} but expected {expected_version}",
                m.version
            );
        }
    }

    #[test]
    fn fresh_db_gets_migrated() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let version: u32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM _schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, MIGRATIONS.last().unwrap().version);

        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM _schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, MIGRATIONS.len() as u32);
    }

    #[test]
    fn already_migrated_is_noop() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();

        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM _schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, MIGRATIONS.len() as u32);
    }

    #[test]
    fn schema_version_ahead_rejected() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE _schema_version (
                version INTEGER NOT NULL PRIMARY KEY,
                label TEXT NOT NULL,
                applied_at INTEGER NOT NULL
            );
            INSERT INTO _schema_version VALUES (9999, 'future', 0);",
        )
        .unwrap();

        let result = run_migrations(&conn);
        assert!(
            matches!(
                result,
                Err(StorageError::SchemaVersionAhead {
                    on_disk: 9999,
                    ..
                })
            ),
            "expected SchemaVersionAhead, got {result:?}"
        );
    }

    #[test]
    fn migration_failure_rolls_back() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let test_migrations = &[
            TestMigration {
                version: 1,
                label: "good",
                sql: "CREATE TABLE test_ok (id INTEGER PRIMARY KEY);",
            },
            TestMigration {
                version: 2,
                label: "bad",
                // First statement succeeds, second fails -- tests partial rollback
                sql: "CREATE TABLE half_done (id INTEGER PRIMARY KEY);\nTHIS IS NOT VALID SQL;",
            },
        ];

        // Apply with intentionally broken second migration
        let result = run_migrations_with(&conn, test_migrations);

        // Must return MigrationFailed for version 2
        assert!(
            matches!(
                result,
                Err(StorageError::MigrationFailed { version: 2, .. })
            ),
            "expected MigrationFailed for v2, got {result:?}"
        );

        // Version 1 should be applied (persisted in _schema_version)
        let applied_version: u32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM _schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(applied_version, 1, "v1 should be persisted, v2 rolled back");

        // test_ok table from v1 should exist
        let table_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='test_ok'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(table_exists, "v1 table should exist after v2 rollback");

        // half_done table from v2's first statement should NOT exist (rolled back)
        let half_done_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='half_done'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !half_done_exists,
            "v2 partial changes should be rolled back"
        );
    }
}
