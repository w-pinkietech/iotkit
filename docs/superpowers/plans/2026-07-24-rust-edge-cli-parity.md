# Rust IoTKit Edge CLI Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the last eight Go `iotkit-edge` operator commands with Rust CLI adapters over the current semantic, output, and storage models.

**Architecture:** Clap dispatches typed application services in `application/cli_compat.rs`; backend-specific SQL remains in storage operations. Compatibility IDs and JSON are reversible views, output fan-out extends the provider-neutral binding/route model, and offline migration copies a closed Rust schema catalog under exclusive locks with transactional verification.

**Tech Stack:** Rust 2024, Clap, Tokio, SQLx SQLite/PostgreSQL, Serde, SHA-256, fs2

## Global Constraints

- The current Rust semantic and output tables are the only source of truth.
- CLI code contains no SQL and no provider-ID switch.
- Every mutation uses a typed application operation and atomic local-CLI audit.
- Go flags, JSON fields, stdout/stderr, validation order, and exit status remain compatible.
- Route additions are QoS 1, Adapter-validated, future-only, and support fan-out.
- Migration accepts only the exact Rust SQLx schema, never a Go-era database.
- Migration failure rolls back PostgreSQL and never publishes a report.
- Secrets and DSNs never appear in `Debug`, logs, errors, JSON output, or reports.

---

### Task 1: CLI oracle contract and typed read models

**Files:**
- Create: `edge/tests/cli_parity_contract.rs`
- Create: `edge/src/application/cli_compat.rs`
- Create: `edge/src/storage/cli_compat.rs`
- Modify: `edge/src/application/mod.rs`
- Modify: `edge/src/storage/mod.rs`

**Interfaces:**
- Produces: `CliQueries::new(Storage)`
- Produces: `CliQueries::raw_records(usize) -> Result<Vec<CliRawRecord>, StorageError>`
- Produces: `CliQueries::semantic_events(usize) -> Result<Vec<LegacySemanticEvent>, StorageError>`
- Produces: `legacy_mapping_id(&str) -> Result<String, CliCompatibilityError>`
- Produces: `rule_id_from_legacy_mapping(&str) -> Result<String, CliCompatibilityError>`

- [ ] **Step 1: Write failing alias and raw-query tests**

Add tests that construct a Rust UUID rule ID, require `sm-<32 lowercase hex>`
round trip, insert two raw records, and assert the Go ordering and JSON shape:

```rust
assert_eq!(
    legacy_mapping_id("550e8400-e29b-41d4-a716-446655440000").unwrap(),
    "sm-550e8400e29b41d4a716446655440000"
);
assert_eq!(records[0].edge_node_id, "edge-node-b");
assert_eq!(serde_json::to_value(&records[0]).unwrap()["record"]["values"][0], 2);
```

- [ ] **Step 2: Verify RED**

Run:

```bash
TMPDIR="$PWD/target/tmp" cargo test -p iotkit-edge --test cli_parity_contract raw_query
```

Expected: compile failure because `application::cli_compat` is absent.

- [ ] **Step 3: Implement typed reads**

Define serializable result types with exact Go field names. Add one storage
method that validates `1..=10_000` and queries:

```sql
SELECT edge_node_id,ledger_epoch,pub_seq,publication_id,record_json,received_at
FROM raw_records
ORDER BY received_at DESC,edge_node_id,ledger_epoch,pub_seq DESC
LIMIT ?
```

Use `$1` for PostgreSQL and decode `record_json` into `Box<RawValue>`.

- [ ] **Step 4: Verify GREEN**

Run the focused test and:

```bash
TMPDIR="$PWD/target/tmp" cargo test -p iotkit-edge --test storage_contract
```

### Task 2: Legacy mapping operations over semantic rules

**Files:**
- Modify: `edge/src/application/cli_compat.rs`
- Modify: `edge/src/storage/cli_compat.rs`
- Test: `edge/tests/cli_parity_contract.rs`

