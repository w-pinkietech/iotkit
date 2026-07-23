# Rust IoTKit Edge Replacement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Go IoTKit Edge with a functionally equivalent Rust implementation and one Rust-based Adapter development toolchain.

**Architecture:** Build an independent Rust `iotkit-edge` executable beside the Go oracle, implement external behavior in vertical slices, and compare both through language-neutral fixtures and process tests. Keep Edge internals in one crate with private responsibility modules; expose Output Adapter API, testkit, example, and each built-in Adapter as separate leaf crates. Switch deployment only after all Rust journeys pass, then delete Go in the final cutover.

**Tech Stack:** Rust workspace, Tokio, Axum, Tower, Askama, SQLx SQLite/PostgreSQL, rumqttc/Rustls, Clap, Serde, Argon2, XChaCha20-Poly1305, TypeScript, Mosquitto, Docker Compose.

## Global Constraints

- Issue #83 and one draft PR contain the replacement; commits remain independently reviewable and green.
- Wasm, dynamic plugins, FFI, gRPC, production dual-write, `sqlx::Any`, ORM, generic event bus, SPA conversion, and UI redesign are prohibited.
- Go-era databases and backup artifacts are not read or converted; Rust starts with a fresh schema.
- MQTT, HTTP, Console, auth, semantic, Output Adapter, custody, backup/restore, diagnostics, and CLI external behavior remains compatible.
- Go remains an isolated black-box oracle until the final cutover commit.
- Output Adapter implementations are trusted compile-time Rust crates and modify only their crate, workspace membership, and one static registry.
- SQLite and PostgreSQL use backend-specific SQL/migrations behind operation-level application ports.
- MQTT enqueue is not PUBACK; custody acknowledgement is emitted only after raw/cursor commit.
- Detached production tasks and cancellation of an in-flight commit are prohibited.
- Target-hardware capacity measurement is outside this Issue; host capacity regression remains required.
- Use test-first RED/GREEN cycles and commit at every task boundary.

---

### Task 1: Freeze external parity surfaces and create the Rust composition skeleton

**Files:**
- Modify: `Cargo.toml`
- Create: `edge/Cargo.toml`
- Create: `edge/src/lib.rs`
- Create: `edge/src/main.rs`
- Create: `edge/src/config/mod.rs`
- Create: `edge/src/application/mod.rs`
- Create: `edge/src/lifecycle.rs`
- Create: `edge/tests/cli_surface.rs`
- Create: `edge/tests/parity/go_surface_snapshot.rs`
- Create: `testdata/edge-parity/v1/manifest.json`
- Create: `scripts/test-edge-parity.sh`
- Modify: `scripts/check-layers`
- Modify: `scripts/check-source-layout`

**Interfaces:**
- Produces: `iotkit_edge::Application`, `iotkit_edge::lifecycle::Supervisor`,
  and a temporary Rust executable invoked as `cargo run -p iotkit-edge --`.
- Produces: a parity manifest listing CLI, HTTP, MQTT, Console, CSV, backup,
  and diagnostics scenarios with exact fixture paths.
- Consumes: current Go binary and language-neutral files under `testdata/`.

- [ ] **Step 1: Add a failing workspace/package inventory test**

Add a script test asserting that package `iotkit-edge` exists, exposes a
library and binary target, and is selected by Edge-only CI. It must also assert
that Go remains present until the manifest reports every parity group complete.

Run:

```bash
node --test scripts/tests/select-ci-jobs.test.mjs
cargo metadata --no-deps --format-version 1
```

Expected: FAIL because the Rust Edge package is absent.

- [ ] **Step 2: Add the minimal package and typed lifecycle**

Define:

```rust
pub struct Application {
    supervisor: lifecycle::Supervisor,
}

pub enum ExitReason {
    Requested,
    CriticalTaskFailed { task: &'static str },
    ShutdownTimedOut,
}

pub struct Supervisor {
    cancellation: tokio_util::sync::CancellationToken,
    tasks: tokio::task::JoinSet<Result<(), CriticalTaskError>>,
}
```

`main.rs` parses only `--version` and `serve` initially, owns the Tokio runtime,
and returns non-zero for a critical task failure. No detached task is allowed.

