# Rust Console Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make all Console presentation settings, semantic/export previews, and binding delivery inspection durable and production-backed across SQLite and PostgreSQL.

**Architecture:** Migration 0006 adds stable inventory identities and profile tables. Typed application services own authorization and validation; storage owns transactions; semantic and output previews reuse the production evaluator and adapter registry. The web adapter only maps typed results to API and SSR models.

**Tech Stack:** Rust, Tokio, SQLx SQLite/PostgreSQL, Axum, Askama, serde, existing semantics evaluator and Output Adapter API, Node DevTools browser journey, Mosquitto.

## Global Constraints

- No SQL is added under `edge/src/web/`.
- Operational mutations require an active admin or system-admin principal.
- SQLite and PostgreSQL implement the same profile, preview, and delivery contract.
- MQTT PUBACK is the only durable publication-success transition.
- Production CLI validation is not weakened for E2E.
- Browser or diagnostic output never contains Broker or account secrets.

---

### Task 1: Stable inventory and presentation profiles

**Files:**
- Create: `edge/migrations/sqlite/0006_console_profiles.sql`
- Create: `edge/migrations/postgres/0006_console_profiles.sql`
- Create: `edge/src/application/profiles.rs`
- Create: `edge/src/storage/profiles.rs`
- Modify: `edge/src/application/mod.rs`
- Modify: `edge/src/storage/mod.rs`
- Modify: `edge/src/storage/activation.rs`
- Modify: `edge/src/storage/semantic_output/sqlite.rs`
- Modify: `edge/src/storage/semantic_output/postgres.rs`
- Test: `edge/tests/profile_contract.rs`

**Interfaces:**
- Produces: `InventoryProfiles::update_device`, `InventoryProfiles::update_signal`, `Storage::inventory_devices`, `Storage::inventory_signals`.
- Produces: `InventoryDevice`, `InventorySignal`, `DeviceProfileInput`, `SignalProfileInput`, `DeviceProfile`, `SignalProfile`.

- [ ] **Step 1: Write failing SQLite persistence tests**

Create tests that apply a descriptor, assert stable `dev_`/`sig_` refs, update profiles through `InventoryProfiles`, reopen storage, and assert all presentation fields and revisions remain. Assert a stale revision fails and the successful audit rows are present.

- [ ] **Step 2: Verify the profile contract fails**

Run: `cargo test -p iotkit-edge --test profile_contract -- --nocapture`

Expected: compilation fails because `InventoryProfiles` and profile types do not exist.

- [ ] **Step 3: Add migration and typed implementation**

Add the four tables from the approved design. Backfill descriptor identities in both migrations. Extend descriptor application to insert missing stable refs. Implement trimmed bounded validation, revision checks, transactional upsert, and audit insertion for both backends.

- [ ] **Step 4: Reuse stable signal refs in semantics**

Change semantic-signal creation to select `inventory_signals.signal_ref` for `(edge_node_id, series_key)` and reject semantic creation for an unknown descriptor signal.

- [ ] **Step 5: Verify SQLite profiles**

Run: `cargo test -p iotkit-edge --test profile_contract -- --nocapture`

Expected: all SQLite profile tests pass.

### Task 2: Production semantic and export previews

**Files:**
- Modify: `edge/src/application/semantics.rs`
- Modify: `edge/src/application/output_profiles.rs`
- Modify: `edge/src/storage/semantic_output/operations.rs`
- Modify: `edge/src/storage/semantic_output/common.rs`
- Modify: `edge/src/storage/semantic_output/sqlite.rs`
- Modify: `edge/src/storage/semantic_output/postgres.rs`
- Test: `edge/tests/preview_contract.rs`

**Interfaces:**
- Produces: `Semantics::preview(MappingPreviewRequest) -> MappingPreviewResponse`.
- Produces: `OutputProfiles::preview_activation(&str) -> ExportProfileActivationPreview`.
- Produces: `OutputProfiles::publication(&str, i64) -> OutputPublicationPreview`.

- [ ] **Step 1: Write failing semantic preview tests**

Seed bounded scalar raw history and assert numeric, counter, and alarm drafts call the real evaluator, produce independent rule results, preserve window bounds, and do not write observations or profile rows.

- [ ] **Step 2: Verify semantic preview tests fail**

Run: `cargo test -p iotkit-edge --test preview_contract semantic -- --nocapture`

Expected: compilation fails because the preview request/response API is absent.

- [ ] **Step 3: Implement evaluator-backed preview**

Resolve stable signals, decode only finite one-value measurement records, cap the input window, load saved calibration/rules when no draft is supplied, and call `build_preview` once per rule.

- [ ] **Step 4: Write failing output preview and delivery tests**

Assert adapter classification for numeric/boolean/counter/alarm rules. Assert publication precedence `actual`, `latest_observation`, `sample`; exact adapter-produced topic/payload; and delivery states before and after `mark_output_published`.

- [ ] **Step 5: Verify output preview tests fail**

Run: `cargo test -p iotkit-edge --test preview_contract output -- --nocapture`

Expected: compilation fails because output preview and durable delivery APIs are absent.

- [ ] **Step 6: Implement adapter-policy and publication preview**

Query typed rule/profile/route facts in storage, invoke registered `ProfilePolicy` compatibility and `OutputAdapter::transform`, aggregate durable outbox rows, and derive the five-minute stall state.