**Interfaces:**
- Produces: `LegacyMappingSpec { edge_node_id, series_key, meaning, trigger_mode, active_value }`
- Produces: `LegacyMappings::put(spec, now) -> Result<LegacyMapping, CliCompatibilityError>`
- Produces: `LegacyMappings::deactivate(edge_node_id, series_key, now) -> Result<LegacyMapping, CliCompatibilityError>`
- Produces: `LegacyMappings::list() -> Result<Vec<LegacyMapping>, CliCompatibilityError>`
- Consumes: `Semantics`, `RuleSpec`, and atomic storage audit operations

- [ ] **Step 1: Write failing mapping lifecycle tests**

Cover:

```rust
let first = mappings.put(active_sample_high, 10).await.unwrap();
let second = mappings.put(active_edge_low, 20).await.unwrap();
assert_eq!(first.mapping_id, second.mapping_id);
assert_eq!((first.revision, second.revision), (1, 2));
assert_eq!(mappings.list().await.unwrap().len(), 2);
let retired = mappings.deactivate("edge-node-01", "contact", 30).await.unwrap();
assert!(!retired.active);
```

Also assert the stored current rule is `CumulativeCounter`, has the expected
detector/trigger, captures future-only cursor boundaries, and writes the exact
three local CLI audit operations.

- [ ] **Step 2: Verify RED**

```bash
TMPDIR="$PWD/target/tmp" cargo test -p iotkit-edge --test cli_parity_contract mapping_
```

Expected: missing mapping operation APIs.

- [ ] **Step 3: Implement validation and conversion**

Use the closed conversion:

```rust
let detector = Detector {
    mode: if active_value == 1 {
        DetectorMode::BooleanHighActive
    } else {
        DetectorMode::BooleanLowActive
    },
    ..Detector::default()
};
let rule = RuleSpec {
    kind: SemanticKind::CumulativeCounter,
    detector,
    trigger: match trigger_mode {
        LegacyTriggerMode::ActiveSample => TriggerMode::OnNotification,
        LegacyTriggerMode::ActiveEdge => TriggerMode::OnTransition,
    },
};
```

Implement SQLite and PostgreSQL transactions that find the exact compatibility
rule, create or revise it through shared semantic helpers, and insert the audit
row before commit. Listing joins `semantic_rules`, `semantic_signals`, and
`semantic_rule_revisions`, decodes each `spec_json`, and derives active state.

- [ ] **Step 4: Verify GREEN**

Run mapping tests plus `semantic_contract` and `output_contract`.

### Task 3: Provider-neutral route fan-out schema

**Files:**
- Create: `edge/migrations/sqlite/0006_output_route_fanout.sql`
- Create: `edge/migrations/postgres/0006_output_route_fanout.sql`
- Modify: `edge/src/application/output_profiles.rs`
- Modify: `edge/src/storage/semantic_output/operations.rs`
- Modify: `edge/src/storage/semantic_output/sqlite.rs`
- Modify: `edge/src/storage/semantic_output/postgres.rs`
- Modify: `edge/src/storage/semantic_output/common.rs`
- Test: `edge/tests/output_contract.rs`

**Interfaces:**
- Produces: `OutputRouteDraft { rule_id, mode, config }`
- Produces: `OutputRoute { route_id, binding_id, rule_id, adapter_id, config_schema_version, config, start_after_observation_row_id, active, created_at }`
- Produces: `OutputProfiles::add_route(registration, draft, now) -> Result<OutputRoute, StorageError>`
- Produces: `OutputProfiles::route_statuses() -> Result<Vec<OutputRouteStatus>, StorageError>`

- [ ] **Step 1: Write failing fan-out tests**

Create one cumulative rule, add two generic routes with distinct exact topics,
accept and project one later record, and assert two outbox rows. Insert an
observation before the second route and assert the second route does not
backfill it. Reject blank, slash-bounded, wildcard, NUL, non-QoS-1, unknown
Adapter, and invalid config cases.

- [ ] **Step 2: Verify RED**

