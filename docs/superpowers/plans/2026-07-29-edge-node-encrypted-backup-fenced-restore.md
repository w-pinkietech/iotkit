# Edge Node Encrypted Backup and Fenced Restore Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver slice 1 of Issue #92: an optional encrypted,
custody-complete Edge Node database backup with offline deployment-credential
sanitation, a capability-checked mounted destination, and restore into an
unpublished, durably fenced candidate that the normal Edge Node runtime cannot
start.

**Architecture:** Add `iotkit-core-recovery` as the owner of the Edge Node backup artifact, complete migration set, backup/restore state, filesystem capability checks, and offline candidate fencing. `iotkit-edge-nodectl` remains a thin local-root adapter for owner-only configuration and passphrase files. The Edge Node composition root reads the durable recovery state before spawning any normal task; slice 1 exits safely for a fenced candidate and does not yet implement the restricted MQTT recovery runtime.

**Tech Stack:** Rust 1.95.0, rusqlite online backup API, SQLite WAL/FULL source storage, Argon2id, XChaCha20-Poly1305, SHA-256, serde JSON, clap, Linux mountinfo/systemd, existing `iotkit-core-ops` typed dispatcher.

## Global Constraints

- This plan implements only delivery slice 1 from `docs/superpowers/specs/2026-07-29-edge-node-computer-replacement-design.md`; Edge recovery cases, Broker fencing, C/B/E/W network reconciliation, new-epoch activation, and Console actions remain disabled.
- Backup is optional and default-off. A missing configuration must not change normal collection, publication, retention, API, or process readiness.
- The legacy plaintext `snapshot` command remains inventory-only and must never be called or used as fallback by any `backup` command.
- The artifact suffix is `.iotkit-node-backup`; its magic and `artifact_kind` must be distinct from the IoTKit Edge server backup.
- Passphrases contain 12 through 1024 Unicode scalar values and are read only from an owner-only regular file. They never appear in argv, environment, logs, errors, audit, fixtures, JSON output, or `Debug`.
- The container uses Argon2id and XChaCha20-Poly1305. The exact unencrypted header bytes, including magic, artifact kind, format version, salt, bounded KDF parameters, nonce prefix, and chunk size, are authenticated as associated data.
- Default KDF parameters are time cost 3, memory 65,536 KiB, and parallelism 4. Accepted input bounds are time `1..=10`, memory `16,384..=262,144` KiB, parallelism `1..=16`, and chunk size `4,096..=4,194,304` bytes.
- Plaintext SQLite staging must be an owner-only directory on `tmpfs`; the configured backup destination must not contain plaintext.
- Backup creation holds a nonblocking exclusive `flock` on an owner-only
  staging lock for its entire run. Restore holds an equivalent
  candidate-target lock. A competing invocation fails with `operation_busy`
  before snapshot/decryption work.
- Capacity checks use checked `u64` arithmetic and the same conservative rule
  everywhere: `required(bytes) = bytes + max(64 MiB, ceil(bytes / 20))`.
  Overflow is `capacity_overflow`; insufficient staging, destination, or
  candidate-filesystem space is `storage_full`. Backup rechecks destination
  capacity from the completed snapshot length before encryption, and restore
  checks both plaintext staging and the candidate filesystem from the
  authenticated manifest length before writing either.
- The destination must be an absolute path on a distinct mounted filesystem whose recorded source/type identity and create-new, owner-mode, file-sync, no-replace publication, parent-sync, and read-back checks pass. Failure never falls back to another path.
- On Linux, destination and candidate publication hold an
  `O_DIRECTORY|O_NOFOLLOW` parent descriptor and perform create/read/rename/
  sync/retention relative to that descriptor. The implementation must not
  resolve the configured path again after verification; path replacement or
  unmount after verification therefore cannot redirect bytes into a same-named
  local directory.
- Restore never overwrites or opens a live target for mutation. The recovery fence, authority rotation, SQLite self-containment, integrity checks, file sync, and close happen on an unpublished same-filesystem temporary candidate before no-replace publication.
- A restored candidate keeps the backed-up `edge_node_id` and old ledger epoch. Slice 1 must not renew the ledger epoch or enable the normal runtime.
- Slice 1 creates online snapshots only: `snapshot_mode` is `online` and
  `shutdown_seal_id` is always absent. The typed shutdown seal and any
  `no_loss_proven` decision belong to slice 3.
- All durable mutations use an `iotkit-core-ops` `OpDescriptor` and dispatcher. CLI and filesystem orchestration do not write recovery tables directly.
- Source tests follow repository placement rules: product `src/` contains no test bodies/helpers; private unit tests live under `tests/unit/**/*_tests.rs`; reusable fakes/fixtures live under `tests/support/`.
- The production mount/mode/descriptor implementation is Linux-only behind
  narrow platform ports. Parsing and fault tests remain host-independent;
  real backup/configure/restore calls on non-Linux return the closed
  `platform_unsupported` reason rather than weakening checks. The complete
  workspace must still compile and test on Windows.
- Do not bump the workspace product version, create a tag, publish a release, or claim the complete computer-replacement journey in this slice.

## Execution Prerequisite

Before Task 1, create a child issue under #92 titled:

> Edge Nodeの暗号化backupとfenced restoreを実装する

Its outcome is exactly this plan's goal. Its exclusions are Broker credential
fencing, remote recovery cases/permits, C/B/E/W reconciliation, new-epoch
activation, no-backup initialization, Console work, and closing parent #92.
After GitHub returns the child issue number, update `master`, create
`agent/issue-N-edge-node-backup-restore` and
`.worktrees/issue-N-edge-node-backup-restore` using that returned number, and
execute Tasks 1–10 only there. The draft PR closes the child issue, references
#92 without closing it, and keeps the recovery feature default-off.

## File and Responsibility Map

### New recovery crate

- `edge-node/core/recovery/Cargo.toml` — package dependencies and test-only dependencies.
- `edge-node/core/recovery/migrations/0023_edge_node_recovery.sql` — durable backup-attempt state machine and singleton candidate-fence state.
- `edge-node/core/recovery/src/lib.rs` — public exports, `MIGRATIONS`, and `all_edge_node_migrations()`.
- `edge-node/core/recovery/src/model.rs` — closed IDs, manifest/config/handoff/status/startup types, validation, and secret-safe errors.
- `edge-node/core/recovery/src/state.rs` — read models and typed operation descriptors for backup outcomes and candidate installation.
- `edge-node/core/recovery/src/snapshot.rs` — online SQLite snapshot, canonical schema/boundary validation, manifest derivation, and database digest.
- `edge-node/core/recovery/src/container.rs` — Node-specific authenticated encrypted container encode/decode.
- `edge-node/core/recovery/src/destination.rs` — Linux mountinfo parsing, mount identity/capability verification, no-replace publication, directory sync, read-back, and safe retention.
- `edge-node/core/recovery/src/config.rs` — owner-only schema-1 backup configuration parsing/validation and atomic creation.
- `edge-node/core/recovery/src/backup.rs` — create/inspect/status orchestration and attempt reconciliation.
- `edge-node/core/recovery/src/restore.rs` — handoff validation, decrypt/verify, unpublished fence installation, SQLite checkpoint/close, and candidate publication.
- `edge-node/core/recovery/contracts/node-backup-header-v1.schema.json` — exact outer authenticated-header JSON schema.
- `edge-node/core/recovery/contracts/node-backup-manifest-v1.schema.json` — exact encrypted manifest JSON schema.
- `edge-node/core/recovery/contracts/recovery-handoff-v1.schema.json` — exact protected handoff JSON schema.
- `edge-node/core/recovery/contracts/restore-receipt-v1.schema.json` — exact nonsecret receipt JSON schema.
- `edge-node/core/recovery/tests/fixtures/` — checked-in secret-free v1 JSON and deterministic binary golden artifacts shared by conformance tests.
- `edge-node/core/recovery/tests/support/mod.rs` — temporary complete Edge Node databases, deterministic IDs/clocks, mountinfo fixtures, and filesystem fault ports.
- `edge-node/core/recovery/tests/unit/*_tests.rs` — private module tests included from matching `src/*.rs`.
- `edge-node/core/recovery/tests/backup_contract.rs` — public artifact create/inspect/restore contract.

### Existing Rust packages

