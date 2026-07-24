# Console Commissioning Journey Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and validate the first-run Console journey from a retained MQTT descriptor for an unknown Edge Node through activation, raw custody, device setup, sensor setup, and normal monitoring.

**Architecture:** Keep the approved MQTT custody and activation contracts unchanged. Derive a Console-only commissioning projection from existing Edge Node, inventory device, inventory signal, profile, and rule state; server-render one contextual next action instead of introducing a second registration gate or a client-side wizard. Exercise the journey against a real Mosquitto process and Rust IoTKit Edge, then inspect it as an unfamiliar operator through Playwright MCP.

**Tech Stack:** Rust 1.95, Axum, Askama SSR, SQLite/PostgreSQL storage profiles, TypeScript/Vitest, Mosquitto, Chromium/Playwright MCP.

## Global Constraints

- `accepted-through` means durable raw custody only; it never means setup, semantic projection, or external output success.
- Inactive Edge Nodes publish descriptors but no custody records.
- Active Edge Nodes continue raw storage and acknowledgement for unconfigured devices and signals.
- Semantic rules remain future-only; setup never backfills earlier raw records.
- Device and signal setup do not introduce another activation or acknowledgement gate.
- Mutations continue through `edge/src/application/`; Console code does not write SQL.
- Console status uses text and shape in addition to color, keeps the current desktop-first responsive shell, and exposes one primary next action per commissioning state.

---

### Task 1: Encode the commissioning projection

**Files:**
- Create: `edge/src/web/console/commissioning.rs`
- Modify: `edge/src/web/console/mod.rs`
- Modify: `edge/src/web/mod.rs`
- Modify: `edge/src/composition/web.rs`
- Test: `edge/tests/console_contract.rs`
- Test: `edge/tests/web_application_contract.rs`

**Interfaces:**
- Consumes: `ConsoleEdgeNode`, `ConsoleDevice`, `ConsoleSignal`, `EdgeNodeState`, profile revisions, and active semantic rules.
- Produces: `CommissioningView { stage, title, explanation, action_label, action_href, completed_steps, total_steps, pending_edge_nodes, pending_devices, pending_signals }`.

- [ ] **Step 1: Write failing Console contract tests**

Add assertions that the status page contains `data-commissioning-stage="activate-edge-node"`, a link to the discovered Edge Node, and setup counts. Add storage-backed tests proving an active node with an unconfigured device produces `setup-device`, and a configured device with an unconfigured signal produces `setup-sensor`.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
TMPDIR=$PWD/target/test-tmp/issue-98 cargo test -p iotkit-edge \
  --test console_contract --test web_application_contract commissioning
```

Expected: FAIL because the commissioning projection and HTML hooks do not exist.

- [ ] **Step 3: Implement the pure projection**

Create a pure function with the following priority:

```rust
pub fn commissioning_view(
    edge_nodes: &[ConsoleEdgeNode],
    devices: &[ConsoleDevice],
    signals: &[ConsoleSignal],
) -> CommissioningView {
    // recovery_hold > activating > discovered > unconfigured device
    // > unconfigured signal > missing rule > complete
}
```

Use stable values `recovery`, `activation-in-progress`, `activate-edge-node`, `setup-device`, `setup-sensor`, `setup-rule`, and `complete`. Choose the first affected resource as the action URL. A missing semantic rule is setup work, but an inactive Output Adapter is optional and does not keep commissioning incomplete.

- [ ] **Step 4: Attach the projection to `ConsoleView`**

Populate the projection after nodes, devices, and signals are assembled. Do not query storage from the web template or duplicate activation state.

- [ ] **Step 5: Run the focused tests and verify GREEN**

Run the command from Step 2. Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add edge/src/web edge/src/composition/web.rs edge/tests/console_contract.rs edge/tests/web_application_contract.rs
git commit -m "feat(edge): derive console commissioning state"
```

### Task 2: Render one contextual commissioning path

**Files:**
- Modify: `edge/src/web/templates/console.html`
- Modify: `edge/frontend/static/edge.css`
- Modify: `edge/src/web/mod.rs`
- Test: `edge/tests/console_contract.rs`