```bash
TMPDIR="$PWD/target/tmp" cargo test -p iotkit-edge --test output_contract route_fanout
```

Expected: the second route conflicts with the current unique binding constraint.

- [ ] **Step 3: Add append-only migrations**

SQLite rebuilds route/outbox/attempt tables with:

```sql
start_after_observation_row_id INTEGER NOT NULL DEFAULT 0
  CHECK(start_after_observation_row_id >= 0)
```

and without `UNIQUE(binding_id)`. PostgreSQL drops
`output_routes_binding_id_key` and adds the same `BIGINT` column. Preserve and
foreign-key-check all rows.

- [ ] **Step 4: Implement generic add-route transaction**

Validate through:

```rust
registration
    .adapter
    .validate_config(&draft.config, adapter_kind(rule.kind))?;
```

Find/create the live profile and rule binding without synthesizing a default
route, capture `MAX(observation_row_id)`, insert the route, and audit the
mutation. Update projection candidates to enforce the per-route boundary.

- [ ] **Step 5: Verify GREEN**

Run `output_contract`, `output_registry`, `output_puback`, and both migration
upgrade tests.

### Task 4: Legacy route application view

**Files:**
- Modify: `edge/src/application/cli_compat.rs`
- Modify: `edge/src/composition/output_adapters.rs`
- Test: `edge/tests/cli_parity_contract.rs`

**Interfaces:**
- Produces: `LegacyRoutes::new(OutputProfiles, &'static OutputAdapterRegistration)`
- Produces: `LegacyRoutes::add(mapping_id, topic, now) -> Result<LegacyRoute, CliCompatibilityError>`
- Produces: `LegacyRoutes::list() -> Result<Vec<LegacyRouteStatus>, CliCompatibilityError>`
- Consumes: generic MQTT JSON v1 registration, never its string ID in core

- [ ] **Step 1: Write failing route JSON tests**

Require `mr-<32 hex>` aliases, idempotent `(mapping, topic)` add, two-topic
fan-out, exact Go JSON fields, pending/published counts, oldest pending time,
and mapping/route alias round trips.

- [ ] **Step 2: Verify RED**

```bash
TMPDIR="$PWD/target/tmp" cargo test -p iotkit-edge --test cli_parity_contract route_
```

- [ ] **Step 3: Implement compatibility codec**

Build:

```json
{"schema_version":1,"topic":"factory/production-pulses"}
```

as `RawValue`, pass the registration into `OutputProfiles::add_route`, and
decode only that registration's validated non-secret config for list output.
Filter to compatibility rules and the supplied Adapter.

- [ ] **Step 4: Verify GREEN**

Run route and output tests.

### Task 5: Offline SQLite-to-PostgreSQL migration engine

**Files:**
- Create: `edge/src/storage/profile_migration.rs`
- Modify: `edge/src/storage/mod.rs`
- Modify: `edge/src/backup/sqlite.rs`
- Create: `edge/tests/profile_migration.rs`
- Create: `scripts/test-rust-edge-profile-migration.sh`

**Interfaces:**
- Produces: `migrate_sqlite_to_postgres(source: &Path, target: PostgresSecret) -> Result<ProfileMigrationReport, ProfileMigrationError>`
- Produces: `ProfileMigrationReport { source_profile, target_profile, edge_id, schema_version, table_counts, cursors, content_digest, completed }`
- Produces: closed `MIGRATION_TABLES: &[MigrationTable]`
- Consumes: normal SQLite file guard, PostgreSQL advisory guard, and protected snapshot helper

- [ ] **Step 1: Write failing local rejection tests**

Test nonexistent/non-regular source, live source, Go-era schema, insufficient
capacity, failed/ahead SQLx migration, report no-clobber, and redacted errors.

- [ ] **Step 2: Verify RED**

```bash
TMPDIR="$PWD/target/tmp" cargo test -p iotkit-edge --test profile_migration rejection_
```

- [ ] **Step 3: Implement snapshot and closed schema catalog**