- `Cargo.toml` and `Cargo.lock` — add `edge-node/core/recovery`.
- `scripts/check-layers` — classify `iotkit-core-recovery` as data plane.
- `edge-node/core/publish/src/activation.rs` — expose allocation high-water read without changing allocation.
- `edge-node/core/publish/tests/unit/activation_tests.rs` — prove high-water survives pruning.
- `edge-node/apps/nodectl/Cargo.toml` — depend on recovery and zeroizing secret support.
- `edge-node/apps/nodectl/src/main.rs` — register and early-route `backup` commands without opening/migrating the live DB through the generic command path.
- `edge-node/apps/nodectl/src/cmd/backup.rs` — clap arguments, owner-only file reads, JSON output, and recovery-library calls.
- `edge-node/apps/nodectl/tests/backup_cli.rs` — subprocess-level secret, exit-code, no-fallback, and no-clobber coverage.
- `edge-node/apps/node/Cargo.toml` — depend on recovery.
- `edge-node/apps/node/src/main.rs` — use the centralized full migration set and enforce the startup recovery gate before runtime/task construction.
- `edge-node/apps/node/tests/recovery_startup.rs` — prove a fenced candidate starts no normal service.

### Deployment and current authority

- `deploy/systemd/iotkit-edge-node-backup.service` — hardened oneshot using the owner-only config and tmpfs runtime directory.
- `deploy/systemd/iotkit-edge-node-backup.timer` — persistent daily timer with randomized delay.
- `scripts/tests/edge-node-backup-systemd.test.mjs` — verify the unit/timer contract and generated mount drop-in.
- `docs/okf/en/architecture/system-overview.md` and `docs/okf/ja/architecture/system-overview.md` — classify the recovery crate and startup fence.
- `docs/okf/en/contracts/ingest-v1.md` and `docs/okf/ja/contracts/ingest-v1.md` — replace only the deferred backup/candidate statement; keep remote replacement and new-epoch activation deferred.
- `docs/okf/en/contracts/edge-node-recovery-v1.md` and
  `docs/okf/ja/contracts/edge-node-recovery-v1.md` — exact v1 header,
  manifest, handoff, receipt, fence, and slice boundary authority.
- `docs/README.md` — index the paired recovery contract and machine-readable
  artifacts.
- `docs/okf/en/operations/installation-and-recovery.md` and `docs/okf/ja/operations/installation-and-recovery.md` — optional configuration, manual/scheduled backup, inspect/status, candidate restore, and explicit “not active yet” runbook.

## Shared Interfaces

Task implementations use these names and shapes consistently:

```rust
pub const NODE_BACKUP_SUFFIX: &str = ".iotkit-node-backup";
pub const NODE_BACKUP_FORMAT_VERSION: u32 = 1;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryHandoff {
    pub schema_version: u32,
    pub recovery_id: String,
    pub edge_id: String,
    pub edge_node_id: String,
    pub old_ledger_epoch: String,
    pub expected_backup_id: Option<String>,
    pub proposed_new_epoch: String,
    pub credential_generation: i64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupConfig {
    pub schema_version: u32,
    pub database: PathBuf,
    pub destination: PathBuf,
    pub staging_directory: PathBuf,
    pub passphrase_file: PathBuf,
    pub expected_mount: MountIdentity,
    pub freshness_seconds: u64,
    pub retention_count: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountIdentity {
    pub mount_point: PathBuf,
    pub source: String,
    pub filesystem_type: String,
    pub filesystem_id: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeBackupManifest {
    pub artifact_kind: String,
    pub format_version: u32,
    pub backup_id: String,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub created_at_ms: i64,
    pub accepted_cursor: i64,          // C
    pub allocation_high_water: i64,    // B
    pub snapshot_mode: SnapshotMode,   // Online in slice 1
    pub shutdown_seal_id: Option<String>,
    pub schema_version: u32,
    pub database_length: u64,
    pub database_sha256: String,
    pub counts: BackupCounts,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotMode {
    Online,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupCounts {
    pub devices: u64,
    pub series: u64,
    pub readings: u64,
    pub publication_rows: u64,
    pub ingest_dedup_rows: u64,
    pub staged_readings: u64,
    pub quarantine_rows: u64,
    pub device_principals: u64,
    pub device_credentials: u64,
    pub activation_rows: u64,
    pub ledger_events: u64,
    pub audit_events: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub enum RecoveryStartupMode {
    Normal,
    FencedCandidate {
        recovery_id: String,
        candidate_instance_id: String,
        backup_id: Option<String>,
        edge_id: String,
        old_ledger_epoch: String,
        proposed_new_epoch: String,
        credential_generation: i64,
    },
}

#[derive(Clone, PartialEq, Eq)]
pub enum BackupReadiness {
    NotConfigured,
    Healthy { artifact: BackupStatusArtifact },
    Stale { artifact: BackupStatusArtifact },
    Failed {
        reason_code: String,
        observed_at_ms: i64,
        last_verified: Option<BackupStatusArtifact>,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupStatusArtifact {
    pub backup_id: String,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub created_at_ms: i64,
    pub artifact_length: u64,
    pub accepted_cursor: i64,
    pub allocation_high_water: i64,
}

pub struct RestoreRequest {
    pub input: PathBuf,
    pub live_database: PathBuf,
    pub candidate_database: PathBuf,
    pub staging_directory: PathBuf,
    pub handoff: RecoveryHandoff,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreReceipt {
    pub schema_version: u32,
    pub status: RestoreStatus,
    pub recovery_id: String,
    pub candidate_instance_id: String,
    pub backup_id: String,
    pub edge_id: String,
    pub edge_node_id: String,
    pub old_ledger_epoch: String,
    pub proposed_new_epoch: String,
    pub credential_generation: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreStatus {
    DurablyFencedCandidate,
}

pub struct BackupPassphrase(zeroize::Zeroizing<String>);

pub fn all_edge_node_migrations() -> Vec<iotkit_core_storage::Migration>;
pub fn recovery_descriptors() -> &'static [iotkit_core_ops::OpDescriptor];
pub fn startup_mode(conn: &rusqlite::Connection) -> Result<RecoveryStartupMode, RecoveryError>;
pub fn probe_startup_path(path: &Path) -> Result<RecoveryStartupMode, RecoveryError>;
pub fn create_backup(config: &BackupConfig, passphrase: &BackupPassphrase, now_ms: i64)
    -> Result<NodeBackupManifest, RecoveryError>;
pub fn inspect_backup(input: &Path, passphrase: &BackupPassphrase)
    -> Result<NodeBackupManifest, RecoveryError>;
pub fn backup_status(config_path: &Path, now_ms: i64)
    -> Result<BackupReadiness, RecoveryError>;
pub fn restore_candidate(request: &RestoreRequest, passphrase: &BackupPassphrase)
    -> Result<RestoreReceipt, RecoveryError>;

fn encrypt_container(
    snapshot: &Path,
    manifest: &NodeBackupManifest,
    passphrase: &BackupPassphrase,
    output: &Path,
) -> Result<(), RecoveryError>;
fn authenticate_container(
    input: &Path,
    passphrase: &BackupPassphrase,
) -> Result<NodeBackupManifest, RecoveryError>;
fn decrypt_container_to_new_file(
    input: &Path,
    passphrase: &BackupPassphrase,
    output: &Path,
) -> Result<NodeBackupManifest, RecoveryError>;
```

`BackupConfig`, `MountIdentity`, `RecoveryHandoff`, `RecoveryStartupMode`,
`BackupReadiness`, `BackupStatusArtifact`, `NodeBackupManifest`, and
`RestoreReceipt` implement custom redacted `Debug`. They expose only type/
state/format labels and safe aggregate counts; paths, mount point/source/
filesystem ID, database digest, all node/Edge/backup/recovery/candidate/epoch
IDs, credential generation, C/B, and handoff content are omitted. Normal JSON
output is projected into dedicated response types and never serializes the
manifest wholesale. Sentinel tests exercise every custom `Debug`
implementation.

`RecoveryError` is a closed, secret-safe enum. It uses reason codes such as
`config_invalid`, `mount_missing`, `mount_identity_mismatch`,
`filesystem_capability_missing`, `destination_exists`, `authentication_failed`,
`manifest_invalid`, `snapshot_invalid`, `handoff_mismatch`,
`candidate_exists`, `candidate_fence_invalid`, `capacity_overflow`, and
`storage_full`, `mount_identity_unavailable`, and `platform_unsupported`; it
also includes `operation_busy`, `candidate_publication_uncertain`, and
`candidate_conflict`, plus `artifact_publication_uncertain`. It never embeds
passphrases, mount sources, payload bytes, SQL text containing sensitive
values, or full paths in its `Display` output.

---

### Task 1: Establish the Recovery Crate, Complete Migration Set, and Durable State

