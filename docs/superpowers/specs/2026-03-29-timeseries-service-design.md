# Timeseries Service v1 Design Spec

**Issue:** #22 [roadmap 3] timeseries-service v1

**Goal:** Persist sensor readings from the engine event stream to SQLite so that data survives gateway restarts, and provide range-query and retention APIs for downstream consumers.

**Depends on:** #19 (timestamp/provenance), #21 (SQLite migration harness)

---

## 1. Architecture

### Crate structure

| Crate | Role | Dependencies |
|---|---|---|
| `core/storage` | Domain-agnostic SQLite infrastructure: DbHandle, migration runner, StorageError | rusqlite, tokio, tracing |
| `core/timeseries` | **NEW.** Timeseries INSERT/query/delete + migration SQL | core/types, core/storage |
| `iotkit-gateway` | Composition root. Assembles migrations, wires event loop | core/engine, core/storage, core/timeseries, adapters |

### Dependency graph

```
core/types  <- core/engine     <- adapters <- iotkit-gateway
core/types  <- core/timeseries <- iotkit-gateway
core/storage <- core/timeseries
core/storage <-                   iotkit-gateway
```

`core/engine` has **no** dependency on `core/storage` or `core/timeseries`. `core/storage` has **no** dependency on `core/types`.

### File layout

```
core/timeseries/
  Cargo.toml
  src/
    lib.rs              -- public API: insert_reading, query_readings, latest_reading, delete_before
    model.rs            -- ReadingRow, TimeRange
  migrations/
    0002_timeseries.sql -- table DDL

core/storage/
  src/
    lib.rs              -- init_db/init_db_memory signatures change (accept &[Migration])
    migrate.rs          -- Migration struct becomes pub, run_migrations becomes pub
    error.rs            -- adds InvalidReading, InvalidMigrationOrder variants
    handle.rs           -- unchanged
  migrations/
    0001_init.sql       -- unchanged
```

---

## 2. core/storage Refactor

### Migration type becomes public

```rust
// core/storage/src/migrate.rs

/// A single schema migration step.
pub struct Migration {
    pub version: u32,
    pub label: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration { version: 1, label: "init", sql: include_str!("../migrations/0001_init.sql") },
];
```

`MigrationEntry` trait and `TestMigration` remain `pub(crate)` for internal failure-scenario testing.

Re-exports in `lib.rs`:

```rust
pub use migrate::{Migration, MIGRATIONS, run_migrations};
```

### run_migrations becomes public with validation

```rust
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

### init_db / init_db_memory signature change

```rust
pub fn init_db(db_path: &Path, migrations: &[Migration]) -> Result<DbHandle, StorageError> {
    // parent dir check (unchanged)
    let conn = Connection::open(db_path)?;
    configure_pragmas(&conn)?;
    run_migrations(&conn, migrations)?;
    Ok(DbHandle::new(conn))
}

#[cfg(any(test, feature = "test-util"))]
pub fn init_db_memory(migrations: &[Migration]) -> Result<DbHandle, StorageError> {
    let conn = Connection::open_in_memory()?;
    configure_pragmas(&conn)?;
    run_migrations(&conn, migrations)?;
    Ok(DbHandle::new(conn))
}
```

The old `configure_and_migrate` function is removed. Its responsibilities split into:
- `configure_pragmas(conn)` (private) — sets all PRAGMAs
- `run_migrations(conn, migrations)` (public) — validates and applies migrations

### PRAGMA set (updated)

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA cache_size = -8000;   -- NEW: 8MB page cache
```

`cache_size = -8000` is a new addition in #22. Keeps hot B-tree pages resident for WITHOUT ROWID range scans.

### New StorageError variants

```rust
pub enum StorageError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    MigrationFailed { version: u32, source: Box<StorageError> },
    SchemaVersionAhead { on_disk: u32, latest_known: u32 },
    InvalidReading(String),             // NEW
    InvalidMigrationOrder { first: u32, second: u32 },  // NEW
}
```

Display implementations:
- `InvalidReading(msg)` → `"invalid reading: {msg}"`
- `InvalidMigrationOrder { first, second }` → `"migration versions not strictly ascending: v{first} >= v{second}"`

Both return `None` from `source()`.

---

