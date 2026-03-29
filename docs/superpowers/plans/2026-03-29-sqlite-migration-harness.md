# SQLite Migration Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create `iotkit-core-storage` crate with a SQLite migration harness and `DbHandle` abstraction, then integrate it into the gateway startup sequence.

**Architecture:** New `core/storage` crate owns connection init (open + PRAGMAs + migrate) and a thread-safe `DbHandle` wrapper. Gateway calls `init_db()` synchronously before the tokio runtime, passes `DbHandle` to `run()`. Migration SQL files are compiled into the binary via `include_str!()`.

**Tech Stack:** Rust 2024, rusqlite 0.32 (bundled), tokio 1.x (spawn_blocking), tracing 0.1

**Task ordering:** Sequential: 1 → 2 → 3 → 4 → 5. Each task builds on the previous. `lib.rs` is extended incrementally.

---

## File Structure

| Action | Path | Responsibility |
|---|---|---|
| Create | `core/storage/Cargo.toml` | Crate manifest with rusqlite, tokio, tracing deps |
| Create | `core/storage/src/lib.rs` | Public API: `init_db`, `init_db_memory`, re-exports |
| Create | `core/storage/src/error.rs` | `StorageError` enum |
| Create | `core/storage/src/handle.rs` | `DbHandle` struct + `with_conn` / `with_conn_sync` |
| Create | `core/storage/src/migrate.rs` | `Migration` struct, `MIGRATIONS` array, `run_migrations()` |
| Create | `core/storage/migrations/0001_init.sql` | Empty baseline migration |
| Modify | `Cargo.toml` (workspace root) | Add `"core/storage"` to `members` |
| Modify | `iotkit-gateway/Cargo.toml` | Add `iotkit-core-storage` dependency |
| Modify | `iotkit-gateway/src/main.rs` | Call `init_db()` before runtime, pass `DbHandle` to `run()` |

---

### Task 1: Crate Scaffold + StorageError

**Files:**
- Create: `core/storage/Cargo.toml`
- Create: `core/storage/src/lib.rs`
- Create: `core/storage/src/error.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create directory structure and Cargo.toml**

```toml
# core/storage/Cargo.toml
[package]
name = "iotkit-core-storage"
version = "0.1.0"
edition = "2024"

[dependencies]
rusqlite = { version = "0.32", features = ["bundled"] }
tokio = { version = "1", features = ["rt"] }
tracing = "0.1"

[dev-dependencies]
tokio = { version = "1", features = ["rt", "macros"] }
tempfile = "3"

[features]
test-util = []
```

- [ ] **Step 2: Create error.rs with StorageError**

```rust
// core/storage/src/error.rs

/// All errors from iotkit-core-storage.
#[derive(Debug)]
pub enum StorageError {
    /// SQLite operation failure.
    Sqlite(rusqlite::Error),
    /// Filesystem error (e.g., parent directory missing).
    Io(std::io::Error),
    /// A specific migration failed.
    MigrationFailed {
        version: u32,
        source: Box<StorageError>,
    },
    /// On-disk schema is newer than this binary knows about.
    SchemaVersionAhead { on_disk: u32, latest_known: u32 },
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::MigrationFailed { version, source } => {
                write!(f, "migration v{version} failed: {source}")
            }
            Self::SchemaVersionAhead {
                on_disk,
                latest_known,
            } => {
                write!(
                    f,
                    "schema version {on_disk} is ahead of latest known {latest_known}; upgrade the binary"
                )
            }
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::MigrationFailed { source, .. } => Some(source.as_ref()),
            Self::SchemaVersionAhead { .. } => None,
        }
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
```

- [ ] **Step 3: Create lib.rs**

```rust
// core/storage/src/lib.rs

//! iotkit-core-storage: SQLite persistence infrastructure for the IoT gateway.

mod error;

pub use error::StorageError;
```

- [ ] **Step 4: Add crate to workspace**

In `Cargo.toml` (workspace root), add `"core/storage"` to the `members` array after `"core/engine"`.

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p iotkit-core-storage`
Expected: Compiles with no errors.