- [ ] **Step 3: Add the Go surface snapshot**

Use process execution against the Go binary to save stable command names,
OpenAPI path/method pairs, MQTT fixture names, HTML route names, and release
gate stages into `manifest.json`. Exclude timestamps, UUIDs, and password
hashes; do not snapshot internal SQL or Go type names.

Run:

```bash
scripts/test-edge-parity.sh surface
cargo test -p iotkit-edge --test cli_surface
```

Expected: PASS with Go oracle and Rust skeleton both discoverable.

- [ ] **Step 4: Verify focused checks and commit**

Run:

```bash
cargo fmt --all --check
cargo clippy -p iotkit-edge --all-targets -- -D warnings
cargo test -p iotkit-edge
scripts/check-layers
scripts/check-source-layout
```

Commit:

```bash
git commit -m "feat(edge): establish Rust composition and parity harness"
```

### Task 2: Implement the Rust Output Adapter authoring boundary

**Files:**
- Modify: `Cargo.toml`
- Create: `edge/output-adapters/README.md`
- Create: `edge/output-adapters/README.ja.md`
- Create: `edge/output-adapters/api/Cargo.toml`
- Create: `edge/output-adapters/api/src/lib.rs`
- Create: `edge/output-adapters/testkit/Cargo.toml`
- Create: `edge/output-adapters/testkit/src/lib.rs`
- Create: `edge/output-adapters/example/Cargo.toml`
- Create: `edge/output-adapters/example/src/lib.rs`
- Create: `edge/output-adapters/generic-mqtt-json-v1/Cargo.toml`
- Create: `edge/output-adapters/generic-mqtt-json-v1/src/lib.rs`
- Create: `edge/output-adapters/pinikiet-mqtt-v1/Cargo.toml`
- Create: `edge/output-adapters/pinikiet-mqtt-v1/src/lib.rs`
- Create: `edge/src/composition/mod.rs`
- Create: `edge/src/composition/output_adapters.rs`
- Create: `edge/tests/output_registry.rs`
- Modify: `scripts/check-layers`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Produces: `OutputAdapter`, `ProfilePolicy`, `Descriptor`,
  `Observation`, `ObservationKind`, `RouteConfig`, `MqttPublication`, and
  `AdapterError` in `iotkit-output-adapter-api`.
- Produces: `assert_adapter_conformance(adapter, cases)` in the testkit.
- Produces: `registered_output_adapters()` as the only production registry.
- Consumes: `testdata/output/v1/` fixtures and OKF Output Adapter v1 contract.

- [ ] **Step 1: Write failing API and fixture tests**

The API tests require typed variants:

```rust
pub enum ObservationValue {
    Numeric(f64),
    Boolean(bool),
    CumulativeValue(u64),
    Alarm { active: bool, reading: Option<f64> },
}

pub struct MqttPublication {
    pub topic: String,
    pub qos: u8,
    pub retain: bool,
    pub payload: Box<serde_json::value::RawValue>,
}
```

Tests reject non-finite numbers, non-canonical UUIDs, sequence zero, wildcard
topics, QoS other than one, unknown schema versions, duplicate modes, and
non-deterministic output.

Run:

```bash
cargo test -p iotkit-output-adapter-api
cargo test -p iotkit-output-adapter-testkit
```

Expected: FAIL because packages and types are absent.

- [ ] **Step 2: Implement the leaf API and testkit**

Define separate traits:

```rust
pub trait OutputAdapter: Send + Sync {
    fn descriptor(&self) -> &'static Descriptor;
    fn validate_config(
        &self,
        config: &serde_json::value::RawValue,
        kind: ObservationKind,
    ) -> Result<(), AdapterError>;
    fn transform(
        &self,
        config: &serde_json::value::RawValue,
        observation: &Observation,
    ) -> Result<MqttPublication, AdapterError>;
}

pub trait ProfilePolicy: Send + Sync {
    fn setup(&self) -> &'static ProfileSetup;
    fn propose(
        &self,
        request: &ProfileRequest<'_>,
    ) -> Result<Vec<RouteProposal>, AdapterError>;
}
```