**Interfaces:**
- Consumes: `ConsoleView.commissioning`.
- Produces: accessible SSR markup with `data-commissioning-stage`, a four-step progress list, factual counts, and one primary action.

- [ ] **Step 1: Add failing rendering assertions**

Assert that status and equipment pages render:

```html
<section class="onboarding" data-commissioning-stage="activate-edge-node">
```

and contain the ordered concepts `収集ノードを登録`, `機器を確認`, `センサーを設定`, `計測を開始`. Verify `recovery` never renders an activation button and viewers receive a read-only explanation.

- [ ] **Step 2: Run the rendering test and verify RED**

```bash
TMPDIR=$PWD/target/test-tmp/issue-98 cargo test -p iotkit-edge \
  --test console_contract commissioning
```

Expected: FAIL because the commissioning section is absent.

- [ ] **Step 3: Render the progress and next action**

Place the commissioning section above health metrics on `/status` and above the node list on `/equipment`. Keep supporting facts subordinate to the next action. Use the existing `.onboarding-*` CSS classes, add explicit current/completed/pending text, and show an icon plus text for every state.

- [ ] **Step 4: Improve resource details without adding a wizard**

On a discovered Edge Node page, display Edge Node ID, ledger epoch, descriptor device count, descriptor sensor count, and the fact that formal history begins after activation. On active resource pages, label unconfigured resources `設定が必要` while still showing received raw values.

- [ ] **Step 5: Verify responsive and accessible rendering**

Run:

```bash
TMPDIR=$PWD/target/test-tmp/issue-98 cargo test -p iotkit-edge --test console_contract
scripts/test-edge-console-frontend.sh
```

Expected: PASS with no generated asset drift.

- [ ] **Step 6: Commit**

```bash
git add edge/src/web/templates/console.html edge/frontend/static/edge.css edge/src/web/mod.rs edge/tests/console_contract.rs
git commit -m "feat(console): guide first-run commissioning"
```

### Task 3: Prove ACK independence from setup

**Files:**
- Modify: `edge/tests/mqtt_activation.rs`
- Modify: `edge/tests/storage_contract.rs`

**Interfaces:**
- Consumes: existing descriptor, activation, record batch, raw store, and `accepted-through` APIs.
- Produces: executable evidence that setup state cannot block or falsely advance custody.

- [ ] **Step 1: Write failing behavioral tests**

Add a test that applies a descriptor with two signals, configures a profile/rule for only one, activates the Edge Node, accepts one contiguous batch containing both signals, and asserts:

```rust
assert_eq!(ack.accepted_through, batch.cursor_end);
assert_eq!(storage.raw_record_count().await?, 2);
assert_eq!(semantic_observation_count, 1);
```

Also assert inactive batches return `EdgeNodeNotActive` and emit no acknowledgement.

- [ ] **Step 2: Run the tests and inspect RED or existing behavior**

```bash
TMPDIR=$PWD/target/test-tmp/issue-98 cargo test -p iotkit-edge \
  --test mqtt_activation --test storage_contract unconfigured
```

If the test already passes, retain it as explicit contract evidence and do not change production custody code. If it fails, fix only the layer violating the approved contract.

- [ ] **Step 3: Verify the complete custody-focused set**

```bash
TMPDIR=$PWD/target/test-tmp/issue-98 cargo test -p iotkit-edge \
  --test mqtt_activation --test mqtt_custody --test semantic_contract
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add edge/tests/mqtt_activation.rs edge/tests/storage_contract.rs
git commit -m "test(edge): lock commissioning custody boundary"
```

### Task 4: Exercise a clean MQTT commissioning journey

**Files:**
- Create: `edge/examples/console_commissioning_fixture.rs`
- Modify: `scripts/test-edge-console-e2e.sh`
- Modify: `edge/frontend/e2e/rust-console-journey.mjs`
- Test: `scripts/test-edge-console-e2e.sh`

**Interfaces:**
- Consumes: real Mosquitto topics, `DescriptorSnapshot`, `ActivationRequest`, `ActivationResult`, `RecordBatch`, and Console forms.
- Produces: a deterministic clean-database browser journey that starts with discovery rather than pre-seeded configured inventory.

