# Rust Edge Runtime Composition Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `iotkit-edge serve` own and supervise storage, MQTT ingest, semantic projection, optional MQTT output, and Axum HTTP until signal shutdown or critical failure.

**Architecture:** Parse deployment arguments into secret-redacting typed runtime configuration, construct every fallible dependency before spawning, then run all critical tasks under one cancellation token and owned supervisor. A single `RuntimeFactory::web_application(Storage)` method is the Task 7 adapter connection point; the CLI factory fails closed until it is supplied.

**Tech Stack:** Rust 2024, Tokio, tokio-util, rumqttc, Axum, SQLx, Clap

## Global Constraints

- No detached critical tasks.
- Unexpected clean critical-task exit, error, or panic is a process failure.
- SIGINT and SIGTERM cancel and drain every owned task within a finite deadline.
- Production MQTT uses `ssl://`; `tcp://` requires the explicit insecure flag.
- Passwords and CA bundles are file inputs, and `Debug` never exposes secret bytes.
- The default production factory fails before spawning when no Web adapter exists.
- Provider-specific output behavior remains confined to registered adapters.

---

### Task 1: Typed serve configuration

**Files:**
- Create: `edge/src/composition/runtime_config.rs`
- Modify: `edge/src/composition/mod.rs`
- Modify: `edge/src/cli/mod.rs`
- Test: `edge/tests/runtime_composition.rs`

**Interfaces:**
- Produces: `RuntimeConfig::from_serve_args(&ServeArgs) -> Result<RuntimeConfig, RuntimeConfigError>`
- Produces: `MqttEndpoint`, `MqttConnectionConfig`, `RuntimeConfig`
- Consumes: owner-only secret-file validation and `StorageArgs`

- [ ] **Step 1: Write failing tests**

Add tests that parse the exact deployment flags, reject scheme/trust conflicts
and partial output settings, and assert formatted config omits password and CA
contents.

- [ ] **Step 2: Verify RED**

Run:

```bash
TMPDIR="$PWD/target/tmp" cargo test -p iotkit-edge --test runtime_composition typed_
```

Expected: compile failure because `composition::runtime_config` is absent.

- [ ] **Step 3: Implement minimal typed conversion**

Parse URLs with `url::Url`, enforce explicit ports and closed schemes, map
`system_roots` and `bundle_only` into runtime transport types, read owner-only
files once, parse listen/origin values, and implement manual redacted `Debug`.

- [ ] **Step 4: Verify GREEN**

Run the focused command and `cargo test -p iotkit-edge --test cli_contract`.
Expected: all tests pass.

### Task 2: Owned critical supervisor

**Files:**
- Modify: `edge/src/lifecycle.rs`
- Test: `edge/tests/unit/lifecycle_tests.rs`

**Interfaces:**
- Produces: `Supervisor::with_token(CancellationToken, Duration)`
- Produces: `Supervisor::spawn(&'static str, Future)`
- Produces: `Supervisor::run() -> ExitReason`

- [ ] **Step 1: Write failing lifecycle tests**

Cover unexpected clean exit, error, panic, signal cancellation joining siblings,
and timeout aborting an uncooperative sibling.

- [ ] **Step 2: Verify RED**

Run:

```bash
TMPDIR="$PWD/target/tmp" cargo test -p iotkit-edge --lib lifecycle::tests
```

Expected: assertions fail because clean exit is currently `Requested` and
siblings are not drained.

- [ ] **Step 3: Implement owned cancellation and drain**

Keep every handle in `JoinSet`, cancel on first failure, distinguish requested
cancellation from early clean completion, drain with `tokio::time::timeout`,
and call `abort_all` only after timeout.

- [ ] **Step 4: Verify GREEN**

Run the lifecycle test command. Expected: all tests pass.

### Task 3: Runtime composition and HTTP factory