- [ ] **Step 6: Commit**

```bash
git add core/storage/ Cargo.toml
git commit -m "feat(iotkit-core-storage): scaffold crate with StorageError"
```

---

### Task 2: DbHandle with Reentrancy Guard

**Depends on:** Task 1

**Files:**
- Create: `core/storage/src/handle.rs`
- Modify: `core/storage/src/lib.rs`

- [ ] **Step 1: Create handle.rs with full implementation + tests**

Create `core/storage/src/handle.rs` with the complete `DbHandle` implementation and test module. The module is declared in `lib.rs` in the same step so the tree stays compilable.

```rust
// core/storage/src/handle.rs

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::StorageError;

/// Thread-safe handle to a SQLite connection. Clone is cheap (Arc clone).
///
/// # Non-reentrancy
///
/// Do NOT call `with_conn` or `with_conn_sync` from inside a closure passed
/// to these methods. Pass the `&Connection` reference to inner helpers instead.
/// Same-thread reentry is detected and panics; cross-thread reentry can deadlock.
#[derive(Clone)]
pub struct DbHandle {
    conn: Arc<Mutex<Connection>>,
}

impl std::fmt::Debug for DbHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbHandle").finish_non_exhaustive()
    }
}

thread_local! {
    static ACTIVE_HANDLES: std::cell::RefCell<std::collections::HashSet<usize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

struct ReentrancyGuard {
    key: usize,
}

impl Drop for ReentrancyGuard {
    fn drop(&mut self) {
        ACTIVE_HANDLES.with(|set| {
            set.borrow_mut().remove(&self.key);
        });
    }
}

fn enter_guard(key: usize) -> ReentrancyGuard {
    ACTIVE_HANDLES.with(|set| {
        if !set.borrow_mut().insert(key) {
            panic!("DbHandle re-entered — pass &Connection instead");
        }
    });
    ReentrancyGuard { key }
}

impl DbHandle {
    pub(crate) fn new(conn: Connection) -> Self {
        Self {
            conn: Arc::new(Mutex::new(conn)),
        }
    }

    fn identity(&self) -> usize {
        Arc::as_ptr(&self.conn) as usize
    }

    pub async fn with_conn<F, T>(&self, f: F) -> Result<T, StorageError>
    where
        F: FnOnce(&Connection) -> Result<T, StorageError> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        let key = self.identity();
        tokio::task::spawn_blocking(move || {
            let _guard = enter_guard(key);
            let lock = conn.lock().expect("DbHandle mutex poisoned");
            f(&lock)
        })
        .await
        .expect("DbHandle spawn_blocking task panicked")
    }

    pub fn with_conn_sync<F, T>(&self, f: F) -> Result<T, StorageError>
    where
        F: FnOnce(&Connection) -> Result<T, StorageError>,
    {
        let _guard = enter_guard(self.identity());
        let lock = self.conn.lock().expect("DbHandle mutex poisoned");
        f(&lock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_conn_sync_executes_query() {
        let conn = Connection::open_in_memory().unwrap();
        let handle = DbHandle::new(conn);
        let result = handle
            .with_conn_sync(|c| {
                let n: i64 = c.query_row("SELECT 42", [], |row| row.get(0))?;
                Ok(n)
            })
            .unwrap();
        assert_eq!(result, 42);
    }

    #[tokio::test]
    async fn with_conn_executes_query() {
        let conn = Connection::open_in_memory().unwrap();
        let handle = DbHandle::new(conn);
        let result = handle
            .with_conn(|c| {
                let n: i64 = c.query_row("SELECT 1 + 1", [], |row| row.get(0))?;
                Ok(n)
            })
            .await
            .unwrap();
        assert_eq!(result, 2);
    }

    #[test]
    #[should_panic(expected = "DbHandle re-entered")]
    fn with_conn_sync_reentry_panics() {
        let conn = Connection::open_in_memory().unwrap();
        let handle = DbHandle::new(conn);
        let handle_clone = handle.clone();
        handle
            .with_conn_sync(|_c| {
                handle_clone.with_conn_sync(|_c2| Ok(())).unwrap();
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn cross_thread_contention_both_succeed() {
        let conn = Connection::open_in_memory().unwrap();
        let handle = DbHandle::new(conn);
        let handle2 = handle.clone();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let barrier2 = barrier.clone();

        let t1 = std::thread::spawn(move || {
            handle.with_conn_sync(|c| {
                barrier.wait();
                let n: i64 = c.query_row("SELECT 1", [], |row| row.get(0))?;
                std::thread::sleep(std::time::Duration::from_millis(50));
                Ok(n)
            })
        });

        let t2 = std::thread::spawn(move || {
            barrier2.wait();
            std::thread::sleep(std::time::Duration::from_millis(10));
            handle2.with_conn_sync(|c| {
                let n: i64 = c.query_row("SELECT 2", [], |row| row.get(0))?;
                Ok(n)
            })
        });

        assert_eq!(t1.join().unwrap().unwrap(), 1);
        assert_eq!(t2.join().unwrap().unwrap(), 2);
    }

    #[tokio::test]
    async fn concurrent_async_contention_succeeds() {
        let conn = Connection::open_in_memory().unwrap();
        let handle = DbHandle::new(conn);
        let h1 = handle.clone();
        let h2 = handle.clone();

        let (r1, r2) = tokio::join!(
            h1.with_conn(|c| {
                let n: i64 = c.query_row("SELECT 10", [], |row| row.get(0))?;
                Ok(n)
            }),
            h2.with_conn(|c| {
                let n: i64 = c.query_row("SELECT 20", [], |row| row.get(0))?;
                Ok(n)
            }),
        );

        assert_eq!(r1.unwrap(), 10);
        assert_eq!(r2.unwrap(), 20);
    }
}
```