The API has no Tokio, SQLx, rumqttc, Axum, filesystem, environment, thread, or
clock dependency.

- [ ] **Step 3: Port generic and Pinikiet fixtures test-first**

For every existing Go fixture, load the same bytes and compare exact topic,
QoS, retain, and JSON payload. Add an adapter-specific closed Serde config with
`deny_unknown_fields`.

Run:

```bash
cargo test -p iotkit-output-adapter-generic-mqtt-json-v1
cargo test -p iotkit-output-adapter-pinikiet-mqtt-v1
```

Expected: PASS for all shared fixtures.

- [ ] **Step 4: Add the compile-tested example and one static registry**

The example supports one numeric mode, invokes the testkit, and is not in the
production registry. Registry tests reject duplicate IDs and invalid
descriptors and assert the two production IDs exactly.

Run:

```bash
cargo test -p iotkit-output-adapter-example
cargo test -p iotkit-edge output_registry
scripts/check-layers
```

Expected: PASS and no provider-ID branch outside the registry/adapter packages.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(edge): add Rust Output Adapter API and built-ins"
```

### Task 3: Implement fresh SQLite and PostgreSQL storage

**Files:**
- Modify: `edge/Cargo.toml`
- Create: `edge/migrations/sqlite/0001_baseline.sql`
- Create: `edge/migrations/postgres/0001_baseline.sql`
- Create: `edge/src/storage/mod.rs`
- Create: `edge/src/storage/error.rs`
- Create: `edge/src/storage/model.rs`
- Create: `edge/src/storage/sqlite/mod.rs`
- Create: `edge/src/storage/postgres/mod.rs`
- Create: `edge/tests/storage_contract.rs`
- Create: `edge/tests/storage_faults.rs`
- Create: `edge/tests/fixtures/storage_vectors.json`

**Interfaces:**
- Produces: operation-level `Store` enum delegating to backend-specific
  implementations.
- Produces: `accept_batch`, activation operations, account/session operations,
  semantic/outbox operations, history, audit, diagnostics, and backup metadata.
- Produces: closed `StoreError` variants shared by both profiles.
- Consumes: final logical behavior of `edge/internal/store` without importing
  its SQL or legacy migrations.

- [ ] **Step 1: Write the shared failing store contract**

Define vectors for fresh identity, duplicate/gap/conflict record batches,
activation, mutation plus audit, semantic plus outbox, session revoke, bounded
history, and outbox claim/PUBACK mark. Run each vector against SQLite and
PostgreSQL.

Run:

```bash
cargo test -p iotkit-edge --test storage_contract
```

Expected: FAIL because no Rust Store exists.

- [ ] **Step 2: Implement SQLite operations**

Use a one-connection SQLx pool and set WAL, `synchronous=FULL`, foreign keys,
and busy timeout on connection creation. Each facade method owns its
transaction. Unique constraints enforce record and output identity.

Run:

```bash
cargo test -p iotkit-edge --test storage_contract sqlite
```

Expected: PASS for SQLite vectors.

- [ ] **Step 3: Implement PostgreSQL operations independently**

Use backend-native SQL, row locks for cursor/outbox mutation, and a singleton
deployment lock. Do not translate SQLite placeholders or functions.

Run:

```bash
IOTKIT_TEST_POSTGRES_DSN_FILE="$PWD/target/test-postgres.json" \
  cargo test -p iotkit-edge --test storage_contract postgres
```

Expected: PASS for the same logical vectors.

- [ ] **Step 4: Add commit fault and fresh-schema rejection tests**

Inject failure before commit and after statement execution for raw/cursor,
semantic/outbox, and mutation/audit. Assert no partial state. Present a Go
schema to the Rust opener and assert an actionable refusal without mutation.

Run:

```bash
cargo test -p iotkit-edge --test storage_faults
```

Expected: PASS for both profiles.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(edge): implement Rust SQLite and PostgreSQL stores"
```

### Task 4: Implement Edge Node MQTT custody and activation