## 3. Table Schema

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

### Column details

| Column | Type | Source | Notes |
|---|---|---|---|
| `adapter_id` | TEXT | `EngineEvent.adapter_id.as_str()` | Adapter identity |
| `device_key` | TEXT | `AdapterEvent::SensorData.device_key.as_str()` | Device identity within adapter |
| `ingested_at` | INTEGER | `SystemTime` → unix milliseconds | `duration_since(UNIX_EPOCH).as_millis() as i64` |
| `sensor_type` | TEXT | `SensorType::as_db_str()` | e.g. `"temperature"`, `"acceleration"`, `"unknown:custom"` |
| `values_json` | TEXT | `&[f64]` → JSON array | e.g. `"[25.3]"` or `"[0.1,-0.3,9.8]"` |
| `rssi` | INTEGER | `Option<i16>` | NULL if not available |
| `battery_pct` | INTEGER | `Option<u8>` | NULL if not available |

### Design decisions

- **WITHOUT ROWID**: Composite PK is the B-tree key directly. Range scans on `(adapter_id, device_key, time_range)` are single B-tree seeks with no rowid indirection.
- **sensor_type in PK**: Prevents collision when one device produces multiple sensor types in the same millisecond (e.g., BravePI burst frames).
- **sensor_type as TEXT**: `SensorType::Unknown(String)` variant cannot round-trip through INTEGER. TEXT stores the variant name directly.
- **values_json as JSON text**: Human-readable via `sqlite3` CLI on headless RPi. Write throughput difference vs BLOB is negligible (bottleneck is WAL fsync, not serialization).
- **No labels per-row**: Labels are a property of `SensorType`, not per-reading. Stored once in application code (or future #23 device-config-service).
- **No secondary indexes**: PK prefix covers the primary query pattern. Add indexes when profiling shows need.
- **INSERT policy**: Plain INSERT. UNIQUE violation surfaces as `StorageError::Sqlite`. At 1Hz polling, same-millisecond same-sensor-type collisions indicate an adapter bug and should be visible.

---

## 4. SensorType DB Mapping (core/types change)

Add to `SensorType` in `core/types/src/lib.rs`:

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

---

## 5. Rust API (core/timeseries)

### Public types

```rust
/// A single reading row from the sensor_readings table.
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
pub struct TimeRange {
    pub start: SystemTime,  // inclusive
    pub end: SystemTime,    // exclusive
}
```

### insert_reading

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
) -> Result<(), StorageError>
```

Behavior:
1. Validate `values`: if any element is NaN or Infinity, return `StorageError::InvalidReading("NaN/Inf in values at index {i}")`.
2. Convert `ingested_at` to unix millis (`i64`).
3. Serialize `values` to JSON array string.
4. `db.with_conn(move |conn| { conn.execute("INSERT INTO sensor_readings ...") })`.

### query_readings

```rust
pub async fn query_readings(
    db: &DbHandle,
    adapter_id: &AdapterId,
    device_key: &DeviceKey,
    range: TimeRange,
    limit: u32,
) -> Result<Vec<ReadingRow>, StorageError>
```

SQL: `SELECT ... WHERE adapter_id = ? AND device_key = ? AND ingested_at >= ? AND ingested_at < ? ORDER BY ingested_at DESC LIMIT ?`

`limit` is mandatory to prevent OOM on RPi for large time ranges.

### latest_reading

```rust
pub async fn latest_reading(
    db: &DbHandle,
    adapter_id: &AdapterId,
    device_key: &DeviceKey,
) -> Result<Option<ReadingRow>, StorageError>
```

SQL: `SELECT ... WHERE adapter_id = ? AND device_key = ? ORDER BY ingested_at DESC LIMIT 1`

### delete_before

```rust
pub async fn delete_before(
    db: &DbHandle,
    cutoff: SystemTime,
) -> Result<u64, StorageError>
```

SQL: `DELETE FROM sensor_readings WHERE ingested_at < ?`

Returns the number of rows deleted. The actual retention scheduling (timer, cron) is **out of scope** for #22. This API is a building block.

### Migrations constant

```rust
use iotkit_core_storage::Migration;