**Files:**
- Create: `edge-node/core/recovery/Cargo.toml`
- Create: `edge-node/core/recovery/migrations/0023_edge_node_recovery.sql`
- Create: `edge-node/core/recovery/src/lib.rs`
- Create: `edge-node/core/recovery/src/model.rs`
- Create: `edge-node/core/recovery/src/state.rs`
- Create: `edge-node/core/recovery/tests/support/mod.rs`
- Create: `edge-node/core/recovery/tests/unit/model_tests.rs`
- Create: `edge-node/core/recovery/tests/unit/state_tests.rs`
- Modify: `Cargo.toml`
- Modify: `scripts/check-layers`
- Modify: `edge-node/core/publish/src/activation.rs`
- Modify: `edge-node/core/publish/tests/unit/activation_tests.rs`
- Modify: `docs/okf/en/architecture/system-overview.md`
- Modify: `docs/okf/ja/architecture/system-overview.md`

**Interfaces:**
- Consumes: Existing migration arrays from storage, ledger, timeseries, registry, publish, and ops; `iotkit-core-ops` dispatcher/descriptor types.
- Produces: `all_edge_node_migrations()`, model types above,
  read-only `probe_startup_path()`, `startup_mode()`, and public
  `publication_allocation_high_water()`. Tasks 2, 5, and 6 populate the final
  recovery descriptor catalog as their mutations are introduced.

- [ ] **Step 1: Write failing migration and high-water tests**

Add tests that require a migration-23 singleton candidate table, a
single-direction backup-attempt state machine, strict state checks, and a
high-water value that remains
5 after rows 1 through 5 are pruned:

```rust
#[test]
fn allocation_high_water_survives_pruning() {
    let db = activated_database();
    for reading in 1..=5 {
        enqueue_measurement(&db, reading);
    }
    prune_acked_outbox(&db, &epoch(&db), 5).unwrap();
    assert_eq!(publication_allocation_high_water(&db).unwrap(), 5);
}

#[test]
fn recovery_migration_defaults_to_normal_without_a_candidate_row() {
    let db = complete_database();
    assert_eq!(startup_mode(&db).unwrap(), RecoveryStartupMode::Normal);
    assert_table_columns(
        &db,
        "edge_node_recovery_candidate",
        &["singleton", "state", "recovery_id", "candidate_instance_id",
          "backup_id", "edge_id", "edge_node_id", "old_ledger_epoch",
          "proposed_new_epoch", "credential_generation",
          "handoff_schema_version", "installed_at_ms"],
    );
}
```

Add read-only startup probes for a missing/new/pre-v23 database, a current
normal database, a valid fenced candidate, and malformed/partial candidate
state. Missing/pre-v23 is normal-eligible; any present but invalid recovery
schema/row fails closed without migration or repair.

- [ ] **Step 2: Run the focused tests and confirm RED**

Run:

```bash
cargo test -p iotkit-core-publish allocation_high_water_survives_pruning
cargo test -p iotkit-core-recovery
```

Expected: the publish test fails because the read function is absent; Cargo
fails to resolve `iotkit-core-recovery`.

- [ ] **Step 3: Add the package and closed model/state schema**

Use migration version 23 and these state constraints:

```sql
CREATE TABLE edge_node_recovery_candidate (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  state TEXT NOT NULL CHECK (state = 'durably_fenced_candidate'),
  recovery_id TEXT NOT NULL,
  candidate_instance_id TEXT NOT NULL UNIQUE,
  backup_id TEXT,
  edge_id TEXT NOT NULL,
  edge_node_id TEXT NOT NULL,
  old_ledger_epoch TEXT NOT NULL,
  proposed_new_epoch TEXT NOT NULL,
  credential_generation INTEGER NOT NULL CHECK (credential_generation >= 0),
  handoff_schema_version INTEGER NOT NULL CHECK (handoff_schema_version = 1),
  installed_at_ms INTEGER NOT NULL
);

CREATE TABLE edge_node_backup_attempts (
  attempt_id TEXT PRIMARY KEY,
  backup_id TEXT NOT NULL UNIQUE,
  state TEXT NOT NULL CHECK (state IN ('started', 'success', 'failed')),
  reason_code TEXT,
  artifact_name TEXT NOT NULL UNIQUE,
  artifact_length INTEGER,
  edge_node_id TEXT NOT NULL,
  ledger_epoch TEXT,
  accepted_cursor INTEGER,
  allocation_high_water INTEGER,
  started_at_ms INTEGER NOT NULL,
  artifact_created_at_ms INTEGER,
  completed_at_ms INTEGER,
  CHECK (
    (state = 'started' AND reason_code IS NULL AND completed_at_ms IS NULL)
    OR
    (state = 'success' AND reason_code = 'ok'
      AND artifact_length IS NOT NULL AND ledger_epoch IS NOT NULL
      AND accepted_cursor IS NOT NULL AND allocation_high_water IS NOT NULL
      AND artifact_created_at_ms IS NOT NULL AND completed_at_ms IS NOT NULL)
    OR
    (state = 'failed' AND reason_code IS NOT NULL
      AND reason_code <> 'ok' AND completed_at_ms IS NOT NULL)
  )
);
```

`all_edge_node_migrations()` concatenates all owning arrays, appends recovery
version 23 after the existing ledger version 22, sorts by version, and rejects duplicate versions in a debug
assertion. Add `iotkit-core-recovery` to `DATA_PLANE` in `check-layers` and to
the bilingual architecture crate map. State operations allow only
`absent -> started -> success|failed` or `absent -> failed` for a preflight
failure; terminal rows are immutable, and replay is idempotent only for
identical bounded content.

- [ ] **Step 4: Expose allocation high-water without changing publication behavior**

Add:

```rust
pub fn publication_allocation_high_water(
    conn: &Connection,
) -> Result<i64, PublishError> {
    publication_allocation_sequence(conn)
}
```

Do not change the global `AUTOINCREMENT` schema in this slice; epoch-scoped
allocation belongs to slice 3.

- [ ] **Step 5: Run focused verification and confirm GREEN**

Run:

```bash
cargo test -p iotkit-core-publish allocation_high_water_survives_pruning
cargo test -p iotkit-core-recovery
scripts/check-layers
node scripts/check-okf-docs.mjs
```

Expected: all commands exit 0 and `check-layers` reports the new classified
crate.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock scripts/check-layers \
  edge-node/core/recovery edge-node/core/publish \
  docs/okf/en/architecture/system-overview.md \
  docs/okf/ja/architecture/system-overview.md
git commit -m "feat: establish Edge Node recovery state"
```

### Task 2: Create and Validate a Consistent Full SQLite Snapshot

**Files:**
- Create: `edge-node/core/recovery/src/snapshot.rs`
- Create: `edge-node/core/recovery/tests/unit/snapshot_tests.rs`
- Modify: `edge-node/core/recovery/src/lib.rs`
- Modify: `edge-node/core/recovery/Cargo.toml`

**Interfaces:**
- Consumes: `all_edge_node_migrations()`, ledger identity reads, publish activation/cursor/high-water reads, and a source database path.
- Produces: `create_consistent_snapshot(source, staging, backup_id, now_ms) -> SnapshotArtifact` and `validate_snapshot(path) -> NodeBackupManifest`.

- [ ] **Step 1: Write boundary and concurrency tests**

Cover a source in WAL mode while a second connection continues committing:

```rust
#[test]
fn online_snapshot_is_self_consistent_while_source_advances() {
    let fixture = active_database_with_publications(accepted = 3, allocated = 5);
    let snapshot = temp.path().join("snapshot.db");
    let writer = fixture.start_writer_after_backup_begins();

    let artifact = create_consistent_snapshot(
        fixture.path(),
        &snapshot,
        "node-backup-test",
        1_725_000_000_000,
    ).unwrap();
    writer.join().unwrap();

    assert_eq!(artifact.manifest.accepted_cursor, 3);
    assert_eq!(artifact.manifest.allocation_high_water, 5);
    assert_eq!(query_only_snapshot_rows(&snapshot), artifact.manifest.counts);
    assert!(!snapshot.with_extension("db-wal").exists());
}
```

Add rejection tests for:

- activation epoch different from ledger epoch;
- `C > B`;
- a missing publication in `C+1..B`;
- a measurement publication whose reading is absent;
- any nonempty legacy `target_registry.credential_token`;
- invalid family/subtype/schema JSON;
- allocator sequence different from `B`;
- noncanonical schema/migration rows;
- failed `quick_check` or foreign-key check.

The source fixture includes a sentinel legacy HTTP target bearer. Snapshot
construction dispatches `recovery.snapshot.remove_deployment_credentials`
only against the offline snapshot, clearing
`target_registry.credential_token` before manifest derivation and recording a
redacted audit event. It never mutates the live source. Validation then
requires every target token to be empty. Every online manifest test also
asserts `snapshot_mode == Online` and `shutdown_seal_id.is_none()`. The
container contains exactly the manifest and one sanitized SQLite database byte
stream; external configuration, HTTP/MQTT credentials, and TLS key files are
never traversed or packed.

- [ ] **Step 2: Run the tests and confirm RED**

Run:

```bash
cargo test -p iotkit-core-recovery snapshot
```

Expected: compile failure because `snapshot` and its public functions do not
exist.

- [ ] **Step 3: Implement online backup and manifest derivation**

Enable rusqlite's `backup` feature. Open the source read-only, create the
destination with `create_new`, and use `rusqlite::backup::Backup` rather than
copying database/WAL files:

```rust
pub struct SnapshotArtifact {
    pub path: PathBuf,
    pub manifest: NodeBackupManifest,
}