**Files:**
- Create: `edge/src/mqtt/mod.rs`
- Create: `edge/src/mqtt/ingest/mod.rs`
- Create: `edge/src/mqtt/ingest/contract.rs`
- Create: `edge/src/mqtt/ingest/processor.rs`
- Create: `edge/src/mqtt/ingest/runtime.rs`
- Create: `edge/src/mqtt/tls.rs`
- Create: `edge/tests/mqtt_custody.rs`
- Create: `edge/tests/mqtt_activation.rs`
- Modify: `scripts/test-edge-bootstrap.sh`
- Modify: `scripts/test-edge-resilience.sh`

**Interfaces:**
- Produces: strict descriptor, activation, record-batch, and acknowledgement
  Serde types matching existing MQTT fixtures.
- Produces: `IngestProcessor::handle(topic, payload)` that commits before
  returning an optional acknowledgement.
- Produces: supervised rumqttc EventLoop runtime with reconnect/resubscribe.
- Consumes: Task 3 Store and existing Edge Node contract fixtures.

- [ ] **Step 1: Write failing fixture and custody state tests**

Cover discovery, pending activation, completed activation, unregistered record
rejection, contiguous commit, exact duplicate, conflicting replay, gap,
storage failure, restart, and acknowledgement topic/payload.

Run:

```bash
cargo test -p iotkit-edge --test mqtt_custody --test mqtt_activation
```

Expected: FAIL before MQTT modules exist.

- [ ] **Step 2: Implement strict contract decoding and processor**

Reject unknown fields and invalid topic identity. Commit raw records and cursor
in one Store operation. Return `accepted-through` only after success.

- [ ] **Step 3: Implement supervised rumqttc runtime**

Use a dedicated client ID/EventLoop, exact subscriptions, reconnect
resubscription, bounded input, and separate typed custody acknowledgement.

- [ ] **Step 4: Run actual Broker and restart gates**

```bash
scripts/test-edge-bootstrap.sh
scripts/test-edge-resilience.sh
```

