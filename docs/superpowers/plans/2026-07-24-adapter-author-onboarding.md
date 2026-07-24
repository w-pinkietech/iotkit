# Adapter Author Onboarding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Rust Input and Output Adapter authors an exact, tested path from the public API through compile-time registration.

**Architecture:** Promote the existing test-only Input Adapter reference rather than creating another crate. Keep production composition unchanged, document its current central schema and private catalogs precisely, and add a lightweight source guard for normative Output API vocabulary.

**Tech Stack:** Rust 2024, Tokio, Node.js built-in test runner, Markdown/OKF bilingual documents.

## Global Constraints

- Keep Input and Output Adapters as trusted compile-time Rust code.
- Do not add runtime plugin discovery or Console installation.
- Do not redesign Input Adapter configuration; document the current central `RawInputAdapterInstance`.
- Edit paired English and Japanese OKF contracts together and increment their revisions.
- Commit locally and do not push.

---

### Task 1: Production-shaped test-only Input Adapter reference

**Files:**
- Modify: `edge-node/input/testkit/Cargo.toml`
- Modify: `edge-node/input/testkit/src/lib.rs`
- Modify: `edge-node/input/testkit/tests/unit/lib_tests.rs`

**Interfaces:**
- Consumes: `InputAdapterTypeDescriptor`, `AdapterStartContext`, `RunningInputAdapter`, and `runtime_channels` from `iotkit-input-adapter-host-api`.
- Produces: `ReferenceAdapterConfig`, `ReferenceAdapter::descriptor`, `ReferenceAdapter::parse_and_validate`, and `ReferenceAdapter::start`.

- [ ] **Step 1: Write the failing lifecycle test**

Add a Tokio test that validates the descriptor, rejects zero diagnostic capacity, starts the reference with a real `AdapterStartContext`, receives its source-bound envelope, requests shutdown, and observes `AdapterCompletion::RequestedStop`.

- [ ] **Step 2: Run the focused test and verify RED**

Run: `cargo test -p iotkit-input-adapter-testkit reference_adapter_exercises_descriptor_config_start_and_shutdown`

Expected: compile failure because the production-shaped reference API does not exist.

- [ ] **Step 3: Implement the minimal reference API**

Add a typed config containing diagnostic capacity, a stable vendor-neutral descriptor, strict validation, and a start method that creates runtime channels and a Tokio task. The task submits the existing two-subject observations, reports activity, waits for shutdown, and reports requested completion.

- [ ] **Step 4: Run the Input Adapter testkit**

Run: `cargo test -p iotkit-input-adapter-testkit`

Expected: all tests pass.

### Task 2: Rust Output contract and drift guard

**Files:**
- Create: `scripts/tests/adapter-author-docs.test.mjs`
- Modify: `docs/okf/en/contracts/output-adapter-v1.md`
- Modify: `docs/okf/ja/contracts/output-adapter-v1.md`

**Interfaces:**
- Consumes: exact names and signatures from `edge/output-adapters/api/src/lib.rs`.
- Produces: bilingual normative Rust authoring documentation protected from stale Go vocabulary.

- [ ] **Step 1: Write and run the failing source guard**

Require the English contract to contain `pub trait OutputAdapter`, `MqttPublication`, `AdapterError`, and links to `api`, `example`, and `testkit`; reject Go fences, `ModeDescriptor`, `MQTTPublication`, and `ErrInvalidDescriptor`.

Run: `node --test scripts/tests/adapter-author-docs.test.mjs`

Expected: failure against the stale contract.

- [ ] **Step 2: Replace stale Output API sections in both languages**

Use exact Rust declarations for `Mode`, `Descriptor`, `Observation`, `OutputAdapter`, `MqttPublication`, and `AdapterError`; use lowercase trait methods; link the API, example, and testkit; increment both revisions from 3 to 4.

- [ ] **Step 3: Run the source guard**

Run: `node --test scripts/tests/adapter-author-docs.test.mjs`

Expected: pass.

### Task 3: Truthful Input Adapter onboarding

**Files:**
- Modify: `edge-node/adapters/README.md`
- Modify: `edge-node/adapters/README.ja.md`
- Modify: `docs/okf/en/contracts/input-adapter-v1.md`
- Modify: `docs/okf/ja/contracts/input-adapter-v1.md`

**Interfaces:**
- Consumes: current workspace, `iotkit-edge-node` dependency list, central raw config schema, private factory catalog, layer checker, architecture map, and testkit reference.
- Produces: matching English/Japanese checklists and exact commands.

- [ ] **Step 1: Add the exact author checklist to both READMEs**

Name the root workspace, node Cargo dependency, central raw schema, private factory/catalog, layer classification, architecture map, fixtures, and focused test commands.

- [ ] **Step 2: Correct both normative contracts**

Replace the inaccurate “only” integration claim with the same exhaustive compile-time steps, document the central schema edit explicitly, and identify the production-shaped test-only reference lifecycle test. Increment both revisions from 3 to 4.

- [ ] **Step 3: Run bilingual and structural checks**

Run: `OKF_BASE_REF=c2355c4 node scripts/check-okf-docs.mjs`

Run: `scripts/check-layers`

Run: `scripts/check-source-layout`

Expected: all commands pass.

### Task 4: Final verification and commit

**Files:**
- Review all files above.

- [ ] **Step 1: Run adapter-focused verification**

Run:

```bash
node --test scripts/tests/adapter-author-docs.test.mjs
cargo test -p iotkit-input-adapter-testkit
cargo test -p iotkit-output-adapter-example
cargo test -p iotkit-output-adapter-testkit
cargo test -p iotkit-edge --test output_registry
OKF_BASE_REF=c2355c4 node scripts/check-okf-docs.mjs
scripts/check-layers
scripts/check-source-layout
```

Expected: every command exits zero.

- [ ] **Step 2: Review the diff**

Run: `git diff --check && git status --short && git diff --stat`

Expected: no whitespace errors and only scoped files changed.

- [ ] **Step 3: Commit**

Run:

```bash
git add edge-node/input/testkit edge-node/adapters docs/okf scripts/tests/adapter-author-docs.test.mjs
git commit -m "docs: clarify Rust adapter authoring"
```

Expected: one local implementation commit; no push.
