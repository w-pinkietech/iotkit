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
    for m in migrations
        .iter()
        .filter(|m| !applied.contains(&m.version()))
    {
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
        tx.commit().map_err(|e| StorageError::MigrationFailed {
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
#[path = "../tests/unit/migrate_tests.rs"]
mod tests;