- [ ] **Step 2: Update lib.rs to declare handle module**

```rust
// core/storage/src/lib.rs

//! iotkit-core-storage: SQLite persistence infrastructure for the IoT gateway.

mod error;
mod handle;

pub use error::StorageError;
pub use handle::DbHandle;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p iotkit-core-storage`
Expected: All 5 handle tests pass.

- [ ] **Step 4: Commit**

```bash
git add core/storage/src/handle.rs core/storage/src/lib.rs
git commit -m "feat(iotkit-core-storage): add DbHandle with reentrancy guard"
```

---

### Task 3: Migration Harness

**Depends on:** Task 2

**Files:**
- Create: `core/storage/migrations/0001_init.sql`
- Create: `core/storage/src/migrate.rs`
- Modify: `core/storage/src/lib.rs`

- [ ] **Step 1: Create baseline migration file**

```sql
-- core/storage/migrations/0001_init.sql
-- Baseline migration for iotkit-core-storage.
-- Application tables are added by downstream issues (#22, #23).
```

- [ ] **Step 2: Create migrate.rs with implementation + tests**

Create `core/storage/src/migrate.rs` with the full migration harness and test module. Declare `mod migrate;` in `lib.rs` in the same step.

```rust
// core/storage/src/migrate.rs

use rusqlite::Connection;

use crate::StorageError;

struct Migration {
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
    fn version(&self) -> u32 { self.version }
    fn label(&self) -> &str { self.label }
    fn sql(&self) -> &str { self.sql }
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
        let tx = conn.unchecked_transaction().map_err(|e| StorageError::MigrationFailed {
            version: m.version(),
            source: Box::new(e.into()),
        })?;
        tx.execute_batch(m.sql()).map_err(|e| StorageError::MigrationFailed {
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
    fn version(&self) -> u32 { self.version }
    fn label(&self) -> &str { self.label }
    fn sql(&self) -> &str { self.sql }
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
            matches!(result, Err(StorageError::SchemaVersionAhead { on_disk: 9999, .. })),
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
                // First statement succeeds, second fails — tests partial rollback
                sql: "CREATE TABLE half_done (id INTEGER PRIMARY KEY);\nTHIS IS NOT VALID SQL;",
            },
        ];

        // Apply with intentionally broken second migration
        let result = run_migrations_with(&conn, test_migrations);

        // Must return MigrationFailed for version 2
        assert!(
            matches!(result, Err(StorageError::MigrationFailed { version: 2, .. })),
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
        assert!(!half_done_exists, "v2 partial changes should be rolled back");
    }
}
```

