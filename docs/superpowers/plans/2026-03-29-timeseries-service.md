# Timeseries Service v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist sensor readings to SQLite via a new `core/timeseries` crate and wire it into the gateway event loop.

**Architecture:** Refactor `core/storage` to expose public `Migration` type and accept external migrations, add `SensorType` DB mapping to `core/types`, create new `core/timeseries` crate with insert/query/delete APIs, and integrate into the gateway event loop with sequential fan-out.

**Tech Stack:** Rust 2024, rusqlite 0.32 (bundled), serde_json 1, tokio 1.x, tracing 0.1

**Task ordering:** Sequential: 1 → 2 → 3 → 4 → 5 → 6. Inner-to-outer: storage refactor → types extension → new timeseries crate → gateway integration.

---

## File Structure

| Action | Path | Responsibility |
|---|---|---|
| Modify | `core/storage/src/error.rs` | Add `InvalidMigrationOrder` variant |
| Modify | `core/storage/src/migrate.rs` | Make `Migration` pub with `Clone, Copy`, `run_migrations` pub with validation, accept `&[Migration]` |
| Modify | `core/storage/src/lib.rs` | Change `init_db`/`init_db_memory` signatures, add re-exports, add `configure_pragmas` + `cache_size` PRAGMA |
| Modify | `core/types/src/lib.rs` | Add `SensorType::as_db_str()` and `from_db_str()` methods |
| Create | `core/timeseries/Cargo.toml` | Crate manifest |
| Create | `core/timeseries/src/error.rs` | `TimeseriesError` enum |
| Create | `core/timeseries/src/model.rs` | `ReadingRow`, `TimeRange` types |
| Create | `core/timeseries/src/lib.rs` | Public API: `insert_reading`, `query_readings`, `latest_reading`, `delete_before`, `MIGRATIONS` |
| Create | `core/timeseries/migrations/0002_timeseries.sql` | Table DDL |
| Modify | `Cargo.toml` (workspace root) | Add `"core/timeseries"` to members |
| Modify | `iotkit-gateway/Cargo.toml` | Add `iotkit-core-timeseries` dependency |
| Modify | `iotkit-gateway/src/main.rs` | Migration assembly, event loop fan-out with timeseries writes |

---

### Task 1: core/storage — Make Migration Public + Validation

**Files:**
- Modify: `core/storage/src/error.rs`
- Modify: `core/storage/src/migrate.rs`
- Modify: `core/storage/src/lib.rs`

- [ ] **Step 1: Add `InvalidMigrationOrder` to `StorageError`**

In `core/storage/src/error.rs`, add the new variant and its Display/Error impls:

```rust
// Add to the StorageError enum after SchemaVersionAhead:
    /// Migration versions are not strictly ascending.
    InvalidMigrationOrder { first: u32, second: u32 },
```

Add the Display match arm:

```rust
            Self::InvalidMigrationOrder { first, second } => {
                write!(
                    f,
                    "migration versions not strictly ascending: v{first} >= v{second}"
                )
            }
```

Add the `source()` match arm:

```rust
            Self::InvalidMigrationOrder { .. } => None,
```

- [ ] **Step 2: Make `Migration` struct public with `Clone, Copy`**

In `core/storage/src/migrate.rs`, change:

```rust
// FROM:
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

// TO:
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
```

- [ ] **Step 3: Implement `MigrationEntry` for public `Migration`**

The existing `MigrationEntry` trait impl already exists. Because the fields are now `pub`, the trait impl still works. Verify the trait impl reads:

```rust
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
```

No changes needed — the trait and impl are `pub(crate)` which is fine.

- [ ] **Step 4: Make `run_migrations` public with validation**

Change the existing `run_migrations` function:

```rust
// FROM:
/// Production entry point: runs MIGRATIONS through the shared inner runner.
pub(crate) fn run_migrations(conn: &Connection) -> Result<(), StorageError> {
    run_migrations_inner(conn, MIGRATIONS)
}

// TO:
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
```

- [ ] **Step 5: Update `lib.rs` — split `configure_and_migrate`, change signatures, add re-exports**

Replace the entire `core/storage/src/lib.rs` with:

```rust
//! iotkit-core-storage: SQLite persistence infrastructure for the IoT gateway.

mod error;
mod handle;
mod migrate;

use std::path::Path;

use rusqlite::Connection;

pub use error::StorageError;
pub use handle::DbHandle;
pub use migrate::{Migration, MIGRATIONS, run_migrations};

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
    fn pragma_verification_file_backed() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = init_db(&db_path, MIGRATIONS).unwrap();

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

            let cache_size: i32 =
                conn.query_row("PRAGMA cache_size", [], |row| row.get(0)).unwrap();
            assert_eq!(cache_size, -8000);

            Ok(())
        })
        .unwrap();
    }
}
```

- [ ] **Step 6: Add migration validation tests to `migrate.rs`**

Add these tests to the existing `#[cfg(test)] mod tests` in `core/storage/src/migrate.rs`:

```rust
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
```

- [ ] **Step 7: Update existing `test_migrations_array_invariants` test**

The existing test directly accesses `m.version` which now works since the field is `pub`. No changes needed.

However, the existing `run_migrations(&conn)` calls in tests that use the old signature (no `migrations` arg) need updating. Change all `run_migrations(&conn)` calls in the existing tests to `run_migrations(&conn, MIGRATIONS)`:

```rust
    // In fresh_db_gets_migrated:
    run_migrations(&conn, MIGRATIONS).unwrap();

    // In already_migrated_is_noop:
    run_migrations(&conn, MIGRATIONS).unwrap();
    run_migrations(&conn, MIGRATIONS).unwrap();

    // In schema_version_ahead_rejected:
    let result = run_migrations(&conn, MIGRATIONS);
```

The `migration_failure_rolls_back` test uses `run_migrations_with` which is unchanged.

- [ ] **Step 8: Run tests**

Run: `cargo test -p iotkit-core-storage`
Expected: All tests pass, including new migration validation tests and updated pragma verification (cache_size).

- [ ] **Step 9: Commit**

```bash
git add core/storage/src/error.rs core/storage/src/migrate.rs core/storage/src/lib.rs
git commit -m "refactor(core/storage): make Migration pub, accept external migrations

- Migration struct is now pub with Clone, Copy and pub fields
- run_migrations is now pub, accepts &[Migration], validates ascending order
- init_db/init_db_memory accept &[Migration] parameter
- Split configure_and_migrate into configure_pragmas + run_migrations
- Add PRAGMA cache_size = -8000 (8MB page cache)
- Add StorageError::InvalidMigrationOrder variant
- Add migration validation tests"
```

---

### Task 2: core/types — SensorType DB Mapping

**Files:**
- Modify: `core/types/src/lib.rs`

- [ ] **Step 1: Write failing tests for `as_db_str` / `from_db_str`**

Add to the existing `#[cfg(test)] mod tests` in `core/types/src/lib.rs`:

```rust
    #[test]
    fn sensor_type_db_str_round_trip() {
        let variants: Vec<SensorType> = vec![
            SensorType::ContactInput,
            SensorType::ContactOutput,
            SensorType::Adc,
            SensorType::Ranging,
            SensorType::Temperature,
            SensorType::Acceleration,
            SensorType::DifferentialPressure,
            SensorType::Illuminance,
        ];
        for v in variants {
            let db_str = v.as_db_str();
            let round_tripped = SensorType::from_db_str(db_str);
            assert_eq!(v, round_tripped, "round-trip failed for {v:?} -> {db_str:?}");
        }
    }

    #[test]
    fn sensor_type_unknown_round_trip() {
        let original = SensorType::Unknown("custom_xyz".to_string());
        let db_str = original.as_db_str();
        assert_eq!(db_str, "custom_xyz");
        let round_tripped = SensorType::from_db_str(db_str);
        assert_eq!(round_tripped, SensorType::Unknown("custom_xyz".to_string()));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p iotkit-core-types`
Expected: FAIL — `as_db_str` and `from_db_str` do not exist yet.

- [ ] **Step 3: Implement `as_db_str` and `from_db_str`**

Add to `core/types/src/lib.rs`, after the existing `impl fmt::Display for SensorType` block:

```rust
impl SensorType {
    /// Convert to the string stored in SQLite sensor_type column.
    pub fn as_db_str(&self) -> &str {
        match self {
            Self::ContactInput => "contact_input",
            Self::ContactOutput => "contact_output",
            Self::Adc => "adc",
            Self::Ranging => "ranging",
            Self::Temperature => "temperature",
            Self::Acceleration => "acceleration",
            Self::DifferentialPressure => "differential_pressure",
            Self::Illuminance => "illuminance",
            Self::Unknown(s) => s.as_str(),
        }
    }

    /// Parse from the string stored in SQLite sensor_type column.
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "contact_input" => Self::ContactInput,
            "contact_output" => Self::ContactOutput,
            "adc" => Self::Adc,
            "ranging" => Self::Ranging,
            "temperature" => Self::Temperature,
            "acceleration" => Self::Acceleration,
            "differential_pressure" => Self::DifferentialPressure,
            "illuminance" => Self::Illuminance,
            other => Self::Unknown(other.to_string()),
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p iotkit-core-types`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add core/types/src/lib.rs
git commit -m "feat(core/types): add SensorType DB string mapping

Add as_db_str() and from_db_str() for SQLite sensor_type column.
Known variants map to snake_case strings, Unknown stores raw string."
```

---

### Task 3: core/timeseries — Crate Scaffold + Error + Model

**Files:**
- Create: `core/timeseries/Cargo.toml`
- Create: `core/timeseries/src/error.rs`
- Create: `core/timeseries/src/model.rs`
- Create: `core/timeseries/src/lib.rs` (initial — re-exports only)
- Create: `core/timeseries/migrations/0002_timeseries.sql`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create directory structure and `Cargo.toml`**

```toml
# core/timeseries/Cargo.toml
[package]
name = "iotkit-core-timeseries"
version = "0.1.0"
edition = "2024"