Expected: Rust target passes discovery, activation, raw custody, restart, and
outage convergence without using the Go process.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(edge): implement Rust MQTT custody and activation"
```

### Task 5: Implement accounts, sessions, authorization, CSRF, and audit

**Files:**
- Create: `edge/src/auth/mod.rs`
- Create: `edge/src/auth/password.rs`
- Create: `edge/src/auth/session.rs`
- Create: `edge/src/auth/csrf.rs`
- Create: `edge/src/auth/principal.rs`
- Create: `edge/src/application/accounts.rs`
- Create: `edge/src/application/authorization.rs`
- Create: `edge/tests/auth_contract.rs`
- Create: `edge/tests/session_contract.rs`
- Create: `edge/tests/secret_safety.rs`

**Interfaces:**
- Produces: role/principal types, Argon2 password hashing, opaque DB-backed
  sessions, CSRF/origin validation, account operations, and audit actor data.
- Consumes: Task 3 account/session/audit Store operations.

- [ ] **Step 1: Port failing auth/session security vectors**

Assert password policy, invalid password delay behavior, role matrix, cookie
name/HttpOnly/Secure/SameSite/Path/expiry, rotation, revoke, password change,
account disable, CSRF, origin, and session invalidation.

Run:

```bash
cargo test -p iotkit-edge --test auth_contract --test session_contract
```

Expected: FAIL because the auth modules are absent.

- [ ] **Step 2: Implement password and session behavior**

Keep plaintext inside secret wrappers, never derive `Debug`, and write only
hashes/session digests. Do not adopt framework defaults that differ from
fixtures.

- [ ] **Step 3: Implement authorization and audit coupling**

Every configuration mutation uses an application operation that performs
authorization before a Store transaction containing state and audit event.

- [ ] **Step 4: Run secret safety checks and commit**

```bash
cargo test -p iotkit-edge --test secret_safety
rg -n 'password|token|private_key' edge/src | scripts/check-secret-log-usage
git commit -m "feat(edge): implement Rust account and session security"
```

### Task 6: Implement semantics, Output Adapter policy, and durable delivery

**Files:**
- Create: `edge/src/semantics/mod.rs`
- Create: `edge/src/semantics/calibration.rs`
- Create: `edge/src/semantics/evaluator.rs`
- Create: `edge/src/semantics/preview.rs`
- Create: `edge/src/application/semantics.rs`
- Create: `edge/src/application/output_profiles.rs`
- Create: `edge/src/mqtt/output/mod.rs`
- Create: `edge/src/mqtt/output/runtime.rs`
- Create: `edge/tests/semantic_contract.rs`
- Create: `edge/tests/output_contract.rs`
- Create: `edge/tests/output_puback.rs`
- Modify: `scripts/test-edge-output.sh`

**Interfaces:**
- Produces: current numeric/boolean state, calibration, transition/debounce,
  cumulative values, alarm state, preview, future-only rule revisions.
- Produces: provider-neutral profile/binding persistence driven by Adapter
  `ProfilePolicy`.
- Produces: one-in-flight outbox runtime that marks delivery only on PUBACK.
- Consumes: Tasks 2 and 3.

- [ ] **Step 1: Port failing semantic and Adapter fixtures**

Run current calibration, threshold, rise/fall, debounce, counter reset,
future-only, generic output, and Pinikiet cases against Rust.

```bash
cargo test -p iotkit-edge --test semantic_contract --test output_contract
```

Expected: FAIL before implementation.

- [ ] **Step 2: Implement current v3 semantics only**

Do not port deprecated internal Go semantic implementations. Preserve public
deprecated endpoint responses later, but use one canonical evaluator.

- [ ] **Step 3: Implement generic profile expansion**

Store and Console know generic setup fields, confirmation state, bindings,
routes, and Adapter descriptors. Assert with source scan tests that Pinikiet
IDs do not appear outside its crate, fixtures, registry, and documentation.

- [ ] **Step 4: Implement PUBACK-driven outbox**

Test enqueue-before-send, Broker outage, reconnect, publish/PUBACK crash,
PUBACK/DB-mark crash, duplicate PUBACK, and restart. Queueing to rumqttc is not
delivery.

- [ ] **Step 5: Run real output gate and commit**

```bash
scripts/test-edge-output.sh
git commit -m "feat(edge): implement Rust semantics and durable output"
```

### Task 7: Implement Axum API and existing Console

**Files:**
- Create: `edge/src/web/mod.rs`
- Create: `edge/src/web/router.rs`
- Create: `edge/src/web/error.rs`
- Create: `edge/src/web/api/`
- Create: `edge/src/web/console/`
- Create: `edge/src/web/templates/`
- Create: `edge/tests/http_contract.rs`
- Create: `edge/tests/console_contract.rs`
- Create: `edge/tests/history_contract.rs`
- Modify: `edge/frontend/e2e/console-journey.mjs`
- Modify: `scripts/test-edge-console-e2e.sh`
- Modify: `scripts/test-edge-console-frontend.sh`

**Interfaces:**
- Produces: Axum router for every route in `edge/openapi/edge-console-v1.yaml`
  and every existing SSR/form/static route.
- Produces: Askama typed view models preserving significant DOM hooks.
- Consumes: Tasks 3, 5, and 6 application operations.

- [ ] **Step 1: Write failing route inventory and HTTP parity tests**

Compare OpenAPI path/method pairs, non-OpenAPI form/static routes, status,
redirect, JSON field/error code, security header, cookie, body limit, and CSV
escaping against the Go oracle.

Run:

```bash
cargo test -p iotkit-edge --test http_contract
```

Expected: FAIL before the router exists.

- [ ] **Step 2: Implement API handlers over application operations**

Handlers parse bounded input, authenticate, authorize, invoke one operation,
and map typed output. No SQLx import is permitted under `web/`.

- [ ] **Step 3: Reuse Console assets and implement typed SSR**

Preserve current TypeScript, CSS, SVG, navigation, URLs, form fields, and
significant selectors. Askama autoescape remains enabled.

- [ ] **Step 4: Implement bounded history and CSV**

Preserve raw/semantic distinction, pagination, aggregation, filtering, units,
escaping, and content-disposition behavior.

- [ ] **Step 5: Run frontend and browser journeys and commit**

```bash
scripts/test-edge-console-frontend.sh
scripts/test-edge-console-e2e.sh
git commit -m "feat(edge): serve the existing Console from Axum"
```

### Task 8: Implement backup, restore, diagnostics, capacity, and CLI

**Files:**
- Create: `edge/src/backup/mod.rs`
- Create: `edge/src/backup/sqlite.rs`
- Create: `edge/src/backup/postgres.rs`
- Create: `edge/src/backup/crypto.rs`
- Create: `edge/src/diagnostics/mod.rs`
- Create: `edge/src/cli/mod.rs`
- Create: `edge/src/cli/commands/`
- Create: `edge/tests/backup_contract.rs`
- Create: `edge/tests/diagnostics_contract.rs`
- Create: `edge/tests/cli_contract.rs`
- Modify: `scripts/test-edge-postgres.sh`
- Modify: `scripts/test-edge-capacity.sh`

**Interfaces:**
- Produces: Rust-created encrypted backup and restore with private modes,
  integrity checks, identity/cursor manifest, and session revoke.
- Produces: current storage/queue/certificate/diagnostic views.
- Produces: every existing local CLI journey with compatible flags/output.
- Consumes: Task 3 Store and application operations.

- [ ] **Step 1: Write failing backup and CLI parity matrices**

Cover create/restore, wrong passphrase, corruption, existing output refusal,
capacity shortage, profile mismatch, restore-to-live refusal, session revoke,
SQLite/PostgreSQL migration, diagnostics, bootstrap/recover, and JSON output.

Run:

```bash
cargo test -p iotkit-edge --test backup_contract \
  --test diagnostics_contract --test cli_contract