- [ ] **Step 1: Add the failing browser journey**

Before loading the existing configured fixture, assert the browser can:

1. See `新しい収集ノードを検出しました`.
2. Open the discovered node and activate it.
3. Observe `登録処理中`.
4. Receive the fixture's matching activation result.
5. See an unconfigured device and sensor.
6. Save device name/location.
7. Open the sensor, observe raw data, and save its basic profile.
8. Create a numeric rule.
9. Return to status and see commissioning complete.

- [ ] **Step 2: Run E2E and verify RED**

```bash
IOTKIT_TEST_STORAGE_PROFILE=embedded scripts/test-edge-console-e2e.sh
```

Expected: FAIL because the MQTT commissioning fixture and new UI path do not exist.

- [ ] **Step 3: Implement the MQTT fixture**

Create an example process that:

- publishes a retained schema-2 descriptor for `edge-node-commissioning`,
- subscribes to its exact activation request topic,
- validates the request and publishes a matching applied result,
- begins publishing contiguous measurement records only after activation,
- waits for exact `accepted-through`,
- contains no product-only shortcut or direct SQL write.

Pass Broker credentials by owner-only file, never argv payload or debug output.

- [ ] **Step 4: Wire the fixture into the E2E harness**

Start it after Mosquitto and IoTKit Edge are ready, retain its PID for cleanup, and run the commissioning journey before the established configured-sensor checks.

- [ ] **Step 5: Verify SQLite and PostgreSQL**

```bash
IOTKIT_TEST_STORAGE_PROFILE=embedded scripts/test-edge-console-e2e.sh
IOTKIT_TEST_STORAGE_PROFILE=postgres scripts/test-edge-console-e2e.sh
```

Expected: both PASS.

- [ ] **Step 6: Commit**

```bash
git add edge/examples/console_commissioning_fixture.rs scripts/test-edge-console-e2e.sh edge/frontend/e2e/rust-console-journey.mjs
git commit -m "test(console): cover clean mqtt commissioning"
```

### Task 5: Review the journey through Playwright MCP and correct friction

**Files:**
- Modify: files identified by observed journey failures under `edge/src/web/`, `edge/frontend/`, and focused tests.
- Create: `review/battle-tested/reports/issue-98-console-commissioning.md`

**Interfaces:**
- Consumes: the running clean commissioning environment.
- Produces: observed operator evidence, fixes for reproducible friction, and a retained report.

- [ ] **Step 1: Start a reviewable clean environment**

Run the E2E fixture in a keep-running mode on a loopback port. Do not use the already-configured demo database.

- [ ] **Step 2: Use Playwright MCP as an unfamiliar admin**

Navigate only from `/status`, without direct URLs. Record every point where the next action, state, terminology, result, or recovery path is unclear. Inspect at desktop notebook width and a narrow window. Use keyboard navigation for activation and profile forms.

- [ ] **Step 3: Write failing tests for each accepted finding**

For every reproducible issue, add the smallest Console contract, TypeScript unit, or browser journey assertion and confirm it fails before changing production code.

- [ ] **Step 4: Implement and re-run the journey**

Apply only changes required by observed findings. Follow Design Guideline principles:

- essential next action appears first in reading order,
- context-specific help sits beside the affected resource,
- state uses text/icon in addition to color,
- loading and activation progress remain visible,
- forms request only data IoTKit cannot derive,
- control targets remain at least 28px on desktop and body-text contrast reaches 4.5:1.

- [ ] **Step 5: Record the review**

Document viewport, role, scenario, observations, changes, unresolved items, and links to tests in the battle-tested report. Do not claim that this internal review replaces #91's non-IT human usability test.

- [ ] **Step 6: Run final verification**

```bash
scripts/test-edge-console-frontend.sh
IOTKIT_TEST_STORAGE_PROFILE=embedded scripts/test-edge-console-e2e.sh
TMPDIR=$PWD/target/test-tmp/issue-98 scripts/verify.sh
node scripts/battle-tested-review.mjs check
```

Expected: all commands PASS.

- [ ] **Step 7: Commit**

```bash
git add edge review/battle-tested scripts
git commit -m "fix(console): refine commissioning from operator review"
```