pub fn create_consistent_snapshot(
    source: &Path,
    staging: &Path,
    backup_id: &str,
    now_ms: i64,
) -> Result<SnapshotArtifact, RecoveryError>;
```

After the online backup closes, reopen only the snapshot. Run the canonical
migration-set comparison, dispatch the snapshot-only credential-removal
operation, then run `PRAGMA quick_check`, `PRAGMA foreign_key_check`,
identity/activation/cursor/high-water checks, the empty-target-token
invariant, contiguous unacknowledged range, measurement materialization,
closed record-family validation, table counts, byte length, and SHA-256.
Derive every manifest value from this sanitized snapshot. A fixture sentinel
must remain unchanged in the concurrently running source DB and be absent from
the snapshot bytes, container bytes, diagnostics, audit detail, and restored
candidate.

- [ ] **Step 4: Run focused tests and confirm GREEN**

Run:

```bash
cargo test -p iotkit-core-recovery snapshot
```

Expected: all snapshot tests pass, including the concurrent writer case.

- [ ] **Step 5: Commit**

```bash
git add edge-node/core/recovery
git commit -m "feat: create consistent Edge Node snapshots"
```

### Task 3: Implement the Node-Specific Authenticated Encrypted Container

**Files:**
- Create: `edge-node/core/recovery/src/container.rs`
- Create: `edge-node/core/recovery/tests/unit/container_tests.rs`
- Create: `edge-node/core/recovery/contracts/node-backup-header-v1.schema.json`
- Create: `edge-node/core/recovery/contracts/node-backup-manifest-v1.schema.json`
- Create: `edge-node/core/recovery/tests/fixtures/node-backup-header-v1.json`
- Create: `edge-node/core/recovery/tests/fixtures/node-backup-manifest-v1.json`
- Create: `edge-node/core/recovery/tests/fixtures/node-backup-v1.bin`
- Modify: `edge-node/core/recovery/src/lib.rs`
- Modify: `edge-node/core/recovery/src/model.rs`
- Modify: `edge-node/core/recovery/Cargo.toml`

**Interfaces:**
- Consumes: `NodeBackupManifest`, a plaintext snapshot path, and `BackupPassphrase`.
- Produces: `encrypt_container()`, streaming `authenticate_container()`, and
  `decrypt_container_to_new_file()` with the exact Node-specific format. Task
  5 wraps streaming authentication as public `inspect_backup()`.

- [ ] **Step 1: Write crypto format and negative tests**

Require round-trip plus strict negative behavior:

```rust
#[test]
fn exact_header_bytes_are_authenticated() {
    let artifact = encrypted_fixture();
    for offset in artifact.header_range() {
        let changed = artifact.with_flipped_byte(offset);
        assert_matches!(
            authenticate_container(&changed, &passphrase()),
            Err(RecoveryError::AuthenticationFailed | RecoveryError::ContainerInvalid)
        );
    }
}