- [ ] **Step 3: Update lib.rs to declare migrate module**

```rust
// core/storage/src/lib.rs

//! iotkit-core-storage: SQLite persistence infrastructure for the IoT gateway.

mod error;
mod handle;
mod migrate;

pub use error::StorageError;
pub use handle::DbHandle;
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p iotkit-core-storage`
Expected: All tests pass (handle tests from Task 2 + migration tests including `migration_failure_rolls_back`).

- [ ] **Step 5: Commit**

```bash
git add core/storage/migrations/ core/storage/src/migrate.rs core/storage/src/lib.rs
git commit -m "feat(iotkit-core-storage): add migration harness with version tracking"
```

---

### Task 4: init_db + init_db_memory

**Depends on:** Tasks 2 and 3

**Files:**
- Modify: `core/storage/src/lib.rs`

- [ ] **Step 1: Add init_db, init_db_memory, and tests to lib.rs**

```rust
// core/storage/src/lib.rs

//! iotkit-core-storage: SQLite persistence infrastructure for the IoT gateway.

mod error;
mod handle;
mod migrate;

use std::path::Path;

use rusqlite::Connection;

pub use error::StorageError;
pub use handle::DbHandle;

/// Open (or create) the database, configure pragmas, run migrations.
/// Sole public entry point for obtaining a DbHandle.
/// Synchronous — call before entering the async runtime.
pub fn init_db(db_path: &Path) -> Result<DbHandle, StorageError> {
    // Check parent directory exists — surface as StorageError::Io, not Sqlite.
    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err(StorageError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "parent directory does not exist: {}",
                    parent.display()
                ),
            )));
        }
    }
    let conn = Connection::open(db_path)?;
    configure_and_migrate(conn)
}

/// Open an in-memory database with all migrations applied.
/// For tests only.
#[cfg(any(test, feature = "test-util"))]
pub fn init_db_memory() -> Result<DbHandle, StorageError> {
    let conn = Connection::open_in_memory()?;
    configure_and_migrate(conn)
}

fn configure_and_migrate(conn: Connection) -> Result<DbHandle, StorageError> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;
    migrate::run_migrations(&conn)?;
    Ok(DbHandle::new(conn))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_db_creates_and_migrates() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = init_db(&db_path).unwrap();

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
        let _db1 = init_db(&db_path).unwrap();
        drop(_db1);
        let _db2 = init_db(&db_path).unwrap();
    }

    #[test]
    fn init_db_missing_parent_returns_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let bad_path = dir.path().join("nonexistent_subdir").join("test.db");
        let result = init_db(&bad_path);
        assert!(
            matches!(result, Err(StorageError::Io(_))),
            "expected StorageError::Io for missing parent, got {result:?}"
        );
    }

    #[test]
    fn init_db_memory_succeeds() {
        let db = init_db_memory().unwrap();
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
    fn pragma_verification_file_backed() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = init_db(&db_path).unwrap();

        db.with_conn_sync(|conn| {
            let journal_mode: String =
                conn.query_row("PRAGMA journal_mode", [], |row| row.get(0)).unwrap();
            assert_eq!(journal_mode, "wal");

            let synchronous: i32 =
                conn.query_row("PRAGMA synchronous", [], |row| row.get(0)).unwrap();
            assert_eq!(synchronous, 2); // FULL

            let foreign_keys: i32 =
                conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0)).unwrap();
            assert_eq!(foreign_keys, 1);

            let busy_timeout: i32 =
                conn.query_row("PRAGMA busy_timeout", [], |row| row.get(0)).unwrap();
            assert_eq!(busy_timeout, 5000);

            Ok(())
        })
        .unwrap();
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p iotkit-core-storage`
Expected: All tests pass (handle + migration + init_db tests).

- [ ] **Step 3: Commit**

