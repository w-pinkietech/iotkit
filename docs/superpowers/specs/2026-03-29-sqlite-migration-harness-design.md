# SQLite Migration Harness Design (#21)

## Goal

Provide the SQLite persistence foundation for the IoT gateway: a migration harness that manages schema versioning, and a `DbHandle` abstraction for runtime database access. This is infrastructure — actual table definitions and queries are added by downstream issues (#22 timeseries-service, #23 device-config-service).

## Architecture

A new workspace crate `core/storage` (`iotkit-core-storage`) owns:

1. **Connection initialization** — open SQLite, configure pragmas, run migrations.
2. **Migration harness** — ordered, forward-only, per-migration transactions with version tracking.
3. **DbHandle** — thread-safe connection wrapper with sync and async accessors.

```
iotkit-gateway (main.rs)
  │
  │  main(): config.db_path → init_db(&Path)
  │          → open + pragmas + migrate → DbHandle
  │
  ▼
core/storage/
  ├── migrations/          (SQL files, compiled into binary via include_str!)
  ├── src/lib.rs           (pub API: init_db, DbHandle, StorageError)
  ├── src/handle.rs        (DbHandle implementation)
  ├── src/migrate.rs       (migration harness)
  └── src/error.rs         (StorageError)
```

### Dependency Graph

```
core/types ← core/engine ← adapters ← iotkit-gateway
                                            ↑
core/storage ───────────────────────────────┘
```

`core/storage` has **no dependency on `core/types`**. It speaks only in SQL, version numbers, and raw connections. Domain types enter when #22/#23 build their data access layers on top of `DbHandle`.

### Scope Boundaries

**In scope:**
- `iotkit-core-storage` crate (Cargo.toml, lib.rs, handle.rs, migrate.rs, error.rs)
- Migration harness with `_schema_version` tracking table
- `DbHandle` with `with_conn` (async) and `with_conn_sync` (sync) accessors
- `init_db(&Path)` as sole public entry point
- `init_db_memory()` test helper behind `test-util` feature
- Gateway `main.rs` integration: call `init_db` before tokio runtime
- Unit tests for migration harness
- `001_init.sql` — empty baseline migration (establishes version 1, no application tables)

**Out of scope:**
- Sensor data tables (→ #22 timeseries-service)
- Device config tables (→ #23 device-config-service)
- Repository trait / query logic (→ #22/#23, concrete-first then extract)
- Data retention / pruning
- Connection pooling / multi-connection (v1 = single connection)
- Down migrations / rollback SQL

## PRAGMA Configuration

Set in `init_db()` before any migration runs:

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
```

| PRAGMA | Purpose |
|---|---|
| `journal_mode = WAL` | Crash-safe journaling; enables concurrent reads from external tools (e.g., `sqlite3` CLI). In-process access is serialized by the `Mutex` in `DbHandle`. |
| `synchronous = FULL` | Every commit is durable — critical for RPi power-loss |
| `foreign_keys = ON` | Enable FK constraints for #22/#23 schemas |
| `busy_timeout = 5000` | 5s retry on writer contention (safety valve for future multi-connection) |

## Migration Harness

### SQL File Convention

Migration SQL files live in `core/storage/migrations/` and are compiled into the binary via `include_str!()`. File naming: `NNNN_label.sql` (zero-padded, e.g., `0001_init.sql`). The file name is for human readability; the `version` field in the `Migration` struct is the source of truth.

### schema_version Table

Created by `run_migrations()` before checking any migration versions (bootstrap chicken-and-egg avoidance):

```sql
CREATE TABLE IF NOT EXISTS _schema_version (
    version    INTEGER NOT NULL,
    label      TEXT    NOT NULL,
    applied_at INTEGER NOT NULL,
    PRIMARY KEY (version)
);
```

- `version`: monotonically increasing integer starting at 1.
- `label`: human-readable migration name (e.g., `"init"`).
- `applied_at`: UTC unix timestamp (seconds since epoch). Query with `SELECT version, label, datetime(applied_at, 'unixepoch') FROM _schema_version;` for human-readable output.
- Underscore prefix `_schema_version` signals infrastructure table, not application data.

### Migration Execution

```
run_migrations(conn):
  1. CREATE TABLE IF NOT EXISTS _schema_version
  2. SELECT COALESCE(MAX(version), 0) → current_version
  3. If current_version > latest known migration → StorageError::SchemaVersionAhead (refuse to start)
  4. For each migration where version > current_version:
     a. BEGIN transaction
     b. execute_batch(migration.sql)
     c. INSERT INTO _schema_version (version, label, applied_at)
     d. COMMIT
     e. Log: "applying migration v{version}: {label}"
  5. If any step fails → transaction rolls back, return StorageError::MigrationFailed
```

Each migration runs in its **own transaction**. A failure leaves the DB at the last successfully applied version, never in a half-migrated state.

### Schema Version Ahead Guard

If the on-disk `_schema_version` has a version higher than the latest known migration in the binary, `init_db()` returns `StorageError::SchemaVersionAhead`. This catches the case where an operator rolls back the binary but not the database. The error message includes both versions so the operator knows what to do.

### Initial Migration (001_init.sql)

`0001_init.sql` is an empty baseline migration (comment-only). It establishes version 1 as the "known good starting point" so that future migrations can rely on `current_version >= 1` meaning "init has run." No application tables — those are added by #22/#23.

```sql
-- 0001_init.sql
-- Baseline migration for iotkit-core-storage.
-- Application tables are added by downstream issues (#22, #23).
```

### Migration SQL Rules

Migration SQL files must **not** contain transaction-control statements (`BEGIN`, `COMMIT`, `ROLLBACK`) or statements incompatible with transactions (`VACUUM`, `PRAGMA journal_mode`). The harness wraps each migration in its own transaction. Violations produce confusing SQLite errors. This is enforced by code review, not runtime validation.

### Migration Immutability

Once a migration has been shipped (merged to master), its SQL file is **immutable**. Editing an already-applied migration creates schema drift between databases initialized at different times. The migration harness does not store checksums (YAGNI for <10 migrations), but this rule is enforced by policy and code review. If a shipped migration needs correction, add a new migration that fixes the schema.

### MIGRATIONS Array Invariants

The `MIGRATIONS` const array must satisfy:
- **Unique versions** — no duplicate version numbers.
- **Strictly increasing** — versions must be in ascending order.
- **Contiguous** — no gaps (1, 2, 3... not 1, 3, 5).

These invariants are validated by a unit test (`test_migrations_array_invariants`) that runs on every build. The test iterates the array and asserts uniqueness, ordering, and contiguity.

### Adding Migrations (#22/#23 workflow)

To add a migration:
1. Create `core/storage/migrations/NNNN_label.sql`
2. Add one entry to the `MIGRATIONS` const array in `migrate.rs`
3. The compiler verifies the file exists (`include_str!` fails if not)

## Rust Types

### StorageError

```rust
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
    SchemaVersionAhead {
        on_disk: u32,
        latest_known: u32,
    },
}
```

Implements `Display`, `Error`, `From<rusqlite::Error>`, `From<std::io::Error>`.

### DbHandle

```rust
/// Thread-safe handle to a SQLite connection. Clone is cheap (Arc clone).
#[derive(Clone)]
pub struct DbHandle {
    conn: Arc<Mutex<Connection>>,  // std::sync::Mutex
}
```

**Methods:**

- **`with_conn<F, T>(F) -> Result<T, StorageError>`** (async) — runs closure on `tokio::task::spawn_blocking` with exclusive connection access. Primary API for #22/#23 runtime queries.
- **`with_conn_sync<F, T>(F) -> Result<T, StorageError>`** (sync) — direct mutex lock, no tokio. For startup/shutdown where async runtime may not be available.

**Design decisions:**
- Closure receives `&Connection` (not `&mut`): rusqlite query/execute methods take `&self`, Mutex provides exclusivity. **Known limitation:** `Connection::transaction()` requires `&mut self`. When #22/#23 need multi-statement transactions, the preferred resolution is to add a `with_conn_mut` variant that passes `&mut Connection`. As a runtime-checked fallback, `Connection::unchecked_transaction()` takes `&self` but defers nested-transaction detection to SQLite at runtime — callers must still ensure they do not nest transactions within a single closure (or use savepoints). This is deferred until actual transaction needs emerge.
- Panic on mutex poison / JoinError via `expect("descriptive message")`: mutex poison means a previous closure panicked, leaving the connection in an indeterminate state — this is not recoverable. JoinError means the blocking task panicked. Both indicate bugs, not operational errors. Callers cannot meaningfully retry.
- **Non-reentrant with handle-scoped guard:** Calling `with_conn` or `with_conn_sync` on the **same underlying connection** (including clones) from inside another accessor closure on the **same thread** will deadlock. The API mitigates this by design: closures receive `&Connection`, so helpers that need DB access accept `&Connection` directly. As a fail-fast safety net, both methods use a `thread_local!` set of active handle identities (keyed by `Arc::as_ptr` of the inner `Mutex<Connection>`). The guard check runs **immediately before `Mutex::lock()`** on the thread that will actually acquire the lock — for `with_conn_sync` this is the caller's thread, for `with_conn` this is inside the `spawn_blocking` closure. If the handle's identity is already in the set, the method panics with `"DbHandle re-entered — pass &Connection instead"`. The identity is added before lock acquisition and removed after the closure returns (in a drop guard). This correctly distinguishes: (a) same-handle same-thread reentry → panic (including clones, which share the same `Arc` identity), (b) cross-thread contention on the same handle → normal lock wait, (c) different `DbHandle` instances backed by different connections on the same thread → no conflict. **Limitation:** the guard only catches same-thread reentry. A closure that spawns another thread/task and synchronously waits for it to acquire the same handle (e.g., `block_on(handle.with_conn(...))` from inside a closure) can still deadlock. The doc comment warns against this pattern explicitly.
- No `close()` method: Arc drop triggers `sqlite3_close`. For explicit WAL checkpoint before exit, use `with_conn_sync`.
- `impl Debug for DbHandle`: manual implementation (Connection is not Debug).

### Migration (crate-internal)

```rust
struct Migration {
    version: u32,
    label: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration { version: 1, label: "init", sql: include_str!("../migrations/0001_init.sql") },
];
```

Not public. Downstream crates add migrations by modifying `core/storage` source (adding SQL file + array entry), not by calling an API.

### init_db

```rust
/// Open (or create) the database, configure pragmas, run migrations.
/// Sole public entry point for obtaining a DbHandle.
/// Synchronous — call before entering the async runtime.
pub fn init_db(db_path: &Path) -> Result<DbHandle, StorageError>
```

Steps:
1. Open connection (creates file if absent). If parent directory does not exist, return `StorageError::Io` — do NOT create parent directories (deployment misconfiguration should not be hidden).
2. Set PRAGMAs (WAL, synchronous, foreign_keys, busy_timeout).
3. Run migrations.
4. Return `DbHandle`.

### Test Helper

```rust
/// Open an in-memory database with all migrations applied.
#[cfg(any(test, feature = "test-util"))]
pub fn init_db_memory() -> Result<DbHandle, StorageError>
```

Available within the crate's own tests and to downstream crates via `features = ["test-util"]` in dev-dependencies.

**In-memory PRAGMA differences:** `journal_mode = WAL` is silently ignored for `:memory:` databases (SQLite falls back to in-memory journaling). `synchronous = FULL` and `foreign_keys = ON` still apply. This is acceptable because `init_db_memory()` tests migration logic and query behavior, not durability. File-backed tests (via tempfile) cover PRAGMA verification.

## Gateway Integration

### Startup Sequence

```rust
fn main() {
    // 1. tracing init
    // 2. config::load(&args) → GatewayConfig  (or exit 1)
    // 3. log effective config
    // 4. init_db(Path::new(&config.db_path))   (or exit 1)
    // 5. log "database initialized"
    // 6. Runtime::new()
    // 7. rt.block_on(run(config, db))
}

async fn run(config: GatewayConfig, db: DbHandle) {
    // Engine::new() — unchanged, pure in-memory
    // Start adapters — unchanged
    // Event loop — unchanged
    // Shutdown
}
```

`init_db()` is called in `main()` **before** `Runtime::new()`. It is sync and must complete before any async work begins. If it fails, the gateway exits with a clear error including `db_path`.

### Error Handling

Same pattern as config errors: `tracing::error!` with structured fields + `process::exit(1)`. The error log includes `db_path` so operators can diagnose path/permission issues.

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

### Engine Stays Pure

Engine does **not** gain a DbHandle. For #22/#23, services compose Engine + DbHandle:
- `ReadingLogger` (#22): hooks into the gateway fan-in path (receives `EngineEvent` before or after `engine.apply()`), writes to DbHandle. The Engine itself has no observer/subscription API — the gateway composition root owns the tap point.
- `DeviceConfigService` (#23): reads/writes device config via DbHandle

This keeps Engine testable without any database dependency. The exact subscription/tap mechanism is designed in #22's spec, not here.

### run() Signature Change

```rust
async fn run(config: GatewayConfig, db: DbHandle) { ... }
```

`db` is passed but not yet used by any service in #21. It will be consumed when #22/#23 wire their services.

## Dependencies

### New crate `iotkit-core-storage`

```toml
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

- `rusqlite` with `bundled`: compiles SQLite from C source, no system libsqlite3 dependency. Important for RPi where system version may be old.
- `tokio` for `spawn_blocking` in `with_conn`.
- `tracing` for migration progress logging.

### Gateway additions

```toml
[dependencies]
iotkit-core-storage = { path = "../core/storage" }
```

## Timestamp in _schema_version

`applied_at` stores a UTC unix timestamp as an integer (seconds since epoch), obtained via `SystemTime::now().duration_since(UNIX_EPOCH).as_secs()`. No datetime formatting dependency needed. While less human-readable than ISO 8601, it is unambiguous, trivially sortable, and can be formatted in `sqlite3` CLI via `datetime(applied_at, 'unixepoch')` when needed. This avoids adding `humantime` or `chrono` for a single call site.

## Testing Strategy

Unit tests in `core/storage`:

1. **Fresh DB migration** — `:memory:` DB gets fully migrated, `_schema_version` has correct entries
2. **Already-migrated is no-op** — tempfile DB, call `init_db` twice on same path, second call succeeds without re-applying migrations
3. **Schema version ahead rejected** — manually insert future version, `run_migrations` returns `SchemaVersionAhead`
4. **Migration failure rolls back** — simulate bad SQL, verify DB stays at previous version
5. **with_conn async works** — `#[tokio::test]`, simple query through `with_conn`
6. **with_conn_sync works** — simple query through `with_conn_sync`
7. **PRAGMA verification** — file-backed tempfile DB, verify `journal_mode = wal`, `synchronous = 2` (FULL), `foreign_keys = 1` after `init_db`
8. **with_conn async works** — `#[tokio::test]`, simple query through `with_conn`
9. **with_conn_sync works** — simple query through `with_conn_sync`
10. **MIGRATIONS array invariants** — versions are unique, strictly increasing, and contiguous starting at 1
11. **Same-handle same-thread reentry panics** — `with_conn_sync` nested inside `with_conn_sync` on the same handle triggers panic, not deadlock (`#[should_panic]`)
12. **Cross-thread sync contention waits** — two threads calling `with_conn_sync` on the same handle both succeed (one waits, both return correct results)
13. **Concurrent async contention succeeds** — `#[tokio::test]`, two concurrent `with_conn` calls on the same handle both succeed with serialized execution

No gateway-level integration test in #21 — storage is tested at the crate level. Gateway integration (config → init_db → adapters) is validated when #22/#23 add actual DB consumers.
