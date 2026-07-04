use rusqlite::Connection;

use crate::StorageError;

/// A single schema migration step.
#[derive(Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub label: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[Migration {
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

    // 適用済みversionの集合。分割マイグレーション(storage/ledger/timeseriesが各自のconstを持つ)では
    // 部分セットで初期化されたDBが存在しうるため、MAX(version)水位ではなく集合差で未適用を判定する。
    let applied: std::collections::HashSet<u32> = conn
        .prepare("SELECT version FROM _schema_version")?
        .query_map([], |row| row.get::<_, u32>(0))?
        .collect::<Result<_, _>>()?;
    let current_version: u32 = applied.iter().copied().max().unwrap_or(0);

    // Schema-ahead guard
    let latest_known = migrations.last().map_or(0, |m| m.version());
    if current_version > latest_known {
        return Err(StorageError::SchemaVersionAhead {
            on_disk: current_version,
            latest_known,
        });
    }

    // Apply each pending (= not yet applied) migration in its own transaction
    for m in migrations.iter().filter(|m| !applied.contains(&m.version())) {
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

/// Run a list of migrations on the given connection.
/// Validates that versions are strictly ascending before applying.
pub fn run_migrations(conn: &Connection, migrations: &[Migration]) -> Result<(), StorageError> {
    // Validate: versions must be strictly ascending
    for window in migrations.windows(2) {
        if window[0].version >= window[1].version {
            return Err(StorageError::InvalidMigrationOrder {
                first: window[0].version,
                second: window[1].version,
            });
        }
    }
    run_migrations_inner(conn, migrations)
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
        run_migrations(&conn, MIGRATIONS).unwrap();

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
        run_migrations(&conn, MIGRATIONS).unwrap();
        run_migrations(&conn, MIGRATIONS).unwrap();

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

        let result = run_migrations(&conn, MIGRATIONS);
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
    fn migration_order_validation_rejects_out_of_order() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let bad_migrations = &[
            Migration { version: 2, label: "second", sql: "SELECT 1;" },
            Migration { version: 1, label: "first", sql: "SELECT 1;" },
        ];
        let result = run_migrations(&conn, bad_migrations);
        assert!(
            matches!(
                result,
                Err(StorageError::InvalidMigrationOrder { first: 2, second: 1 })
            ),
            "expected InvalidMigrationOrder, got {result:?}"
        );
    }

    #[test]
    fn migration_duplicate_version_rejected() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let bad_migrations = &[
            Migration { version: 1, label: "first", sql: "SELECT 1;" },
            Migration { version: 1, label: "also-first", sql: "SELECT 1;" },
        ];
        let result = run_migrations(&conn, bad_migrations);
        assert!(
            matches!(
                result,
                Err(StorageError::InvalidMigrationOrder { first: 1, second: 1 })
            ),
            "expected InvalidMigrationOrder, got {result:?}"
        );
    }

    #[test]
    fn external_migrations_applied() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let combined = &[
            Migration {
                version: 1,
                label: "init",
                sql: include_str!("../migrations/0001_init.sql"),
            },
            Migration {
                version: 2,
                label: "extra",
                sql: "CREATE TABLE extra_table (id INTEGER PRIMARY KEY);",
            },
        ];
        run_migrations(&conn, combined).unwrap();

        let version: u32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM _schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 2);

        // Verify extra_table exists
        let table_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='extra_table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(table_exists, "extra_table should exist after migration v2");
    }

    #[test]
    fn missing_middle_migration_is_applied_on_rerun() {
        // 部分セット[1,2,4]で初期化されたDB(分割マイグレーションの誤用シナリオ)に
        // 完全セット[1,2,3,4]を渡すと、水位方式ではv3が永久スキップされる——集合差方式の検証
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let v1 = Migration { version: 1, label: "init", sql: include_str!("../migrations/0001_init.sql") };
        let v2 = Migration { version: 2, label: "a", sql: "CREATE TABLE t2 (id INTEGER PRIMARY KEY);" };
        let v3 = Migration { version: 3, label: "b", sql: "CREATE TABLE t3 (id INTEGER PRIMARY KEY);" };
        let v4 = Migration { version: 4, label: "c", sql: "CREATE TABLE t4 (id INTEGER PRIMARY KEY);" };

        run_migrations(&conn, &[v1, v2, v4]).unwrap(); // 部分適用(1,2,4は昇順なので通る)
        run_migrations(&conn, &[v1, v2, v3, v4]).unwrap(); // 完全セットでv3が埋まる

        let t3_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='t3'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(t3_exists, "v3 must be applied on rerun with the full set");
        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM _schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 4, "no duplicate rows for already-applied versions");
    }

    #[test]
    fn migration_set_difference_tolerates_gap() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let migrations = &[
            TestMigration {
                version: 1,
                label: "init",
                sql: "CREATE TABLE readings (id INTEGER PRIMARY KEY, value TEXT NOT NULL);",
            },
            TestMigration {
                version: 3,
                label: "readings_insert",
                sql: "INSERT INTO readings (value) VALUES ('kept');",
            },
            TestMigration {
                version: 4,
                label: "readings_index",
                sql: "CREATE INDEX idx_readings_value ON readings(value);",
            },
            TestMigration {
                version: 5,
                label: "ledger_extra",
                sql: "CREATE TABLE ledger_extra (id INTEGER PRIMARY KEY);",
            },
            TestMigration {
                version: 6,
                label: "registry_extra",
                sql: "CREATE TABLE registry_extra (id INTEGER PRIMARY KEY);",
            },
            TestMigration {
                version: 7,
                label: "drop_legacy",
                sql: "DROP TABLE IF EXISTS sensor_readings;",
            },
        ];

        run_migrations_with(&conn, migrations).unwrap();

        let versions: Vec<u32> = conn
            .prepare("SELECT version FROM _schema_version ORDER BY version")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(versions, vec![1, 3, 4, 5, 6, 7]);
        let value: String = conn
            .query_row("SELECT value FROM readings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "kept");
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