[dependencies]
iotkit-core-types = { path = "../types" }
iotkit-core-storage = { path = "../storage", features = ["test-util"] }
serde_json = "1"
tracing = "0.1"

[dev-dependencies]
tokio = { version = "1", features = ["rt", "macros"] }
```

- [ ] **Step 2: Add to workspace members**

In `Cargo.toml` (workspace root), add `"core/timeseries"` to the `members` list:

```toml
members = [
    "core/types",
    "core/engine",
    "core/storage",
    "core/timeseries",
    "rpi4b-driver/transport",
    "bravepi-mainboard-adapter",
    "bravepi-mainboard-adapter/codec",
    "bravepi-mainboard-adapter/sensors",
    "bravepi-mainboard-adapter/poc",
    "rpi-local-adapter",
    "iotkit-gateway",
    "iotkit-polling-adapter-runtime",
]
```

- [ ] **Step 3: Create migration SQL**

```sql
-- core/timeseries/migrations/0002_timeseries.sql

CREATE TABLE sensor_readings (
    adapter_id  TEXT    NOT NULL,
    device_key  TEXT    NOT NULL,
    ingested_at INTEGER NOT NULL,
    sensor_type TEXT    NOT NULL,
    values_json TEXT    NOT NULL,
    rssi        INTEGER,
    battery_pct INTEGER,
    PRIMARY KEY (adapter_id, device_key, ingested_at, sensor_type)
) WITHOUT ROWID;
```

- [ ] **Step 4: Create `error.rs`**

```rust
// core/timeseries/src/error.rs

use iotkit_core_storage::StorageError;

/// Errors from timeseries operations.
#[derive(Debug)]
pub enum TimeseriesError {
    /// Invalid reading data (NaN, Inf, pre-epoch timestamp, invalid range, etc.)
    InvalidReading(String),
    /// Underlying storage error.
    Storage(StorageError),
}

impl std::fmt::Display for TimeseriesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidReading(msg) => write!(f, "invalid reading: {msg}"),
            Self::Storage(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TimeseriesError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidReading(_) => None,
            Self::Storage(e) => Some(e),
        }
    }
}

impl From<StorageError> for TimeseriesError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}
```

- [ ] **Step 5: Create `model.rs`**

```rust
// core/timeseries/src/model.rs

use std::time::SystemTime;

use iotkit_core_types::SensorType;

/// A single reading row from the sensor_readings table.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadingRow {
    pub adapter_id: String,
    pub device_key: String,
    /// Unix milliseconds since epoch (1970-01-01T00:00:00Z).
    pub ingested_at: i64,
    pub sensor_type: SensorType,
    pub values: Vec<f64>,
    pub rssi: Option<i16>,
    pub battery_pct: Option<u8>,
}

/// Time range for queries.
#[derive(Debug, Clone)]
pub struct TimeRange {
    /// Inclusive start.
    pub start: SystemTime,
    /// Exclusive end.
    pub end: SystemTime,
}
```

- [ ] **Step 6: Create initial `lib.rs` with re-exports and MIGRATIONS**

```rust
// core/timeseries/src/lib.rs

//! iotkit-core-timeseries: sensor reading persistence (INSERT/query/delete).

mod error;
mod model;

pub use error::TimeseriesError;
pub use model::{ReadingRow, TimeRange};

use iotkit_core_storage::Migration;

/// Timeseries migrations. Append to core/storage MIGRATIONS when assembling.
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 2,
    label: "timeseries",
    sql: include_str!("../migrations/0002_timeseries.sql"),
}];
```

- [ ] **Step 7: Verify it compiles**

Run: `cargo check -p iotkit-core-timeseries`
Expected: Compiles with no errors.

- [ ] **Step 8: Commit**

```bash
git add core/timeseries/ Cargo.toml
git commit -m "feat(core/timeseries): scaffold crate with error, model, migration SQL

New crate for timeseries persistence. Contains:
- TimeseriesError (InvalidReading, Storage)
- ReadingRow, TimeRange model types
- sensor_readings table DDL (migration v2)"
```

---

### Task 4: core/timeseries — insert_reading + Tests

**Files:**
- Modify: `core/timeseries/src/lib.rs`

- [ ] **Step 1: Write failing tests for `insert_reading`**

Add to `core/timeseries/src/lib.rs`:

```rust
use std::time::{SystemTime, UNIX_EPOCH};

use iotkit_core_storage::DbHandle;
use iotkit_core_types::{AdapterId, DeviceKey, SensorType};

/// Helper: convert SystemTime to unix milliseconds.
fn system_time_to_millis(t: SystemTime) -> Result<i64, TimeseriesError> {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .map_err(|_| TimeseriesError::InvalidReading("timestamp before epoch".to_string()))
}