#[test]
fn edge_server_backup_magic_is_rejected() {
    assert_eq!(
        authenticate_container(
            edge_server_backup_fixture(),
            &passphrase(),
        )
            .unwrap_err()
            .reason_code(),
        "container_invalid",
    );
}
```

Also cover wrong passphrase, truncated chunk, trailing bytes, duplicate terminal
chunk, KDF values outside every bound, oversized header/chunk, modified
manifest, modified database, and existing output refusal. Salt and nonce-prefix
decoding must be exact-length (16 bytes each), and invalid base64 is rejected
before KDF work.

The checked-in binary is a deliberately public conformance vector over a
minimal sanitized database. Its fixed input string and salt/nonce bytes are
documented as public format-vector material, never a deployment credential or
randomness source. Deterministic entropy injection exists only under
`cfg(test)`; production encryption always uses OS randomness. Tests decrypt
the golden binary, validate its header/manifest against the checked-in JSON
schemas and JSON goldens, and re-encode it byte-for-byte with the test-only
entropy source. Unknown fields, wrong patterns/bounds, and any Rust/schema/
fixture disagreement fail conformance.

- [ ] **Step 2: Run tests and confirm RED**

Run:

```bash
cargo test -p iotkit-core-recovery container
```

Expected: compile failure because the container module is absent.

- [ ] **Step 3: Implement fixed dispatch and chunk encryption**

Use a distinct magic such as `b"IOTKNDB1"` and header:

```rust
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContainerHeader {
    artifact_kind: String,       // "iotkit_edge_node_database"
    format_version: u32,         // 1
    kdf: String,                 // "argon2id"
    salt_b64: String,
    kdf_time: u32,
    kdf_memory_kib: u32,
    kdf_parallelism: u32,
    cipher: String,              // "xchacha20-poly1305"
    nonce_prefix_b64: String,
    chunk_size: usize,
}
```

Hash `MAGIC || header_length_be || exact_header_json` and use that digest plus
the chunk sequence as AEAD associated data for every chunk. A data record is
`flags:u8 || plaintext_length:u32be || ciphertext_and_tag`, where the nonce is
the 16-byte random prefix followed by the `u64be` sequence and the associated
data is `header_digest || sequence:u64be || flags || plaintext_length:u32be`.
Sequence overflow, unknown flags, or a ciphertext length inconsistent with the
authenticated plaintext length is invalid. Encrypt
`manifest_length:u32be || manifest_json || sqlite_bytes`; bound the outer
header to 16 KiB and the inner manifest to 1 MiB before allocation. Require one
authenticated zero-length terminal record with the terminal flag, followed by
EOF. Reject data after terminal or terminal before the manifest/database
length is satisfied. Zeroize the derived key and passphrase owner.

`authenticate_container()` streams and authenticates the entire artifact,
computes database length/SHA-256, and discards database plaintext; it never
creates a plaintext file. `decrypt_container_to_new_file()` uses the same
parser but writes database bytes only to a caller-precreated owner-only tmpfs
file after the authenticated manifest prefix passes capacity checks. Both
require the computed length/digest to equal the manifest before success.

- [ ] **Step 4: Run focused tests and confirm GREEN**

Run:

```bash
cargo test -p iotkit-core-recovery container
```

Expected: all container tests pass.

- [ ] **Step 5: Commit**

```bash
git add edge-node/core/recovery
git commit -m "feat: encrypt Edge Node backup containers"
```

### Task 4: Validate Owner Configuration and Mounted Destination Capabilities

**Files:**
- Create: `edge-node/core/recovery/src/config.rs`
- Create: `edge-node/core/recovery/src/destination.rs`
- Create: `edge-node/core/recovery/tests/unit/config_tests.rs`
- Create: `edge-node/core/recovery/tests/unit/destination_tests.rs`
- Create: `edge-node/core/recovery/tests/support/mountinfo.rs`
- Modify: `edge-node/core/recovery/src/lib.rs`
- Modify: `edge-node/core/recovery/Cargo.toml`

**Interfaces:**
- Consumes: absolute database/destination/staging/passphrase paths and Linux `/proc/self/mountinfo`.
- Produces: `BackupConfig::load_owner_only()`, `configure_backup()`, `verify_destination()`, `publish_verified_artifact()`, and `apply_retention()`.

- [ ] **Step 1: Write config, mount, and no-fallback tests**

Use mountinfo fixtures for ext4, NFS, SMB, bind mounts, escaped spaces, and a
same-name local directory after the expected mount disappears. Add an injected
path-swap/unmount immediately after verification and prove all later operations
stay on the already opened directory descriptor:

```rust
#[test]
fn missing_expected_mount_never_accepts_local_directory_fallback() {
    let config = configured_remote_mount("/mnt/iotkit", "server:/backups", "nfs4");
    fs::create_dir_all("/tmp/test-root/mnt/iotkit").unwrap();
    let actual = mount_table_without("/mnt/iotkit");
    assert_reason(
        verify_destination_with(&config, &actual, &real_fs()),
        "mount_missing",
    );
}
```

Add tests that reject relative paths, destination inside the database/staging
tree, staging not on tmpfs, passphrase/config modes broader than `0600`,
destination broader than owner access, a destination on the same mounted
filesystem/device as the live database, mismatched source/type, read-only/full
mounts, unsupported no-replace/parent-sync/read-back behavior, symlinks, and
retention attempts against unknown or unverified files. Exercise exact-boundary,
one-byte-short, and `u64` overflow capacity cases for staging, destination, and
candidate filesystems using:

```rust
fn required_capacity(bytes: u64) -> Result<u64, RecoveryError> {
    let five_percent = bytes.checked_add(19)
        .ok_or(RecoveryError::CapacityOverflow)? / 20;
    bytes.checked_add(five_percent.max(64 * 1024 * 1024))
        .ok_or(RecoveryError::CapacityOverflow)
}
```

- [ ] **Step 2: Run tests and confirm RED**

Run:

```bash
cargo test -p iotkit-core-recovery config
cargo test -p iotkit-core-recovery destination
```

Expected: compile failure for missing modules.

- [ ] **Step 3: Implement schema-1 config and mount parsing**

`configure_backup()` writes a mode-`0600` temporary sibling, syncs it,
no-replace publishes it, and syncs the parent. Existing configuration is
refused unless the CLI supplied the explicit `--replace-existing` policy; that
path first validates the existing file as owner-only and atomically replaces
only that exact regular file. It captures the deepest matching
mount point, decoded source, filesystem type, and stable filesystem identity.
For a local block-backed mount the identity is its `/dev/disk/by-uuid` UUID;
for network/other mounts it is the `fstatfs` filesystem ID combined with the
decoded mount source. Configuration fails with `mount_identity_unavailable`
rather than recording only a mutable `/dev/sdX` name. Every run opens the
directory without following symlinks and compares `fstat`, `fstatfs`, mount
source/type, and the persisted filesystem identity. It writes a systemd
drop-in containing only:

```ini
[Unit]
RequiresMountsFor=/absolute/mount/point
```

The raw mount source remains only in the owner-only config and error `Display`
uses closed reason codes. Config, passphrase, and handoff reads open with
`O_NOFOLLOW|O_CLOEXEC`, then validate the opened descriptor is a regular,
single-link file owned by the effective user with no group/other bits. Config
and handoff JSON are bounded to 64 KiB; passphrase input is bounded to 4,098
bytes before UTF-8/scalar validation. Metadata-check-then-reopen patterns are
forbidden.

- [ ] **Step 4: Implement capability probe, verified publication, and retention**

The probe creates random product-prefixed files with `openat(O_CREAT|O_EXCL|
O_NOFOLLOW, 0600)` inside the held destination descriptor, checks owner mode,
file sync, `renameat2(RENAME_NOREPLACE)`, parent-descriptor sync, and
byte-for-byte descriptor-relative read-back, then removes only its own names.
Unsupported `renameat2` semantics fail closed. Artifact publication repeats
those operations and authenticates the final artifact before success.
Retention enumerates and opens entries relative to the held descriptor; it
accepts only regular, non-symlink files that decrypt/authenticate, match the
configured node, are recorded as successful, and are older than a newer
successful artifact. Capacity probes
fail before snapshot work when the source-length estimate cannot fit in tmpfs
or the destination. After snapshot validation, the destination check runs
again with `manifest.database_length`; it never assumes ciphertext is smaller
than the database.

- [ ] **Step 5: Run focused tests and confirm GREEN**

Run:

```bash
cargo test -p iotkit-core-recovery config
cargo test -p iotkit-core-recovery destination
```

Expected: all config/destination tests pass.

- [ ] **Step 6: Commit**

```bash
git add edge-node/core/recovery
git commit -m "feat: validate Edge Node backup destinations"
```

### Task 5: Orchestrate Backup Creation, Inspection, Status, and Safe Retention

**Files:**
- Create: `edge-node/core/recovery/src/backup.rs`
- Create: `edge-node/core/recovery/tests/unit/backup_tests.rs`
- Create: `edge-node/core/recovery/tests/backup_contract.rs`
- Modify: `edge-node/core/recovery/src/state.rs`
- Modify: `edge-node/core/recovery/src/lib.rs`

**Interfaces:**
- Consumes: snapshot/container/destination APIs, `BackupConfig`, passphrase, and typed backup-outcome descriptor.
- Produces: public `create_backup()`, `inspect_backup()`, and `backup_status()`.

- [ ] **Step 1: Write end-to-end library contract tests**

Test a live database with device-token hashes, readings, publication rows,
dedup rows, quarantine state, activation, and audit:

```rust
#[test]
fn encrypted_backup_round_trips_every_custody_table_without_secret_output() {
    let fixture = complete_sensitive_database();
    let manifest = create_backup(&fixture.config, &fixture.passphrase, fixture.now).unwrap();
    let inspected = inspect_backup(&fixture.artifact_path(&manifest), &fixture.passphrase).unwrap();

    assert_eq!(inspected, manifest);
    assert_eq!(manifest.accepted_cursor, fixture.c);
    assert_eq!(manifest.allocation_high_water, fixture.b);
    assert_no_secret_in(&format!("{manifest:?}"));
    assert_no_secret_in(&fixture.captured_diagnostics());
}
```

Add status tests for `not_configured`, `healthy`, `stale`, and `failed`; a
crash after durable `started`, a crash after artifact publication but before
the success receipt; read-only status reporting of the incomplete receipt;
status during an actively held create lock returning `operation_busy` without
misclassifying the attempt as failed;
later next-create reconciliation of the exact authenticated artifact named by
that started attempt; refusal to adopt any unreferenced
artifact; a failed latest attempt
overriding an older healthy attempt; and retention only after a newer success.
Add a concurrent-create test proving the loser returns `operation_busy` before
opening the source snapshot, plus restart cleanup tests proving only
product-prefixed, owner-only, regular, single-link plaintext staging files are
removed after the prior process lock has been released.

Inject backup failures separately after encrypted-file sync, rename success,
parent-directory sync, published read-back, and success-receipt commit. A
rename-success/parent-sync failure leaves only the exact named ciphertext
referenced by the started attempt; the next create re-syncs the held parent,
authenticates read-back, and completes that same attempt before starting new
work. No phase reports success before the receipt commit.

- [ ] **Step 2: Run tests and confirm RED**

Run:

```bash
cargo test -p iotkit-core-recovery --test backup_contract
cargo test -p iotkit-core-recovery backup
```

Expected: missing orchestration functions.

- [ ] **Step 3: Add typed backup-outcome operation**

Expose descriptors:

```rust
pub const BEGIN_BACKUP_ATTEMPT_OP: &str = "recovery.backup.begin";
pub const COMPLETE_BACKUP_ATTEMPT_OP: &str = "recovery.backup.complete";
pub const RECORD_BACKUP_PREFLIGHT_FAILURE_OP: &str =
    "recovery.backup.record_preflight_failure";
pub const REMOVE_SNAPSHOT_DEPLOYMENT_CREDENTIALS_OP: &str =
    "recovery.snapshot.remove_deployment_credentials";
pub const INSTALL_CANDIDATE_OP: &str = "recovery.candidate.install";