**Files:**
- Create: `edge/src/composition/runtime.rs`
- Modify: `edge/src/composition/mod.rs`
- Modify: `edge/src/cli/mod.rs`
- Modify: `edge/src/lib.rs`
- Test: `edge/tests/runtime_composition.rs`

**Interfaces:**
- Produces: `RuntimeFactory::web_application(Storage) -> Result<Arc<dyn WebApplication>, RuntimeError>`
- Produces: `run_runtime(RuntimeConfig, &dyn RuntimeFactory, ShutdownSignal) -> Result<ExitReason, RuntimeError>`
- Produces: `ProductionRuntimeFactory`, which returns `WebAdapterUnavailable`

- [ ] **Step 1: Write failing composition tests**

Assert the production factory fails before a task-start counter changes, an
injected Web adapter binds HTTP, and early task completion maps to failure.

- [ ] **Step 2: Verify RED**

Run:

```bash
TMPDIR="$PWD/target/tmp" cargo test -p iotkit-edge --test runtime_composition composition_
```

Expected: compile failure because runtime composition APIs are absent.

- [ ] **Step 3: Implement composition**

Connect storage, obtain the Web adapter before creating the supervisor, build
ingest/output MQTT options from typed config, spawn ingest, semantics, optional
output, and `axum::serve` tasks with cloned cancellation tokens, and register a
SIGINT/SIGTERM future that only cancels the token.

- [ ] **Step 4: Wire CLI**

Replace the empty `Application` path with typed conversion followed by
`run_runtime(..., &ProductionRuntimeFactory, unix_shutdown_signal())`.

- [ ] **Step 5: Verify GREEN**

Run runtime-composition, CLI, HTTP, MQTT, semantic, and output focused tests.
Expected: all pass.

### Task 4: Real composed-runtime gate

**Files:**
- Create: `edge/tests/runtime_composition_broker.rs`
- Create: `scripts/test-rust-edge-runtime.sh`
- Modify: `scripts/select-ci-jobs.mjs`

**Interfaces:**
- Consumes: `run_runtime`, test `RuntimeFactory`, exact deployment flags
- Proves: MQTT input custody to semantic observation to output PUBACK plus HTTP and SIGTERM

- [ ] **Step 1: Write ignored real-broker test and gate script**

The test uses real `IngestRuntime`, semantic loop, `OutputRuntime`, storage,
Axum listener, and a test Web adapter. The script starts Mosquitto, creates
owner-only credentials, passes the compose-equivalent arguments, publishes
activation and a record, and asserts durable raw, observation, and published
outbox state.

- [ ] **Step 2: Verify RED**

Run:

```bash
TMPDIR="$PWD/target/tmp" scripts/test-rust-edge-runtime.sh
```

Expected: failure before the composition implementation is complete.

- [ ] **Step 3: Make the gate green without test-only product branches**

Use factory injection only for the Web adapter. Keep MQTT, storage, projection,
delivery, and supervisor production paths unchanged.

- [ ] **Step 4: Verify SQLite and PostgreSQL**

Run the gate normally and with `IOTKIT_TEST_STORAGE_PROFILE=postgres`.
Expected: both pass when Docker PostgreSQL is available.

### Task 5: Final verification and commit

**Files:**
- Review all changed runtime/config/test/gate files.

- [ ] **Step 1: Run focused quality gates**

```bash
TMPDIR="$PWD/target/tmp" cargo clippy -p iotkit-edge --all-targets -- -D warnings
scripts/check-source-layout
node scripts/battle-tested-review.mjs select --base d9d39e4
```

- [ ] **Step 2: Run repository verification**

```bash
GOCACHE="$PWD/target/go-build" TMPDIR="$PWD/target/tmp" scripts/verify.sh
```

Expected: `verify.sh PASS`.

- [ ] **Step 3: Review and commit**

Check provider leakage, secret formatting, task ownership, cancellation paths,
and the single Web adapter connection point, then commit:

```bash
git commit -m "feat(edge): compose Rust production runtime"
```