```

Expected: FAIL before implementations exist.

- [ ] **Step 2: Implement backend-specific consistent snapshots**

Use SQLite online backup/snapshot semantics and PostgreSQL tools through
bounded blocking workers. Never expose DSN/password in argv, log, or audit.

- [ ] **Step 3: Implement encryption and safe artifact publication**

Derive with Argon2id, encrypt/authenticate with XChaCha20-Poly1305, write to a
private temporary file, fsync, and atomically publish without overwriting.

- [ ] **Step 4: Implement diagnostics, capacity, and Clap CLI**

CLI parsing remains thin and calls application operations. Preserve stdout,
stderr, JSON fields, and exit status fixtures.

- [ ] **Step 5: Run PostgreSQL/capacity gates and commit**

```bash
scripts/test-edge-postgres.sh
scripts/test-edge-capacity.sh "$PWD/target/rust-edge-capacity"
git commit -m "feat(edge): implement Rust operations and recovery"
```

### Task 9: Switch deployment, CI, and documentation to Rust

**Files:**
- Modify: `edge/Dockerfile`
- Create: `.dockerignore`
- Modify: `compose.dev.yaml`
- Modify: `deploy/compose.edge.yaml`
- Modify: `deploy/compose.edge-postgres.yaml`
- Modify: `.github/workflows/ci.yml`
- Modify: `scripts/select-ci-jobs.mjs`
- Modify: `scripts/tests/select-ci-jobs.test.mjs`
- Modify: `scripts/verify.sh`
- Modify: `scripts/test-edge-*.sh`
- Modify: `README.md`
- Modify: `README.ja.md`
- Modify: `CONTRIBUTING.md`
- Modify: `CONTRIBUTING.ja.md`
- Modify: `AGENTS.md`
- Modify: `docs/okf/en/architecture/system-overview.md`
- Modify: `docs/okf/ja/architecture/system-overview.md`
- Modify: `docs/okf/en/contracts/output-adapter-v1.md`
- Modify: `docs/okf/ja/contracts/output-adapter-v1.md`
- Modify: `docs/okf/en/operations/installation-and-recovery.md`
- Modify: `docs/okf/ja/operations/installation-and-recovery.md`

**Interfaces:**
- Produces: non-root Rust production image containing registered Adapters,
  frontend assets, PostgreSQL tools, and the compatible `iotkit-edge` CLI.
- Produces: path-aware CI for Adapter API, implementations, Edge unit,
  Console, integration, and release gates.
- Consumes: all Rust implementation tasks.

- [ ] **Step 1: Write failing image/CI/source scan tests**

Assert the image runs the Rust binary, every production Adapter descriptor is
present, CI has no unconditional full Edge job for Input-only changes, and
current docs no longer tell contributors to edit Go.

- [ ] **Step 2: Build the Rust image from root context**

Use `Cargo.lock`, dependency caching, minimal source COPY, a release builder,
and non-root runtime. Preserve Compose command, volumes, health, profiles,
Caddy loopback behavior, and PostgreSQL tools.

- [ ] **Step 3: Route CI and scripts to Rust**

Adapter API changes select API, all implementations, Edge, and E2E. Adapter
implementation changes select that package, registry, and output E2E. Edge
changes select Edge/Console/integration. Keep final workspace verification.

- [ ] **Step 4: Update English/Japanese authority documents**

Document one-language Adapter journey, trusted compile-time model, static
registry, fresh Rust schema, unsupported Go database/backup, focused commands,
and component boundaries. Increment paired OKF revisions.

- [ ] **Step 5: Verify image and commit**

```bash
docker build -f edge/Dockerfile .
docker compose -f deploy/compose.edge.yaml config
node scripts/check-okf-docs.mjs
node --test scripts/tests/select-ci-jobs.test.mjs
git commit -m "build(edge): switch deployment and CI to Rust"
```

### Task 10: Run final parity, remove Go, review, and publish the draft PR

**Files:**
- Delete: `edge/cmd/**/*.go`
- Delete: `edge/internal/**/*.go`
- Delete: `edge/go.mod`
- Delete: `edge/go.sum`
- Modify: `scripts/test-edge-parity.sh`
- Modify: `review/battle-tested/catalog.json`
- Modify: `docs/superpowers/plans/2026-07-24-rust-edge-replacement.md`

**Interfaces:**
- Consumes: every prior task.
- Produces: Rust-only IoTKit Edge and evidence attached to PR #83.

- [ ] **Step 1: Run Go/Rust black-box differential gate before deletion**

```bash
scripts/test-edge-parity.sh all
```

Expected: all MQTT, HTTP, Console, CLI, output, diagnostics, and recovery
scenario groups PASS. Save the report under an ignored `target/` directory and
summarize it in the PR.

- [ ] **Step 2: Run all Rust release gates before deletion**

```bash
scripts/test-edge-bootstrap.sh
scripts/test-edge-console-e2e.sh
scripts/test-edge-output.sh
scripts/test-edge-resilience.sh
scripts/test-mqtt-security.sh
scripts/test-edge-postgres.sh
scripts/test-edge-capacity.sh "$PWD/target/final-capacity"
scripts/test-edge-host-release-gate.sh "$PWD/target/final-release"
```

Expected: PASS.

- [ ] **Step 3: Delete Go and prove no stale dependency remains**

Delete Go source/module files and remove `setup-go`, `go test`, GOCACHE, Go
module paths, and old binary build commands. Do not delete language-neutral
fixtures, TypeScript assets, OpenAPI, or contracts.

Run:

```bash
rg -n 'setup-go|go test|GOCACHE|edge/go\\.mod|cmd/iotkit-edge|internal/edge' \
  .github scripts deploy Dockerfile edge docs README.md README.ja.md
```

Expected: no active Go build/runtime references.

- [ ] **Step 4: Run complete Rust-only verification**

```bash
TMPDIR="$PWD/target/tmp" scripts/verify.sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

Expected: PASS with no Go toolchain invocation.

- [ ] **Step 5: Request independent reviews**

Request separate custody/data-loss, authentication/security, operations and
backup, and Adapter-author/onboarding reviews. Resolve all Critical and
Important findings and rerun the focused plus complete gates.

- [ ] **Step 6: Commit, push, and create the draft PR**

```bash
git commit -m "refactor(edge): complete the Rust replacement"
git push -u origin agent/issue-83-rust-edge
```

Create a Draft PR that closes #83 and contains:

- architecture and scope;
- explicit unsupported Go DB/backup compatibility;
- per-checkpoint commit map;
- parity report;
- release-gate results;
- review findings and resolutions;
- remaining target-hardware capacity limitation.