pub async fn insert_reading(
    _db: &DbHandle,
    _adapter_id: &AdapterId,
    _device_key: &DeviceKey,
    _ingested_at: SystemTime,
    _sensor_type: &SensorType,
    _values: &[f64],
    _rssi: Option<i16>,
    _battery_pct: Option<u8>,
) -> Result<(), TimeseriesError> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn all_migrations() -> Vec<Migration> {
        let mut m = Vec::from(iotkit_core_storage::MIGRATIONS);
        m.extend_from_slice(MIGRATIONS);
        m
    }

    fn test_db() -> DbHandle {
        iotkit_core_storage::init_db_memory(&all_migrations()).unwrap()
    }

    fn ts(millis: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(millis)
    }

    #[tokio::test]
    async fn reject_nan_in_values() {
        let db = test_db();
        let result = insert_reading(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            ts(1000),
            &SensorType::Temperature,
            &[f64::NAN],
            None,
            None,
        )
        .await;
        assert!(matches!(result, Err(TimeseriesError::InvalidReading(msg)) if msg.contains("NaN")));
    }

    #[tokio::test]
    async fn reject_infinity_in_values() {
        let db = test_db();
        let result = insert_reading(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            ts(1000),
            &SensorType::Temperature,
            &[f64::INFINITY],
            None,
            None,
        )
        .await;
        assert!(matches!(result, Err(TimeseriesError::InvalidReading(msg)) if msg.contains("Inf")));
    }

    #[tokio::test]
    async fn reject_pre_epoch_timestamp() {
        let db = test_db();
        // SystemTime before epoch — use UNIX_EPOCH - 1s
        let pre_epoch = UNIX_EPOCH - Duration::from_secs(1);
        let result = insert_reading(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            pre_epoch,
            &SensorType::Temperature,
            &[25.0],
            None,
            None,
        )
        .await;
        assert!(matches!(result, Err(TimeseriesError::InvalidReading(msg)) if msg.contains("epoch")));
    }

    #[tokio::test]
    async fn insert_succeeds() {
        let db = test_db();
        insert_reading(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            ts(1000),
            &SensorType::Temperature,
            &[25.3],
            Some(-50),
            Some(85),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn insert_multiple_sensor_types_same_timestamp() {
        let db = test_db();
        let t = ts(1000);
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), t, &SensorType::Temperature, &[25.3], None, None).await.unwrap();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), t, &SensorType::Acceleration, &[0.1, -0.3, 9.8], None, None).await.unwrap();
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p iotkit-core-timeseries`
Expected: FAIL — `insert_reading` panics on `todo!()`.

- [ ] **Step 3: Implement `insert_reading`**

Replace the `insert_reading` function body:

```rust
pub async fn insert_reading(
    db: &DbHandle,
    adapter_id: &AdapterId,
    device_key: &DeviceKey,
    ingested_at: SystemTime,
    sensor_type: &SensorType,
    values: &[f64],
    rssi: Option<i16>,
    battery_pct: Option<u8>,
) -> Result<(), TimeseriesError> {
    // Validate values
    for (i, v) in values.iter().enumerate() {
        if v.is_nan() || v.is_infinite() {
            return Err(TimeseriesError::InvalidReading(format!(
                "NaN/Inf in values at index {i}"
            )));
        }
    }

    // Convert timestamp
    let millis = system_time_to_millis(ingested_at)?;

    // Serialize values to JSON
    let values_json = serde_json::to_string(values)
        .map_err(|e| TimeseriesError::InvalidReading(format!("JSON serialization failed: {e}")))?;

    // Prepare owned values for the closure
    let adapter_id_str = adapter_id.as_str().to_string();
    let device_key_str = device_key.as_str().to_string();
    let sensor_type_str = sensor_type.as_db_str().to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO sensor_readings (adapter_id, device_key, ingested_at, sensor_type, values_json, rssi, battery_pct)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                adapter_id_str,
                device_key_str,
                millis,
                sensor_type_str,
                values_json,
                rssi,
                battery_pct.map(|b| b as i32),
            ],
        )?;
        Ok(())
    })
    .await?;

    Ok(())
}
```

Add `use iotkit_core_storage::StorageError;` if not already imported (needed for the `?` on `with_conn` to go through `From<StorageError>`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p iotkit-core-timeseries`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add core/timeseries/src/lib.rs
git commit -m "feat(core/timeseries): implement insert_reading with validation