pub const MIGRATIONS: &[Migration] = &[
    Migration { version: 2, label: "timeseries", sql: include_str!("../migrations/0002_timeseries.sql") },
];
```

---

## 6. Gateway Integration

### Migration assembly at startup

```rust
// main.rs
let mut all_migrations = Vec::from(iotkit_core_storage::MIGRATIONS);
all_migrations.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
let db = match iotkit_core_storage::init_db(Path::new(&config.db_path), &all_migrations) {
    Ok(handle) => handle,
    Err(e) => {
        tracing::error!(error = %e, "failed to initialize database");
        std::process::exit(1);
    }
};
```

### Event loop fan-out (Option B)

```rust
// State for rate-limited error logging
let mut ts_write_errors: u64 = 0;
let mut last_ts_err_log = Instant::now();

loop {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => break,
        event = host.next_event() => {
            match event {
                Some(AdapterHostEvent::Event(ev)) => {
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
                            if last_ts_err_log.elapsed() > Duration::from_secs(30) {
                                tracing::error!(
                                    error = %e,
                                    suppressed = ts_write_errors,
                                    "timeseries write failed"
                                );
                                ts_write_errors = 0;
                                last_ts_err_log = Instant::now();
                            }
                        }
                    }
                }
                Some(AdapterHostEvent::AdapterClosed(id)) => {
                    tracing::warn!(adapter = %id, "Adapter channel closed");
                }
                None => break,
            }
        }
    }
}
```

### Design rationale for Option B (sequential fan-out)

- **No background task**: gateway currently has zero async background tasks beyond the event loop. Adding one introduces shutdown coordination complexity.
- **No backpressure concern**: at 10 sensors x 1Hz, `DbHandle.with_conn()` (spawn_blocking) takes 1-5ms per INSERT. 100ms between events provides ample headroom.
- **Migration to Option C** (channel-decoupled background writer) is a one-file refactor when throughput demands it. No public API changes needed.

---

## 7. Out of Scope

- **Retention policy scheduling** — `delete_before()` API is provided; timer/cron is a future concern
- **Device metadata / labels** — owned by #23 device-config-service
- **Aggregation / downsampling** — future, based on real Pi measurements
- **Database-per-period partitioning** — future optimization for SD card longevity
- **Background writer / channel decoupling** — Option C, migrate when throughput demands
- **Secondary indexes** — add when profiling shows need
- **Repository trait abstraction** — deferred to #23 (concrete-first, extract later)

---

## 8. Testing Strategy

### core/timeseries unit tests

| Test | What it verifies |
|---|---|
| `insert_and_query_single` | Insert one reading, query it back, verify all fields |
| `insert_multiple_sensor_types_same_timestamp` | Same device, same ms, different sensor_type → no collision |
| `query_time_range` | Multiple readings, query subset by time range |
| `query_respects_limit` | Insert N readings, query with limit < N, verify count |
| `query_returns_newest_first` | Verify ORDER BY ingested_at DESC |
| `latest_reading_returns_most_recent` | Insert several, latest_reading returns last |
| `latest_reading_empty` | No data → returns None |
| `delete_before_removes_old` | Insert old + new, delete_before cutoff, verify only new remain |
| `delete_before_returns_count` | Verify returned u64 matches deleted rows |
| `reject_nan_in_values` | Insert with NaN → InvalidReading error |
| `reject_infinity_in_values` | Insert with Inf → InvalidReading error |
| `values_json_round_trip` | Insert [0.1, -0.3, 9.8], query back, verify exact equality |

### core/storage migration tests (additions)

| Test | What it verifies |
|---|---|
| `migration_order_validation` | Pass out-of-order migrations → InvalidMigrationOrder error |
| `migration_duplicate_version` | Pass duplicate versions → InvalidMigrationOrder error |
| `external_migrations_applied` | Pass combined [v1, v2] list, verify both tables exist |

### core/types tests (additions)

| Test | What it verifies |
|---|---|
| `sensor_type_db_str_round_trip` | All known variants survive as_db_str → from_db_str |
| `sensor_type_unknown_round_trip` | Unknown("custom") → "custom" → Unknown("custom") |

### Gateway integration (manual / CI)

- `cargo test --workspace` passes
- Start gateway with real config → verify sensor_readings table is created
- Observe tracing logs for timeseries write success/failure