- [ ] **Step 7: Verify preview contracts**

Run: `cargo test -p iotkit-edge --test preview_contract -- --nocapture`

Expected: semantic and output preview tests pass.

### Task 3: Web/API/SSR integration and reload behavior

**Files:**
- Modify: `edge/src/composition/web.rs`
- Modify: `edge/src/web/mod.rs`
- Modify: `edge/src/web/templates/console.html`
- Modify: `edge/tests/web_application_contract.rs`
- Modify: `edge/tests/http_contract.rs`
- Test: `edge/tests/profile_contract.rs`

**Interfaces:**
- Consumes: `InventoryProfiles`, `Semantics::preview`, `OutputProfiles` preview/publication methods.
- Produces: durable device/signal profile mutation responses, OpenAPI mapping-preview JSON, exact binding publication JSON, and SSR delivery labels.

- [ ] **Step 1: Write failing production-web tests**

Exercise profile PUT/form mutations through `StorageWebApplication`, recreate the application from the same database, and assert `/api/v1/devices`, `/api/v1/signals`, Console pages, history labels, activation preview, mapping preview, and binding publication reflect saved state.

- [ ] **Step 2: Verify web tests fail**

Run: `cargo test -p iotkit-edge --test web_application_contract -- --nocapture`

Expected: assertions fail because profile mutations are no-ops and previews are simplified.

- [ ] **Step 3: Replace compatibility branches with typed calls**

Parse form/JSON fields and revisions, call typed operations, map field errors, and remove no-op/synthetic preview responses. Populate Console output binding labels from durable delivery facts.

- [ ] **Step 4: Verify focused HTTP and web contracts**

Run: `cargo test -p iotkit-edge --test web_application_contract --test http_contract --test profile_contract --test preview_contract`

Expected: all focused contracts pass.

### Task 4: PostgreSQL parity

**Files:**
- Modify: `edge/tests/profile_contract.rs`
- Modify: `edge/tests/preview_contract.rs`
- Modify: `scripts/test-edge-postgres.sh`

**Interfaces:**
- Consumes: `IOTKIT_TEST_POSTGRES_DSN`.
- Produces: the same contract assertions for PostgreSQL as SQLite.

- [ ] **Step 1: Add PostgreSQL-gated contract cases**

Run the shared profile and preview scenarios against a PostgreSQL `Storage`, including reopen, stale revision, audit, actual/published outbox state, and migration backfill.

- [ ] **Step 2: Verify with a real PostgreSQL service when available**

Run: `test -n "$IOTKIT_TEST_POSTGRES_DSN" && cargo test -p iotkit-edge --test profile_contract --test preview_contract -- --nocapture`

Expected: SQLite and PostgreSQL cases pass; without the variable PostgreSQL cases are explicitly ignored.

### Task 5: Actual production-runtime browser journey

**Files:**
- Modify: `edge/examples/console_fixture.rs`
- Modify: `edge/frontend/e2e/rust-console-journey.mjs`
- Modify: `scripts/test-edge-console-e2e.sh`
- Modify: `edge/frontend/e2e/chromium-launch.test.mjs`

**Interfaces:**
- Produces: an authenticated ephemeral Mosquitto runtime and browser assertions across save and reload.

- [ ] **Step 1: Add failing script/source assertions**

Assert the script provisions a Mosquitto password file with mode 0600 and invokes `serve` with edge identity, authenticated Broker URL, username, password file, insecure development transport, and development HTTP.

- [ ] **Step 2: Verify the existing E2E fails at runtime startup**

Run: `scripts/test-edge-console-e2e.sh`

Expected: current production `serve` exits because required runtime composition arguments are missing.

- [ ] **Step 3: Start ephemeral Mosquitto without changing CLI**

Prefer an installed `mosquitto`; otherwise use the repository’s pinned container path when Docker is available. Generate a test-only password database/config under the E2E directory and stop it in the cleanup trap.

- [ ] **Step 4: Extend save/reload browser assertions**

Save device and signal profiles, reload and verify display values; request semantic and export previews; inspect exact binding publication/delivery state; mutate semantic/output state and verify after another reload. Keep Chrome candidate fallback and per-candidate diagnostics.

- [ ] **Step 5: Verify actual Rust binary E2E**

Run: `scripts/test-edge-console-e2e.sh`

Expected: `Rust IoTKit Console browser journey passed`.

### Task 6: Review and completion

**Files:**
- Modify only files required by review findings.

- [ ] **Step 1: Run focused frontend and backend gates**

Run: `scripts/test-edge-console-frontend.sh && scripts/test-edge-console-e2e.sh`

Expected: both pass.

- [ ] **Step 2: Run architecture and battle-tested review**

Run: `scripts/check-layers && scripts/check-source-layout && node scripts/battle-tested-review.mjs select --base origin/master`

Expected: structure checks pass; inspect selected BT-001, BT-002, BT-003, and BT-005 concerns.

- [ ] **Step 3: Run full verification**

Run: `TMPDIR="$HOME/.cache/iotkit-codex-tmp" scripts/verify.sh`

Expected: workspace tests, Clippy with warnings denied, and Go tests pass.

- [ ] **Step 4: Commit implementation**

Run: `git add edge docs/superpowers scripts && git commit -m "fix(edge): complete durable Console operations"`

Expected: clean worktree and a commit SHA ready for parent integration.