Validates NaN/Inf in values and pre-epoch timestamps.
Serializes values to JSON, writes to sensor_readings table."
```

---

### Task 5: core/timeseries — query_readings, latest_reading, delete_before + Tests

**Files:**
- Modify: `core/timeseries/src/lib.rs`

- [ ] **Step 1: Write failing tests for `query_readings`**

Add to the `tests` module in `core/timeseries/src/lib.rs`:

```rust
    #[tokio::test]
    async fn insert_and_query_single() {
        let db = test_db();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000), &SensorType::Temperature, &[25.3], Some(-50), Some(85)).await.unwrap();

        let rows = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            None,
            TimeRange { start: ts(0), end: ts(2000) },
            100,
        ).await.unwrap();

        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.adapter_id, "a1");
        assert_eq!(r.device_key, "d1");
        assert_eq!(r.ingested_at, 1000);
        assert_eq!(r.sensor_type, SensorType::Temperature);
        assert_eq!(r.values, vec![25.3]);
        assert_eq!(r.rssi, Some(-50));
        assert_eq!(r.battery_pct, Some(85));
    }

    #[tokio::test]
    async fn query_with_sensor_type_filter() {
        let db = test_db();
        let t = ts(1000);
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), t, &SensorType::Temperature, &[25.3], None, None).await.unwrap();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), t, &SensorType::Acceleration, &[0.1, -0.3, 9.8], None, None).await.unwrap();

        let rows = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            Some(&SensorType::Temperature),
            TimeRange { start: ts(0), end: ts(2000) },
            100,
        ).await.unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sensor_type, SensorType::Temperature);
    }

    #[tokio::test]
    async fn query_time_range() {
        let db = test_db();
        for i in 0..5 {
            insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000 + i * 1000), &SensorType::Temperature, &[20.0 + i as f64], None, None).await.unwrap();
        }

        let rows = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            None,
            TimeRange { start: ts(2000), end: ts(4000) },
            100,
        ).await.unwrap();

        assert_eq!(rows.len(), 2); // ts 2000 and 3000
    }

    #[tokio::test]
    async fn query_respects_limit() {
        let db = test_db();
        for i in 0..10 {
            insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000 + i * 1000), &SensorType::Temperature, &[20.0], None, None).await.unwrap();
        }

        let rows = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            None,
            TimeRange { start: ts(0), end: ts(100_000) },
            3,
        ).await.unwrap();

        assert_eq!(rows.len(), 3);
    }

    #[tokio::test]
    async fn query_returns_newest_first() {
        let db = test_db();
        for i in 0..3 {
            insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000 + i * 1000), &SensorType::Temperature, &[20.0], None, None).await.unwrap();
        }

        let rows = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            None,
            TimeRange { start: ts(0), end: ts(100_000) },
            100,
        ).await.unwrap();

        assert!(rows[0].ingested_at > rows[1].ingested_at);
        assert!(rows[1].ingested_at > rows[2].ingested_at);
    }

    #[tokio::test]
    async fn query_rejects_invalid_range() {
        let db = test_db();
        let result = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            None,
            TimeRange { start: ts(2000), end: ts(1000) },
            100,
        ).await;
        assert!(matches!(result, Err(TimeseriesError::InvalidReading(msg)) if msg.contains("start >= end")));
    }

    #[tokio::test]
    async fn values_json_round_trip() {
        let db = test_db();
        let values = vec![0.1, -0.3, 9.8];
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000), &SensorType::Acceleration, &values, None, None).await.unwrap();

        let rows = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            None,
            TimeRange { start: ts(0), end: ts(2000) },
            100,
        ).await.unwrap();

        assert_eq!(rows[0].values, values);
    }
```

- [ ] **Step 2: Write failing tests for `latest_reading`**

```rust
    #[tokio::test]
    async fn latest_reading_returns_most_recent() {
        let db = test_db();
        for i in 0..3 {
            insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000 + i * 1000), &SensorType::Temperature, &[20.0 + i as f64], None, None).await.unwrap();
        }

        let row = latest_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), None).await.unwrap().unwrap();
        assert_eq!(row.ingested_at, 3000);
        assert_eq!(row.values, vec![22.0]);
    }

    #[tokio::test]
    async fn latest_reading_empty() {
        let db = test_db();
        let row = latest_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), None).await.unwrap();
        assert!(row.is_none());
    }

    #[tokio::test]
    async fn latest_reading_with_sensor_type_filter() {
        let db = test_db();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000), &SensorType::Temperature, &[25.0], None, None).await.unwrap();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(2000), &SensorType::Acceleration, &[0.1], None, None).await.unwrap();

        let row = latest_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), Some(&SensorType::Temperature)).await.unwrap().unwrap();
        assert_eq!(row.sensor_type, SensorType::Temperature);
        assert_eq!(row.ingested_at, 1000);
    }