pub fn recovery_descriptors() -> &'static [OpDescriptor];
```

`begin` is a local-CLI construction operation with exact bounded params. After
the snapshot is validated but before any encrypted artifact publication it
durably stores the fresh attempt/backup IDs, safe artifact basename, node ID,
and start time. Therefore the snapshot does not contain a self-referential
in-progress attempt. `complete` permits only the one-way
`started -> success|failed` transition. Success stores ciphertext length,
epoch, C/B, artifact creation/completion times, and `ok`; failure stores a
closed reason/completion time. `record_preflight_failure` inserts a terminal
failed row when mount/capacity/snapshot validation fails before `begin`; it
uses the already generated attempt/backup IDs and basename but cannot later
become success. The dispatcher supplies audit and rollback; raw
config, mount source, full path, manifest bytes, and database digest do not
enter operation params. Because the current dispatcher audits declared params
and targets verbatim except for recognized sensitive keys, every recovery
descriptor declares one bounded `private_recovery_state` object, reads its
typed content internally, and returns an empty target list. The existing
redactor therefore stores `[REDACTED]` for that object while the audit row
retains only operation/outcome class and time. Tests query `r14_op` ledger
events and prove that node, Edge, backup, attempt, recovery, and candidate IDs
are absent.

- [ ] **Step 4: Implement backup orchestration**

Order:

1. load/validate owner config, acquire the nonblocking staging lock, and read
   the passphrase;
2. reconcile any lingering `started` attempt by authenticating only its exact
   recorded final basename, then remove only safely classified stale plaintext
   names from the previous released-lock owner;
3. verify source identity without migration;
4. generate attempt/backup IDs plus the safe final basename;
5. verify mount identity/capabilities and tmpfs staging;
6. create and validate online snapshot using that backup ID;
7. dispatch `begin` for the validated snapshot and intended basename;
8. encrypt and sync a private destination temporary file;
9. no-replace rename to the final basename;
10. sync the held parent descriptor;
11. reopen/authenticate the published artifact;
12. dispatch exact-attempt `complete(success)`;
13. apply safe retention;
14. remove plaintext staging and release the lock.

On any failure, remove only owned temporary names, record a closed failure
reason when the source DB is safely writable, and never call legacy snapshot.
Failures before `begin` use `record_preflight_failure`; failures after it use
`complete(failed)` only when rename definitely did not occur and the final name
is absent. At or after rename success—or whenever rename outcome/publication
durability is uncertain—the attempt remains `started`, the command returns
`artifact_publication_uncertain`, and next-create exact-name reconciliation is
the only path to success/terminal failure. A config/passphrase error that
prevents safely identifying or opening the source database is returned without
inventing a database receipt.
At the start of the next `create`, a lingering `started` attempt becomes
success only when its
exact final basename exists, fully authenticates, and matches the recorded
backup/node IDs; otherwise it becomes `failed(interrupted)` without adopting
or deleting unrelated files. `backup status` is read-only and first tries a
nonblocking shared lock. If the create lock is held it returns
`operation_busy`, not `failed`. A `started` row observed after the exclusive
creator lock is free is an interrupted/publication-pending failure state and
is reported with a closed reason without mutating the database.

- [ ] **Step 5: Run focused tests and confirm GREEN**

Run:

```bash
cargo test -p iotkit-core-recovery
```

Expected: all recovery crate tests pass.

- [ ] **Step 6: Commit**

```bash
git add edge-node/core/recovery
git commit -m "feat: create and track encrypted Node backups"
```

### Task 6: Restore an Unpublished, Self-Contained, Durably Fenced Candidate

**Files:**
- Create: `edge-node/core/recovery/src/restore.rs`
- Create: `edge-node/core/recovery/tests/unit/restore_tests.rs`
- Create: `edge-node/core/recovery/contracts/recovery-handoff-v1.schema.json`
- Create: `edge-node/core/recovery/contracts/restore-receipt-v1.schema.json`
- Create: `edge-node/core/recovery/tests/fixtures/recovery-handoff-v1.json`
- Create: `edge-node/core/recovery/tests/fixtures/restore-receipt-v1.json`
- Modify: `edge-node/core/recovery/src/state.rs`
- Modify: `edge-node/core/recovery/src/lib.rs`
- Modify: `edge-node/core/recovery/tests/backup_contract.rs`

**Interfaces:**
- Consumes: an authenticated artifact, `RecoveryHandoff`, authoritative
  configured live-database path, absent candidate path, passphrase, and the
  install-candidate operation.
- Produces: `restore_candidate()`, `RestoreReceipt`, and a candidate whose `startup_mode()` is `FencedCandidate`.

- [ ] **Step 1: Write restore ordering and negative tests**

Cover wrong node/epoch/recovery handoff, candidate equal to the configured live
path while that live file is both present and absent, conflicting existing/
symlink/hardlink target, exact already-fenced replay, corrupt
snapshot, a concurrent restore to the same target, injected interruption at
every phase, and restored active state:

Load the checked-in handoff/receipt JSON through both serde types and their JSON
schemas, require byte-stable canonical serialization, and add negative
fixtures for unknown fields, bad ID patterns, missing expected backup,
negative credential generation, equal old/proposed epochs, and receipt fields
that differ from the installed candidate row.

```rust
#[test]
fn published_candidate_is_already_fenced_and_wal_independent() {
    let restored = restore_fixture().run().unwrap();
    assert!(!restored.candidate.with_extension("db-wal").exists());
    let conn = Connection::open_with_flags(
        &restored.candidate,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).unwrap();
    assert_matches!(
        startup_mode(&conn).unwrap(),
        RecoveryStartupMode::FencedCandidate { .. }
    );
    assert_eq!(auth_recovery_required(&conn), true);
    assert_eq!(applied_ingress_generation(&conn), 0);
    assert_eq!(ledger_epoch(&conn), restored.manifest.ledger_epoch);
}
```

For each phase (`decrypted`, `copied`, `fence_committed`, `checkpointed`,
`candidate_file_synced`, `rename_succeeded`, `parent_synced`,
`published_readback_verified`), inject an error. Before rename, the final
candidate name is absent. After rename, it may exist but must already be
fenced, WAL-independent, and byte-identical to the exact handoff/artifact
receipt; an unfenced named candidate never exists. The live path is unchanged
in every phase.

- [ ] **Step 2: Run tests and confirm RED**

Run:

```bash
cargo test -p iotkit-core-recovery restore
cargo test -p iotkit-core-recovery --test backup_contract restore
```

Expected: missing restore module/functions.

- [ ] **Step 3: Implement the install-candidate typed operation**

The descriptor validates exact handoff/manifest identity and writes the
singleton candidate row in `durably_fenced_candidate`. Slice 1 never writes or
transitions to `fenced_waiting_permit`; that typed startup transition belongs
to slice 2 after the restricted recovery runtime exists. Slice 1 requires
`handoff.expected_backup_id == Some(manifest.backup_id)`; `None` is reserved
for the later `initialize-empty` path and is rejected by restore. Credential
generation must be nonnegative, and the proposed new epoch must differ from
the old epoch. It also requires `handoff.edge_id` to equal the backed-up
`edge_node_activation.edge_id`, and requires the handoff node/old epoch to
match both the manifest and ledger/activation rows. In the same dispatcher
transaction it:

```rust
let new_auth_epoch = iotkit_core_ops::new_auth_epoch()?;
iotkit_core_ops::enter_restored_local_recovery(tx, &new_auth_epoch)?;
install_candidate_row(tx, request)?;
```

It does not renew the ledger epoch. `candidate_instance_id` is fresh random
128-bit lowercase hex generated after decrypt and excluded from the artifact.
The operation is idempotent only for byte-identical IDs/content. It uses the
same bounded `private_recovery_state` param and empty targets, so the generic
dispatcher audit records the closed transition/result class but never handoff,
node/Edge/recovery/candidate/backup IDs, credential generation, or raw JSON.

- [ ] **Step 4: Implement safe restore ordering**

Use this exact order:

1. require normalized absolute live/candidate paths, open both parents with
   `O_DIRECTORY|O_NOFOLLOW`, and compare parent `(st_dev, st_ino)` plus raw
   basename without following the candidate; reject equality even when the
   configured live file is absent;
2. lock the candidate target; reject an existing symlink/hardlink/conflicting
   candidate unless it is an exact already-fenced replay of this artifact and
   handoff, and return `operation_busy` for a competing restore;
3. authenticate/decrypt only the bounded manifest prefix in memory, without
   writing database plaintext;
4. require `required_capacity(manifest.database_length)` on both tmpfs and the
   candidate filesystem;
5. finish authenticated streaming decryption into an owner-only tmpfs file and
   verify the database length/digest;
6. create an
   unpublished mode-`0600` temporary database relative to that descriptor;
7. copy bytes, open offline, run canonical/integrity/boundary validation;
8. dispatch install-candidate on the temporary DB;
9. switch to offline DELETE journal or checkpoint/truncate, close all connections, and prove no WAL is required;
10. reopen read-only, rerun integrity/fence/authority/identity checks;
11. sync the closed DB;
12. `renameat2(RENAME_NOREPLACE)` publish to the candidate name;
13. sync the held parent descriptor;
14. reopen read-only and verify the published receipt;
15. remove plaintext staging.

Rename success, parent-sync success, and published read-back are separate
durability phases. If rename succeeds but a later step fails, return
`candidate_publication_uncertain` and leave the already fenced name in place.
An exact rerun opens it without mutation, verifies the stored row against the
artifact/handoff, syncs the parent, performs read-back, and returns the same
receipt. A different artifact/handoff/candidate identity is
`candidate_conflict`; it is never overwritten or adopted.

- [ ] **Step 5: Run focused tests and confirm GREEN**

Run:

```bash
cargo test -p iotkit-core-recovery
```

Expected: all recovery tests pass, including every injected interruption.

- [ ] **Step 6: Commit**

```bash
git add edge-node/core/recovery
git commit -m "feat: restore durably fenced Node candidates"
```

### Task 7: Add Local-Root `nodectl backup` Commands Without Secret Leakage

**Files:**
- Create: `edge-node/apps/nodectl/src/cmd/backup.rs`
- Create: `edge-node/apps/nodectl/tests/backup_cli.rs`
- Modify: `edge-node/apps/nodectl/src/main.rs`
- Modify: `edge-node/apps/nodectl/Cargo.toml`

**Interfaces:**
- Consumes: recovery public APIs and owner-only files.
- Produces: `backup configure|create|inspect|status|restore` CLI and stable nonsecret JSON.

- [ ] **Step 1: Write subprocess contract tests**

Required command shapes:

```text
iotkit-edge-nodectl backup configure --config FILE --db DB \
  --destination DIR --staging-directory TMPFS --passphrase-file FILE \
  --freshness-seconds 86400 --retention-count 7 --systemd-drop-in FILE \
  [--replace-existing]