```bash
git add core/storage/src/lib.rs
git commit -m "feat(iotkit-core-storage): add init_db and init_db_memory entry points"
```

---

### Task 5: Gateway Integration

**Depends on:** Task 4

**Files:**
- Modify: `iotkit-gateway/Cargo.toml`
- Modify: `iotkit-gateway/src/main.rs`

- [ ] **Step 1: Add iotkit-core-storage dependency to gateway**

In `iotkit-gateway/Cargo.toml`, add to `[dependencies]`:

```toml
iotkit-core-storage = { path = "../core/storage" }
```

- [ ] **Step 2: Modify main() to call init_db before runtime**

In `iotkit-gateway/src/main.rs`, after the config effective-config log block and before `let rt = tokio::runtime::Runtime::new()`, add:

```rust
    let db = match iotkit_core_storage::init_db(std::path::Path::new(&config.db_path)) {
        Ok(handle) => handle,
        Err(e) => {
            tracing::error!(error = %e, db_path = %config.db_path, "failed to initialize database");
            std::process::exit(1);
        }
    };
    tracing::info!(db_path = %config.db_path, "database initialized");
```

- [ ] **Step 3: Change run() signature to accept DbHandle**

Change:
```rust
async fn run(config: config::GatewayConfig) {
```
To:
```rust
async fn run(config: config::GatewayConfig, _db: iotkit_core_storage::DbHandle) {
```

- [ ] **Step 4: Update rt.block_on call**

Change:
```rust
    rt.block_on(run(config));
```
To:
```rust
    rt.block_on(run(config, db));
```

- [ ] **Step 5: Verify full build**

Run: `cargo check --workspace`
Expected: Compiles with no errors.

- [ ] **Step 6: Run all workspace tests**

Run: `cargo test --workspace`
Expected: All tests pass (existing + new iotkit-core-storage tests).

- [ ] **Step 7: Commit**

```bash
git add iotkit-gateway/Cargo.toml iotkit-gateway/src/main.rs
git commit -m "feat(iotkit-gateway): integrate iotkit-core-storage init_db at startup"
```

---

## Self-Review Checklist

**Spec coverage:**
- [x] `iotkit-core-storage` crate — Task 1
- [x] `StorageError` with all 4 variants — Task 1
- [x] `DbHandle` with `with_conn` + `with_conn_sync` — Task 2
- [x] Reentrancy guard (handle-scoped, thread-local) — Task 2
- [x] Migration harness (`run_migrations`, `_schema_version` table) — Task 3
- [x] `MIGRATIONS` array with invariant test — Task 3
- [x] `0001_init.sql` baseline — Task 3
- [x] `init_db(&Path)` entry point with parent dir check → `StorageError::Io` — Task 4
- [x] `init_db_memory()` test helper with `test-util` feature — Task 4
- [x] PRAGMA configuration (WAL, synchronous, foreign_keys, busy_timeout) — Task 4
- [x] Gateway `main.rs` integration — Task 5
- [x] `run()` signature change — Task 5
- [x] Error handling (tracing::error + exit 1) — Task 5
- [x] Fresh DB migration test — Task 3
- [x] Already-migrated no-op test — Task 3
- [x] Schema version ahead test — Task 3
- [x] Migration failure rollback test — Task 3
- [x] PRAGMA verification test (all 4 pragmas) — Task 4
- [x] Missing parent → StorageError::Io test — Task 4
- [x] with_conn async test — Task 2
- [x] with_conn_sync test — Task 2
- [x] Reentry panic test — Task 2
- [x] Cross-thread contention test — Task 2
- [x] Concurrent async contention test — Task 2
- [x] MIGRATIONS array invariants test — Task 3
- [x] Idempotent reopen test — Task 4

**Placeholder scan:** No TBD/TODO found.

**Type consistency:** `StorageError`, `DbHandle`, `init_db`, `init_db_memory`, `run_migrations`, `run_migrations_with`, `Migration`, `TestMigration`, `MIGRATIONS` — names consistent across all tasks. Test name `test_migrations_array_invariants` matches spec.