```

- [ ] **Step 3: Write failing tests for `delete_before`**

```rust
    #[tokio::test]
    async fn delete_before_removes_old() {
        let db = test_db();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000), &SensorType::Temperature, &[20.0], None, None).await.unwrap();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(5000), &SensorType::Temperature, &[25.0], None, None).await.unwrap();

        delete_before(&db, ts(3000)).await.unwrap();

        let rows = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            None,
            TimeRange { start: ts(0), end: ts(100_000) },
            100,
        ).await.unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ingested_at, 5000);
    }

    #[tokio::test]
    async fn delete_before_returns_count() {
        let db = test_db();
        for i in 0..5 {
            insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000 + i * 1000), &SensorType::Temperature, &[20.0], None, None).await.unwrap();
        }

        let deleted = delete_before(&db, ts(3500)).await.unwrap();
        assert_eq!(deleted, 3); // ts 1000, 2000, 3000
    }
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p iotkit-core-timeseries`
Expected: FAIL — `query_readings`, `latest_reading`, `delete_before` not implemented.

- [ ] **Step 5: Implement `query_readings`**

Add to `core/timeseries/src/lib.rs`:

```rust
/// Helper: parse a row from sensor_readings into ReadingRow.
fn row_to_reading(row: &rusqlite::Row<'_>) -> Result<ReadingRow, rusqlite::Error> {
    let adapter_id: String = row.get(0)?;
    let device_key: String = row.get(1)?;
    let ingested_at: i64 = row.get(2)?;
    let sensor_type_str: String = row.get(3)?;
    let values_json: String = row.get(4)?;
    let rssi: Option<i16> = row.get(5)?;
    let battery_pct: Option<i32> = row.get(6)?;

    let sensor_type = SensorType::from_db_str(&sensor_type_str);
    let values: Vec<f64> = serde_json::from_str(&values_json).unwrap_or_default();

    Ok(ReadingRow {
        adapter_id,
        device_key,
        ingested_at,
        sensor_type,
        values,
        rssi,
        battery_pct: battery_pct.map(|b| b as u8),
    })
}