iotkit-edge-nodectl backup create --config FILE
iotkit-edge-nodectl backup inspect --input FILE --passphrase-file FILE
iotkit-edge-nodectl backup status --config FILE
iotkit-edge-nodectl backup restore --input FILE --candidate-db FILE \
  --live-db CONFIGURED_DB --passphrase-file FILE --recovery-handoff FILE
```

`restore` accepts only a schema-valid handoff whose expected backup matches the
artifact. Slice 1 ships no production handoff producer and no flag that
fabricates or bypasses one. CLI conformance tests use the checked-in public
fixture; a real operator handoff will be emitted by the slice-2 IoTKit Edge
recovery case. `--live-db` is required and must be copied exactly from the
Edge Node service configuration; it is carried into `RestoreRequest` even when
that file is currently absent.

Tests assert:

- no `--passphrase` flag exists;
- an existing config is refused unless `--replace-existing` is explicit;
- config/passphrase/handoff modes broader than `0600` fail;
- stdout JSON contains IDs, state, C/B, and reason codes only;
- captured stdout/stderr never contains passphrase, token/hash fixture, raw
  mount source, or full sensitive path;
- backup commands take their own early route and do not create/migrate a
  missing live DB;
- restore refuses a conflicting existing target; an exact already-fenced
  replay completes parent-sync/read-back and returns the stored receipt;
- legacy `snapshot` behavior remains unchanged and is never invoked.

- [ ] **Step 2: Run tests and confirm RED**

Run:

```bash
cargo test -p iotkit-edge-nodectl --test backup_cli
```

Expected: clap rejects the missing `backup` command.

- [ ] **Step 3: Implement command parsing and owner-only reads**

Add a `BackupCommand` subcommand and route it before the generic `--db`
initialization path. Reuse the permission rule from IoTKit Edge CLI, but open
and validate one descriptor rather than checking a path and reopening it:

```rust
fn read_owner_only_secret(path: &Path) -> Result<BackupPassphrase, CliBackupError> {
    let mut file = open_readonly_nofollow(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file()
        || metadata.nlink() != 1
        || metadata.uid() != effective_uid()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(CliBackupError::OwnerOnlyFileRequired);
    }
    BackupPassphrase::read_bounded(&mut file, 4_098)
}
```

Do not log `Debug` representations of clap args/config. JSON errors use the
closed recovery reason code and a nonsecret action.

The passphrase parser accepts UTF-8, removes at most one terminal `\n` and its
optional preceding `\r`, rejects any remaining CR/LF/NUL, then applies the
12–1024 Unicode-scalar bound. Tests cover newline-terminated secret files,
embedded newline rejection, invalid UTF-8, and both length boundaries.

For non-backup commands, replace the duplicated storage/ledger/timeseries/
registry/publish/ops migration assembly in `main.rs` with
`all_edge_node_migrations()`. Backup `create`, `inspect`, `status`, and
`restore` still take the early route and must not initialize or migrate a
missing live database as a side effect.

- [ ] **Step 4: Implement exact JSON output and drop-in creation**

Success JSON examples:

```json
{"status":"created","backup_id":"node-backup-...","edge_node_id":"...","ledger_epoch":"...","accepted_cursor":12,"allocation_high_water":15,"created_at_ms":1725000000000}
```

```json
{"schema_version":1,"status":"durably_fenced_candidate","recovery_id":"recovery-...","candidate_instance_id":"candidate-...","backup_id":"node-backup-...","edge_id":"edge-...","edge_node_id":"...","old_ledger_epoch":"...","proposed_new_epoch":"...","credential_generation":2}
```

`backup status` returns one of `not_configured`, `healthy`, `stale`, or
`failed`. Healthy/stale output includes artifact ID/time/ciphertext size, node
identity, epoch, and C/B. Failed output includes the closed failure reason/time
and, when one still exists, the same nonsecret summary for the last verified
artifact. It never prints the destination, mount identity, passphrase path,
database path, database digest, or a serialized `NodeBackupManifest`.

- [ ] **Step 5: Run focused tests and confirm GREEN**

Run:

```bash
cargo test -p iotkit-edge-nodectl --test backup_cli
cargo test -p iotkit-edge-nodectl --test cli legacy_snapshot
```

Expected: new CLI tests and existing legacy snapshot refusal tests pass.

- [ ] **Step 6: Commit**

```bash
git add edge-node/apps/nodectl
git commit -m "feat: expose encrypted Node backup CLI"
```

### Task 8: Enforce the Process-Wide Startup Fence

**Files:**
- Create: `edge-node/apps/node/tests/recovery_startup.rs`
- Modify: `edge-node/apps/node/Cargo.toml`
- Modify: `edge-node/apps/node/src/main.rs`
- Modify: `edge-node/apps/node/tests/cutover.rs`

**Interfaces:**
- Consumes: `probe_startup_path()`, `all_edge_node_migrations()`, and
  `startup_mode()`.
- Produces: fail-closed process behavior before effective-config/adapter
  logging, migration, identity/provenance mutation, retention, health writer,
  MQTT, API, collector, ingest, or adapters start.

- [ ] **Step 1: Write a binary-level fence test**

Create a valid active source, restore it as a fenced candidate, launch the Node
binary against that candidate, and assert:

```rust
assert!(!output.status.success());
assert!(stderr.contains("fenced recovery candidate"));
assert!(!stderr.contains("MQTT exit publisher started"));
assert!(!stderr.contains("control-plane API started"));
assert!(!stderr.contains("input adapter instance configured"));
assert!(!stderr.contains("sentinel-sensitive-config"));
assert_eq!(probe_listener_connections(), 0);
assert_eq!(database_mutation_digest_after, database_mutation_digest_before);
```

Put sentinel host/path/source values in the config and assert none are emitted
before the fence. Also launch a normal pre-v23 database and a current normal
database and retain existing migration/startup behavior. A candidate with a
missing/malformed receipt, authority rotation, or recovery row exits generically
without migration, repair, identifiers, or config values in stderr.

- [ ] **Step 2: Run test and confirm RED**

Run:

```bash
cargo test -p iotkit-edge-node --test recovery_startup
```

Expected: restored active state proceeds too far or the recovery dependency is
missing.

- [ ] **Step 3: Gate before runtime construction**

Immediately after the minimal config parse provides the DB path—and before
adapter catalog validation, effective-config logging, migration, identity
initialization, provenance reconciliation, Tokio runtime construction, or any
service/task setup—call `probe_startup_path()` using a read-only,
no-create/no-migrate connection:

```rust
match probe_startup_path(Path::new(&config.db_path)) {
    Ok(RecoveryStartupMode::Normal) => {}
    Ok(RecoveryStartupMode::FencedCandidate { .. }) => {
        tracing::error!("fenced recovery candidate; normal runtime is disabled");
        std::process::exit(3);
    }
    Err(_) => {
        tracing::error!("Edge Node recovery startup state is invalid");
        std::process::exit(3);
    }
}
```

The probe treats a missing DB or absent recovery migration as normal-eligible,
but fails closed when recovery schema/state is present and malformed. Only
after a normal result may main log effective config, call
`all_edge_node_migrations()`, and initialize the DB. It then calls
`startup_mode()` again before identity/provenance mutations as defense in
depth. Slice 1 deliberately exits for `durably_fenced_candidate` and leaves
the row unchanged. Slice 2 will replace this exit with the typed transition to
`fenced_waiting_permit` and restricted recovery runtime after their contract
and tests exist.

- [ ] **Step 4: Run focused tests and confirm GREEN**

Run:

```bash
cargo test -p iotkit-edge-node --test recovery_startup
cargo test -p iotkit-edge-node --test cutover
```

Expected: both pass and the fenced binary performs no normal work.

- [ ] **Step 5: Commit**

```bash
git add edge-node/apps/node
git commit -m "feat: fence restored Node startup"
```

### Task 9: Add Optional systemd Scheduling and Bilingual Operator Authority

**Files:**
- Create: `deploy/systemd/iotkit-edge-node-backup.service`
- Create: `deploy/systemd/iotkit-edge-node-backup.timer`
- Create: `scripts/tests/edge-node-backup-systemd.test.mjs`
- Create: `docs/okf/en/contracts/edge-node-recovery-v1.md`
- Create: `docs/okf/ja/contracts/edge-node-recovery-v1.md`
- Modify: `docs/README.md`
- Modify: `docs/okf/en/contracts/ingest-v1.md`
- Modify: `docs/okf/ja/contracts/ingest-v1.md`
- Modify: `docs/okf/en/operations/installation-and-recovery.md`
- Modify: `docs/okf/ja/operations/installation-and-recovery.md`

**Interfaces:**
- Consumes: the exact CLI from Task 7 plus checked-in schemas/golden fixtures
  from Tasks 3 and 6.
- Produces: optional timer/service templates, generated mount drop-in contract,
  and current bilingual authority for the exact slice-1 format/behavior.

- [ ] **Step 1: Write unit-template tests**

Assert the service contains:

```ini
[Service]
Type=oneshot
UMask=0077
RuntimeDirectory=iotkit-edge-node-backup
RuntimeDirectoryMode=0700
Environment=TMPDIR=/run/iotkit-edge-node-backup
ExecStart=/usr/local/bin/iotkit-edge-nodectl backup create --config /etc/iotkit/edge-node-backup.json
```

Assert the timer is disabled until explicitly enabled and contains:

```ini
[Timer]
OnCalendar=daily
RandomizedDelaySec=2h
Persistent=true
```

The test also invokes/configures a temporary drop-in and requires exact
`RequiresMountsFor=<captured mount point>`.

- [ ] **Step 2: Run test and confirm RED**

Run:

```bash
node --test scripts/tests/edge-node-backup-systemd.test.mjs
```

Expected: missing unit files.

- [ ] **Step 3: Add templates and bilingual runbook**

The new paired recovery contract defines exact magic/framing, header and
manifest fields/bounds, sanitation invariant, KDF/cipher parameters, handoff
and receipt schemas, candidate-row binding, idempotent post-rename recovery,
and the disabled slice-2 surfaces. It links each machine-readable schema and
golden fixture; `docs/README.md` indexes the pair. English/Japanese must make
the same normative statements.

Document:

- no configuration by default;
- owner-only config/passphrase creation;
- supported destination means capability-tested mount, not filesystem-name
  endorsement;
- systemd installation/drop-in/timer enable commands;
- configuring `/run/iotkit-edge-node-backup` as staging: `configure` validates
  the existing `/run` tmpfs parent, and `create` accepts/creates only the exact
  owner-only leaf that systemd's `RuntimeDirectory=` supplies;
- manual `create`, `inspect`, and `status`;
- passphrase escrow and restore drill;
- candidate restore to an absent path;
- the candidate remains fenced and cannot collect/publish in slice 1;
- `restore` is available for conformance/restore-drill validation only with a
  valid handoff; production handoff creation, Broker fencing, and activation
  are not yet shipped, so the runbook must not tell operators to invent a
  handoff or claim a usable replacement journey;
- no legacy snapshot fallback;
- no MQTT/TLS private material in the artifact.

Update the ingest contract's deferred section precisely: encrypted
custody-complete sanitized database backup and local fenced-candidate restore
are implemented, while
Broker fencing, remote permit, reconciliation, dedup risk resolution,
reactivation, and same-ID new epoch remain deferred/default-off. Also replace
the later bilingual statement that replacement never restores readings/dedup:
an encrypted-backup candidate contains readings and dedup claims through the
authenticated snapshot boundary, remains unable to ingest while fenced, and
does not prove anything about post-backup retry state; no-backup replacement
still restores neither. Do not imply that restored dedup is active before the
later permit/generation checks.

- [ ] **Step 4: Run documentation and template tests**

Run:

```bash
node --test scripts/tests/edge-node-backup-systemd.test.mjs
node scripts/check-okf-docs.mjs
scripts/check-layers
scripts/check-source-layout
```

Expected: all exit 0.

- [ ] **Step 5: Commit**

```bash
git add deploy/systemd scripts/tests/edge-node-backup-systemd.test.mjs \
  docs/README.md docs/okf/en/contracts/ingest-v1.md \
  docs/okf/ja/contracts/ingest-v1.md \
  docs/okf/en/contracts/edge-node-recovery-v1.md \
  docs/okf/ja/contracts/edge-node-recovery-v1.md \
  docs/okf/en/operations/installation-and-recovery.md \
  docs/okf/ja/operations/installation-and-recovery.md