Expose a backup-owned protected SQLite snapshot helper. Define every product
table, primary-key order, and typed column:

```rust
enum MigrationValueKind {
    Bool,
    I64,
    F64,
    Text,
    Bytes,
    Json,
}
struct MigrationColumn {
    name: &'static str,
    kind: MigrationValueKind,
    nullable: bool,
}
```

Compare actual SQLite tables/columns and successful migration versions exactly.

- [ ] **Step 4: Write failing real PostgreSQL copy tests**

Seed identity, activation, raw/cursor, auth/session/audit, semantic rule and
observation, two routes, pending and published outbox, backup/restore, and
capacity rows. Require exact table counts, cursors, digest, and destination
reads. Add non-empty/live target and a target-side trigger that forces a copy
failure before verification.

- [ ] **Step 5: Implement transactional copy and verification**

Decode SQLite values by the catalog, use `QueryBuilder<Postgres>` with typed
binds, reset generated sequences, and compute length-delimited canonical hashes
ordered by primary key. Compare the verification view before commit and again
after commit. The copy failure test installs a PostgreSQL trigger before the
operation and requires the normal transaction rollback path; product code has
no test-only hook.

- [ ] **Step 6: Verify GREEN on real PostgreSQL**

```bash
TMPDIR="$PWD/target/tmp" scripts/test-rust-edge-profile-migration.sh
```

Expected: migration, rejection, rollback, and no-report cases pass.

### Task 6: Clap commands and process parity

**Files:**
- Modify: `edge/src/cli/mod.rs`
- Modify: `edge/src/main.rs`
- Modify: `edge/tests/cli_contract.rs`
- Modify: `edge/tests/cli_parity_contract.rs`

**Interfaces:**
- Consumes: Tasks 1-5 application services and migration operation
- Produces: all eight public command variants and Go-compatible serialization

- [ ] **Step 1: Write failing process tests**

Use `CARGO_BIN_EXE_iotkit-edge` to cover accepted flags, every required flag,
validation before DB creation, pretty JSON/newline, empty stderr on success,
empty stdout on failure, and zero/non-zero exit for all eight commands.

- [ ] **Step 2: Verify RED**

```bash
TMPDIR="$PWD/target/tmp" cargo test -p iotkit-edge --test cli_parity_contract process_
```

Expected: Clap rejects the absent command variants.

- [ ] **Step 3: Add Clap argument types and thin dispatch**

Flatten `StorageArgs` for read/mutation commands. Keep migration's exact three
flags. Parse closed enums with Go spellings, validate before storage connect,
call one service method, and use the existing JSON writer.

- [ ] **Step 4: Verify GREEN**

Run `cli_contract`, `cli_parity_contract`, `semantic_contract`,
`output_contract`, and `profile_migration`.

### Task 7: Final verification and commit

**Files:**
- Review every changed CLI/application/storage/migration/test file.

- [ ] **Step 1: Run focused and real gates**

```bash
TMPDIR="$PWD/target/tmp" cargo test -p iotkit-edge
TMPDIR="$PWD/target/tmp" scripts/test-rust-edge-profile-migration.sh
TMPDIR="$PWD/target/tmp" cargo clippy -p iotkit-edge --all-targets -- -D warnings
```

- [ ] **Step 2: Run repository and review gates**

```bash
scripts/check-layers
scripts/check-source-layout
node scripts/battle-tested-review.mjs select --base dc4a98b
GOCACHE="$PWD/target/go-build" TMPDIR="$PWD/target/tmp" scripts/verify.sh
```

- [ ] **Step 3: Self-review**

Check no SQL under `cli/` or `application/`, no Adapter ID switch under storage,
no compatibility table, exact validation ordering, report mode/no-clobber,
transaction rollback, future-only route boundary, and secret redaction.

- [ ] **Step 4: Commit**

```bash
git add edge scripts docs/superpowers/plans/2026-07-24-rust-edge-cli-parity.md
git commit -m "feat(edge): port operator CLI to Rust"
```