pub async fn query_readings(
    db: &DbHandle,
    adapter_id: &AdapterId,
    device_key: &DeviceKey,
    sensor_type: Option<&SensorType>,
    range: TimeRange,
    limit: u32,
) -> Result<Vec<ReadingRow>, TimeseriesError> {
    // Validate range
    if range.start >= range.end {
        return Err(TimeseriesError::InvalidReading(
            "start >= end in time range".to_string(),
        ));
    }

    let start_millis = system_time_to_millis(range.start)?;
    let end_millis = system_time_to_millis(range.end)?;
    let adapter_id_str = adapter_id.as_str().to_string();
    let device_key_str = device_key.as_str().to_string();
    let sensor_type_str = sensor_type.map(|st| st.as_db_str().to_string());

    db.with_conn(move |conn| {
        let rows = if let Some(ref st) = sensor_type_str {
            let mut stmt = conn.prepare(
                "SELECT adapter_id, device_key, ingested_at, sensor_type, values_json, rssi, battery_pct
                 FROM sensor_readings
                 WHERE adapter_id = ?1 AND device_key = ?2 AND sensor_type = ?3
                   AND ingested_at >= ?4 AND ingested_at < ?5
                 ORDER BY ingested_at DESC, sensor_type ASC
                 LIMIT ?6",
            )?;
            stmt.query_map(
                rusqlite::params![adapter_id_str, device_key_str, st, start_millis, end_millis, limit],
                row_to_reading,
            )?
            .collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT adapter_id, device_key, ingested_at, sensor_type, values_json, rssi, battery_pct
                 FROM sensor_readings
                 WHERE adapter_id = ?1 AND device_key = ?2
                   AND ingested_at >= ?3 AND ingested_at < ?4
                 ORDER BY ingested_at DESC, sensor_type ASC
                 LIMIT ?5",
            )?;
            stmt.query_map(
                rusqlite::params![adapter_id_str, device_key_str, start_millis, end_millis, limit],
                row_to_reading,
            )?
            .collect::<Result<Vec<_>, _>>()?
        };
        Ok(rows)
    })
    .await?;

    // Need to return the result — fix the above:
    unreachable!()
}
```

Wait, the `with_conn` returns `Result<Vec<ReadingRow>, StorageError>`, which then converts via `From<StorageError>`. Let me fix this properly:

```rust
pub async fn query_readings(
    db: &DbHandle,
    adapter_id: &AdapterId,
    device_key: &DeviceKey,
    sensor_type: Option<&SensorType>,
    range: TimeRange,
    limit: u32,
) -> Result<Vec<ReadingRow>, TimeseriesError> {
    if range.start >= range.end {
        return Err(TimeseriesError::InvalidReading(
            "start >= end in time range".to_string(),
        ));
    }

    let start_millis = system_time_to_millis(range.start)?;
    let end_millis = system_time_to_millis(range.end)?;
    let adapter_id_str = adapter_id.as_str().to_string();
    let device_key_str = device_key.as_str().to_string();
    let sensor_type_str = sensor_type.map(|st| st.as_db_str().to_string());

    let rows = db
        .with_conn(move |conn| {
            let rows = if let Some(ref st) = sensor_type_str {
                let mut stmt = conn.prepare(
                    "SELECT adapter_id, device_key, ingested_at, sensor_type, values_json, rssi, battery_pct
                     FROM sensor_readings
                     WHERE adapter_id = ?1 AND device_key = ?2 AND sensor_type = ?3
                       AND ingested_at >= ?4 AND ingested_at < ?5
                     ORDER BY ingested_at DESC, sensor_type ASC
                     LIMIT ?6",
                )?;
                stmt.query_map(
                    rusqlite::params![adapter_id_str, device_key_str, st, start_millis, end_millis, limit],
                    row_to_reading,
                )?
                .collect::<Result<Vec<_>, _>>()?
            } else {
                let mut stmt = conn.prepare(
                    "SELECT adapter_id, device_key, ingested_at, sensor_type, values_json, rssi, battery_pct
                     FROM sensor_readings
                     WHERE adapter_id = ?1 AND device_key = ?2
                       AND ingested_at >= ?3 AND ingested_at < ?4
                     ORDER BY ingested_at DESC, sensor_type ASC
                     LIMIT ?5",
                )?;
                stmt.query_map(
                    rusqlite::params![adapter_id_str, device_key_str, start_millis, end_millis, limit],
                    row_to_reading,
                )?
                .collect::<Result<Vec<_>, _>>()?
            };
            Ok(rows)
        })
        .await?;

    Ok(rows)
}
```

- [ ] **Step 6: Implement `latest_reading`**

```rust
pub async fn latest_reading(
    db: &DbHandle,
    adapter_id: &AdapterId,
    device_key: &DeviceKey,
    sensor_type: Option<&SensorType>,
) -> Result<Option<ReadingRow>, TimeseriesError> {
    let adapter_id_str = adapter_id.as_str().to_string();
    let device_key_str = device_key.as_str().to_string();
    let sensor_type_str = sensor_type.map(|st| st.as_db_str().to_string());

    let row = db
        .with_conn(move |conn| {
            let row = if let Some(ref st) = sensor_type_str {
                conn.query_row(
                    "SELECT adapter_id, device_key, ingested_at, sensor_type, values_json, rssi, battery_pct
                     FROM sensor_readings
                     WHERE adapter_id = ?1 AND device_key = ?2 AND sensor_type = ?3
                     ORDER BY ingested_at DESC
                     LIMIT 1",
                    rusqlite::params![adapter_id_str, device_key_str, st],
                    row_to_reading,
                )
            } else {
                conn.query_row(
                    "SELECT adapter_id, device_key, ingested_at, sensor_type, values_json, rssi, battery_pct
                     FROM sensor_readings
                     WHERE adapter_id = ?1 AND device_key = ?2
                     ORDER BY ingested_at DESC, sensor_type ASC
                     LIMIT 1",
                    rusqlite::params![adapter_id_str, device_key_str],
                    row_to_reading,
                )
            };
            match row {
                Ok(r) => Ok(Some(r)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
        .await?;

    Ok(row)
}
```

- [ ] **Step 7: Implement `delete_before`**

```rust
pub async fn delete_before(
    db: &DbHandle,
    cutoff: SystemTime,
) -> Result<u64, TimeseriesError> {
    let cutoff_millis = system_time_to_millis(cutoff)?;

    let deleted = db
        .with_conn(move |conn| {
            let count = conn.execute(
                "DELETE FROM sensor_readings WHERE ingested_at < ?1",
                rusqlite::params![cutoff_millis],
            )?;
            Ok(count as u64)
        })
        .await?;

    Ok(deleted)
}
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test -p iotkit-core-timeseries`
Expected: All 16 tests pass.

- [ ] **Step 9: Run workspace tests**

Run: `cargo test --workspace`
Expected: All tests pass (including core/storage with updated signatures).

- [ ] **Step 10: Commit**

```bash
git add core/timeseries/src/lib.rs
git commit -m "feat(core/timeseries): implement query_readings, latest_reading, delete_before

- query_readings: optional sensor_type filter, time range validation, deterministic ordering
- latest_reading: optional sensor_type filter, tie-breaking by sensor_type ASC
- delete_before: returns deleted row count
- Full test coverage: 16 tests covering all functions and edge cases"
```

---

### Task 6: Gateway Integration — Migration Assembly + Event Loop Fan-out

**Files:**
- Modify: `iotkit-gateway/Cargo.toml`
- Modify: `iotkit-gateway/src/main.rs`

- [ ] **Step 1: Add `iotkit-core-timeseries` dependency**

In `iotkit-gateway/Cargo.toml`, add to `[dependencies]`:

```toml
iotkit-core-timeseries = { path = "../core/timeseries" }
```

- [ ] **Step 2: Update `init_db` call with migration assembly**

In `iotkit-gateway/src/main.rs`, change the `init_db` call:

```rust
// FROM:
    let db = match iotkit_core_storage::init_db(std::path::Path::new(&config.db_path)) {
        Ok(handle) => handle,
        Err(e) => {
            tracing::error!(error = %e, db_path = %config.db_path, "failed to initialize database");
            std::process::exit(1);
        }
    };

// TO:
    let mut all_migrations = Vec::from(iotkit_core_storage::MIGRATIONS);
    all_migrations.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    let db = match iotkit_core_storage::init_db(std::path::Path::new(&config.db_path), &all_migrations) {
        Ok(handle) => handle,
        Err(e) => {
            tracing::error!(error = %e, db_path = %config.db_path, "failed to initialize database");
            std::process::exit(1);
        }
    };
```

- [ ] **Step 3: Update `run()` function signature to accept `DbHandle`**

Change:

```rust
// FROM:
async fn run(config: config::GatewayConfig, _db: iotkit_core_storage::DbHandle) {

// TO:
async fn run(config: config::GatewayConfig, db: iotkit_core_storage::DbHandle) {
```

- [ ] **Step 4: Add imports for event loop fan-out**

Add to the top of `main.rs`:

```rust
use std::time::{Duration, Instant};
use iotkit_core_types::AdapterEvent;
```

- [ ] **Step 5: Replace the event loop with timeseries fan-out**

Replace the existing `// Unified fan-in loop` block with:

```rust
    // State for rate-limited error logging
    let mut ts_write_errors: u64 = 0;
    let mut last_ts_err_log = Instant::now();

    // Unified fan-in loop
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutdown signal received");
                break;
            }
            event = host.next_event() => {
                match event {
                    Some(AdapterHostEvent::Event(ev)) => {
                        tracing::debug!(
                            adapter = %ev.adapter_id,
                            event = ?ev.event,
                            "Adapter event"
                        );

                        // Extract timeseries fields BEFORE engine consumes the event
                        let ts_data = match &ev.event {
                            AdapterEvent::SensorData {
                                device_key, reading, rssi, battery_pct, ingested_at,
                            } => Some((
                                ev.adapter_id.clone(),
                                device_key.clone(),
                                *ingested_at,
                                reading.sensor_type.clone(),
                                reading.values.clone(),
                                *rssi,
                                *battery_pct,
                            )),
                            _ => None,
                        };

                        engine.apply(ev).await;

                        if let Some((adapter_id, device_key, ingested_at, sensor_type, values, rssi, battery_pct)) = ts_data {
                            if let Err(e) = iotkit_core_timeseries::insert_reading(
                                &db, &adapter_id, &device_key, ingested_at, &sensor_type, &values, rssi, battery_pct,
                            ).await {
                                ts_write_errors += 1;
                                // Log immediately on first failure, then rate-limit subsequent errors
                                if ts_write_errors == 1 || last_ts_err_log.elapsed() > Duration::from_secs(30) {
                                    tracing::error!(
                                        error = %e,
                                        suppressed = ts_write_errors.saturating_sub(1),
                                        "timeseries write failed"
                                    );
                                    ts_write_errors = 0;
                                    last_ts_err_log = Instant::now();
                                }
                            }
                        }
                    }
                    Some(AdapterHostEvent::AdapterClosed(id)) => {
                        tracing::warn!(
                            adapter = %id,
                            "Adapter channel closed unexpectedly"
                        );
                    }
                    None => {
                        tracing::info!("All adapter channels closed");
                        break;
                    }
                }
            }
        }
    }
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo check -p iotkit-gateway`
Expected: Compiles with no errors.

- [ ] **Step 7: Run full workspace tests**

Run: `cargo test --workspace`
Expected: All tests pass.

- [ ] **Step 8: Commit**

```bash
git add iotkit-gateway/Cargo.toml iotkit-gateway/src/main.rs
git commit -m "feat(gateway): integrate timeseries service into event loop

- Assemble migrations from core/storage + core/timeseries at startup
- Extract SensorData fields before engine.apply() for DB persistence
- Rate-limited error logging: first failure logged immediately, then every 30s
- Best-effort persistence: DB errors don't block event loop"
```

---

## Self-Review Checklist

### 1. Spec coverage

| Spec Section | Task |
|---|---|
| §1 Architecture (crate structure, dependency graph) | Task 1 (storage refactor), Task 3 (timeseries scaffold) |
| §2 core/storage Refactor (Migration pub, run_migrations pub, init_db signature, PRAGMAs, InvalidMigrationOrder) | Task 1 |
| §3 Table Schema (sensor_readings DDL) | Task 3 Step 3 |
| §4 SensorType DB Mapping (as_db_str, from_db_str) | Task 2 |
| §5 Rust API (TimeseriesError, ReadingRow, TimeRange, insert_reading, query_readings, latest_reading, delete_before, MIGRATIONS) | Tasks 3-5 |
| §6 Gateway Integration (migration assembly, event loop fan-out, rate-limited logging) | Task 6 |
| §8 Testing Strategy (all 16 timeseries tests, 3 storage tests, 2 types tests) | Tasks 1-5 |

### 2. Placeholder scan
No TBDs, TODOs, or placeholders found.

### 3. Type consistency
- `TimeseriesError` used consistently across Tasks 4-5
- `ReadingRow` fields match spec and SQL columns
- `query_readings` signature matches spec (with `sensor_type: Option<&SensorType>`)
- `latest_reading` signature matches spec (with `sensor_type: Option<&SensorType>`)
- `Migration` is `Clone, Copy` — `Vec::from()` and `extend_from_slice()` work in Task 6