git commit -m "docs: operate optional Edge Node backups"
```

### Task 10: Run the Full Slice Gate and Independent Review

**Files:**
- Modify only files required to fix evidence-backed failures found by this task.

**Interfaces:**
- Consumes: completed Tasks 1–9.
- Produces: verification evidence and a review-ready slice; no new feature surface.

- [ ] **Step 1: Run the smallest complete recovery/CLI/runtime suite**

Run:

```bash
cargo test -p iotkit-core-recovery
cargo test -p iotkit-core-publish
cargo test -p iotkit-edge-nodectl
cargo test -p iotkit-edge-node
node --test scripts/tests/edge-node-backup-systemd.test.mjs
```

Expected: all tests pass.

- [ ] **Step 2: Run repository structure and authority checks**

Run:

```bash
node scripts/check-okf-docs.mjs
scripts/check-layers
scripts/check-source-layout
node scripts/battle-tested-review.mjs select --base origin/master \
  --concern edge-node-replacement \
  --concern custody \
  --concern restore \
  --concern power-loss \
  --concern storage \
  --concern storage-pressure
```

Expected: checks pass and BT-001 through BT-004 are reviewed; unmatched paths
are examined rather than treated as safe.

- [ ] **Step 3: Run the broad Rust gate**

Run:

```bash
scripts/verify.sh
```

Expected: formatting, layer/source rules, workspace tests, and Clippy with
`-D warnings` all pass under Rust 1.95.0.

- [ ] **Step 4: Run process interruption tests and record physical-evidence scope**

Run the automated phase-injection matrix from Task 6 again on Linux. Record in
the PR that real Raspberry Pi power-cut/storage-removal evidence remains
required at the integrated release-candidate gate and is not falsely satisfied
by process injection.

- [ ] **Step 5: Dispatch independent review**

Review the complete diff against:

- the approved design;
- current bilingual ingest/recovery authority;
- product invariants for secrets, custody acknowledgement, typed mutation, and
  no silent loss;
- BT-001, BT-002, BT-003, and BT-004;
- crash windows before/after snapshot, encrypted publication, outcome receipt,
  fence commit, WAL checkpoint, and candidate publication.

Fix every confirmed Critical/Important finding on the same branch and rerun the
affected focused command plus `scripts/verify.sh`.

- [ ] **Step 6: Commit review fixes only if needed**

```bash
git add edge-node/core/recovery edge-node/core/publish \
  edge-node/apps/nodectl edge-node/apps/node deploy/systemd \
  scripts/check-layers scripts/tests/edge-node-backup-systemd.test.mjs \
  docs/okf/en docs/okf/ja Cargo.toml Cargo.lock
git diff --cached --check
git commit -m "fix: harden fenced Node backup recovery"
```

If the review finds no actionable issue, do not create an empty commit.

- [ ] **Step 7: Push and open the required draft PR**

Confirm the branch contains only the child-issue scope, push
`agent/issue-N-edge-node-backup-restore`, and open a draft PR whose body:

- uses `Closes #N` for the returned child issue number;
- references parent `#92` without a closing keyword;
- states that backup is optional/default-off and the candidate cannot activate;
- states that production handoff/Broker fence/permit/reconciliation are absent;
- lists focused/full verification and independent-review evidence;
- records that Raspberry Pi power-cut/storage-removal evidence remains a later
  integrated release-candidate gate.

Stop for human review after the draft PR is confirmed. Do not merge, tag, or
release from this plan without a later explicit user approval.

## Slice Completion and Next Plan

This plan is complete when local root can optionally configure and create a
custody-complete encrypted Node artifact, inspect its nonsecret boundary, and
the conformance/drill path can consume a schema-valid recovery handoff to
restore an absent durably fenced candidate and prove the normal runtime will
not start it. Slice 1 does not create a production handoff and does not make
the candidate active; therefore it does not yet deliver an operator-usable
computer-replacement journey.

After merge, write a separate plan for slice 2: IoTKit Edge recovery case
preparation, Broker fence attestation, candidate hello/CAS binding, and the
immutable recovery permit. Do not extend this plan or branch into slice 2.
