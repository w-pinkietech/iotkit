# Console Output Delivery Status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild `/output` around trustworthy delivery-state inspection while preserving the existing add, configure, start, and stop operation contracts.

**Architecture:** Extend the server-side Console read-model with pure presentation classification in `web::console::output`, then compose profile, semantic-rule, publication-preview, and durable outbox facts in `StorageWebApplication`. Askama renders deterministic status-first cards; the existing Console shell supplies timestamp localization and copy behavior, and output-specific CSS provides the narrow layout.

**Tech Stack:** Rust 1.95.0, Tokio, SQLx-backed application/storage APIs, Askama, TypeScript-generated Console JavaScript, CSS, Node.js browser journeys over Chromium DevTools Protocol.

## Global Constraints

- Keep `/output` and every existing add, configure, start, and stop route unchanged.
- Keep Broker endpoint, credential, CA, private key, and ACL configuration out of Console.
- Treat a pending delivery younger than `300_000` milliseconds as neutral `配送中`.
- Treat a pending delivery at least `300_000` milliseconds old as `配送停止の可能性`.
- Apply destination priority in this order: configuration/transform error, delivery stall, external-registration wait, draining, sending.
- Hide stopped profiles from the normal page and permit the same Adapter to appear in the add section again.
- Show viewer users the same state and technical facts, but render no mutation forms for them.
- Keep topic, payload, and identifiers inside read-only `<details>` disclosure.
- At 390 CSS pixels, require `document.documentElement.scrollWidth <= document.documentElement.clientWidth`.
- Do not expose secrets or raw internal error text in the Console read-model.
- Preserve the Output Adapter, durable outbox, typed application operation, authorization, CSRF, revision, and audit boundaries.

---

### Task 1: Pure Console delivery presentation model

**Files:**
- Create: `edge/src/web/console/output.rs`
- Modify: `edge/src/web/console/mod.rs`
- Modify: `edge/src/web/mod.rs:151-266`
- Create: `edge/tests/output_console_presentation.rs`

**Interfaces:**
- Consumes: primitive profile and binding state facts already available to the web boundary.
- Produces:
  - `ConsoleOutputSummary { sending_count, needs_configuration_count, delivery_problem_count }`
  - `ConsoleBindingState { label, class_name, target, needs_configuration, delivery_problem, waiting_registration }`
  - `binding_state(active, needs_configuration, ineligible, prepared, delivery_state, preview_failed) -> ConsoleBindingState`
  - private `state(label, class_name, target, needs_configuration, delivery_problem, waiting_registration) -> ConsoleBindingState`
  - `apply_destination_state(output: &mut ConsoleOutput)`
  - `summarize(outputs: &[ConsoleOutput]) -> ConsoleOutputSummary`

- [ ] **Step 1: Write failing presentation tests**

Create `edge/tests/output_console_presentation.rs` with table-driven assertions equivalent to:

```rust
use iotkit_edge::web::{
    ConsoleBinding, ConsoleOutput,
    console::output::{apply_destination_state, binding_state, summarize},
};

#[test]
fn binding_priority_keeps_configuration_and_stalls_actionable() {
    let configuration = binding_state(false, true, false, false, None, false);
    assert_eq!(configuration.label, "設定が必要");
    assert!(configuration.needs_configuration);

    let stalled = binding_state(
        true,
        false,
        false,
        false,
        Some("possible_delivery_stall"),
        false,
    );
    assert_eq!(stalled.label, "配送停止の可能性");
    assert!(stalled.delivery_problem);

    let delivering = binding_state(
        true,
        false,
        false,
        false,
        Some("delivering"),
        false,
    );
    assert_eq!(delivering.label, "配送中");
    assert!(!delivering.delivery_problem);
}

#[test]
fn destination_summary_counts_each_live_destination_once() {
    let mut output = ConsoleOutput {
        active: true,
        bindings: vec![ConsoleBinding {
            needs_configuration: true,
            ..ConsoleBinding::default()
        }],
        ..ConsoleOutput::default()
    };
    apply_destination_state(&mut output);
    let summary = summarize(&[output]);
    assert_eq!(summary.needs_configuration_count, 1);
    assert_eq!(summary.sending_count, 0);
    assert_eq!(summary.delivery_problem_count, 0);
}
```

Also cover preview failure over delivery state, external-registration wait in the configuration bucket, stalled delivery over registration wait, draining without an issue in the sending bucket, and inactive Adapter cards excluded from all three counts.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test -p iotkit-edge --test output_console_presentation
```

Expected: compilation fails because `web::console::output`, the new fields, and `Default` implementations do not exist.

- [ ] **Step 3: Add the presentation types and classification**

In `edge/src/web/console/output.rs`, define the exact public functions from the Interfaces block. `binding_state` must use this decision order:

```rust
if preview_failed {
    state("変換エラー", "error", true, true, false, false)
} else if needs_configuration {
    state("設定が必要", "needs-action", false, true, false, false)
} else if delivery_state == Some("possible_delivery_stall") {
    state("配送停止の可能性", "error", true, false, true, false)
} else if prepared {
    state("外部登録待ち", "needs-action", true, true, false, true)
} else if ineligible {
    state("対象外", "muted", false, false, false, false)
} else if active && delivery_state == Some("delivering") {
    state("配送中", "delivering", true, false, false, false)
} else if active && delivery_state == Some("published") {
    state("正常に送信中", "healthy", true, false, false, false)
} else if active {
    state("最初の値を待っています", "waiting", true, false, false, false)
} else {
    state("開始待ち", "waiting", false, false, false, false)
}
```

`apply_destination_state` must set `status_label`, `status_class`, and the mutually exclusive `needs_configuration` / `delivery_problem` flags. It must sum `target_count`, `pending_count`, choose the minimum non-`None` `oldest_pending_at`, and choose the maximum non-`None` `last_published_at`.

Extend the view types in `edge/src/web/mod.rs` with these exact fields:

```rust
#[derive(Clone, Debug, Default)]
pub struct ConsoleOutputSummary {
    pub sending_count: usize,
    pub needs_configuration_count: usize,
    pub delivery_problem_count: usize,
}

// Add to ConsoleView:
pub output_summary: ConsoleOutputSummary,

#[derive(Clone, Debug, Default)]
pub struct ConsoleOutput {
    pub profile_id: String,
    pub adapter_id: String,
    pub display_name: String,
    pub adapter_name: String,
    pub description: String,
    pub active: bool,
    pub draining: bool,
    pub future_rules_enabled: bool,
    pub status_label: String,
    pub status_class: String,
    pub needs_configuration: bool,
    pub delivery_problem: bool,
    pub target_count: usize,
    pub pending_count: i64,
    pub oldest_pending_at: Option<i64>,
    pub last_published_at: Option<i64>,
    pub automatic_rule_count: usize,
    pub configuration_rule_count: usize,
    pub ineligible_rule_count: usize,
    pub bindings: Vec<ConsoleBinding>,
}

#[derive(Clone, Debug, Default)]
pub struct ConsoleBinding {
    pub binding_id: String,
    pub rule_id: String,
    pub signal_ref: String,
    pub edge_node_id: String,
    pub series_id: String,
    pub sensor_name: String,
    pub rule_name: String,
    pub state_label: String,
    pub state_class: String,
    pub prepared: bool,
    pub target: bool,
    pub needs_configuration: bool,
    pub delivery_problem: bool,
    pub waiting_registration: bool,
    pub pending_count: i64,
    pub oldest_pending_at: Option<i64>,
    pub last_published_at: Option<i64>,
    pub topic: String,
    pub payload: String,
    pub provenance_label: String,
    pub technical_error: String,
}
```

Derive `Default` for these structures so fixture construction stays explicit only where a fact matters.

- [ ] **Step 4: Export the module**

Add this line to `edge/src/web/console/mod.rs`:

```rust
pub mod output;
```

Keep the existing `commissioning` export unchanged.

- [ ] **Step 5: Run the focused test and verify GREEN**

Run:

```bash
cargo test -p iotkit-edge --test output_console_presentation
```

Expected: every classification, aggregation, and mutually-exclusive summary assertion passes.

- [ ] **Step 6: Run source-boundary and formatting checks**

Run:

```bash
python3 scripts/check-source-layout
cargo fmt --all --check
```

Expected: the external integration test satisfies the source/test boundary and formatting is clean.

- [ ] **Step 7: Commit the pure presentation model**

```bash
git add edge/src/web/console/output.rs edge/src/web/console/mod.rs edge/src/web/mod.rs edge/tests/output_console_presentation.rs
git commit -m "feat(console): model output delivery status"
```

### Task 2: Compose delivery facts into the production Console read-model

**Files:**
- Modify: `edge/src/composition/web.rs:331-393`
- Modify: `edge/src/web/mod.rs:1550-2040`
- Create: `edge/tests/output_console_read_model.rs`

**Interfaces:**
- Consumes:
  - `OutputProfiles::list() -> Vec<ExportProfile>`
  - `OutputProfiles::preview_activation(adapter_id)`
  - `OutputProfiles::publication(binding_id, now)`
  - `Storage::list_semantic_rules()`
  - Task 1 `binding_state`, `apply_destination_state`, and `summarize`
- Produces:
  - `StorageWebApplication::console_outputs() -> Result<(ConsoleOutputSummary, Vec<ConsoleOutput>), WebError>`
  - complete `ConsoleBinding` technical facts: `rule_id`, `signal_ref`, `edge_node_id`, `series_id`, `topic`, pretty JSON `payload`, and provenance label.

- [ ] **Step 1: Write the failing production read-model test**

Create a SQLite fixture in `edge/tests/output_console_read_model.rs` that:

1. creates an initial system administrator;
2. applies `testdata/egress/v2/descriptor-snapshot.json`;
3. activates the Edge Node;
4. creates one numeric semantic rule;
5. activates `iotkit.mqtt-json.v1`;
6. requests `StorageWebApplication::console()` for `/output`.

Assert:

```rust
assert_eq!(view.output_summary.sending_count, 1);
assert_eq!(view.output_summary.needs_configuration_count, 0);
assert_eq!(view.output_summary.delivery_problem_count, 0);
assert_eq!(view.outputs.iter().filter(|item| item.active).count(), 1);

let binding = &view.outputs.iter().find(|item| item.active).unwrap().bindings[0];
assert_eq!(binding.rule_name, "Temperature");
assert_eq!(binding.state_label, "最初の値を待っています");
assert!(binding.topic.starts_with("iotkit/v1/sources/"));
assert!(binding.payload.contains("\"schema_version\""));
assert!(!binding.signal_ref.is_empty());
assert!(!binding.series_id.is_empty());
```

Stop the profile, request `/output` again, and assert the stopped profile is absent while the same Adapter appears once with `active == false`.

- [ ] **Step 2: Run the read-model test and verify RED**

Run:

```bash
cargo test -p iotkit-edge --test output_console_read_model
```

Expected: compilation or assertions fail because production composition still creates the old minimal model and selects stopped profiles.

- [ ] **Step 3: Replace iterator-only composition with an async loop**

In `StorageWebApplication::console_outputs`:

```rust
async fn console_outputs(
    &self,
) -> Result<(ConsoleOutputSummary, Vec<ConsoleOutput>), WebError>
```

Create one `OutputProfiles` service and capture `let current_time = now();`. For each registered Adapter:

- choose only a profile in `Preparing`, `Active`, or `Draining`;
- if none exists, call `preview_activation` and create an inactive add card with automatic, configuration, and ineligible counts;
- if a live profile exists, map every binding and semantic rule into `ConsoleBinding`;
- set `future_rules_enabled = true` for a live profile because active profiles already bind newly created compatible rules by application policy;
- skip `publication` only for ineligible and needs-configuration bindings;
- for other bindings, call `publication(binding_id, current_time)`;
- convert successful payloads with `serde_json::to_string_pretty`;
- map provenance to `実際の配送内容`, `最新値からの確認`, or `サンプル`;
- on publication failure, set `preview_failed = true` and use the fixed user text `送信内容を確認できません`; never place `StorageError::to_string()` in the view.

Call `apply_destination_state` for each live destination, then return `summarize(&outputs)` with the complete vector.

- [ ] **Step 4: Wire the summary into `ConsoleView`**

Before constructing `ConsoleView` in `WebApplication::console`, destructure:

```rust
let (output_summary, outputs) = self.console_outputs().await?;
```

Set both fields on `ConsoleView`. Update `web::test_support::StubApplication` to construct one inactive generic Adapter card and one active Pinikiet fixture card with `ConsoleOutput::default()` / `ConsoleBinding::default()` updates rather than repeating every field.

- [ ] **Step 5: Run read-model and publication contract tests**

Run:

```bash
cargo test -p iotkit-edge --test output_console_read_model
cargo test -p iotkit-edge --test preview_contract
cargo test -p iotkit-edge --test web_application_contract
```

Expected: production output composition, existing publication preview behavior, and unrelated Console application behavior all pass.

- [ ] **Step 6: Commit production composition**

```bash
git add edge/src/composition/web.rs edge/src/web/mod.rs edge/tests/output_console_read_model.rs
git commit -m "feat(console): compose output delivery facts"
```

### Task 3: Render status-first output cards with role-safe controls

**Files:**
- Modify: `edge/src/web/templates/console.html:513-554`
- Modify: `edge/frontend/static/edge.css:972-1017`
- Modify: `edge/frontend/static/edge.css:1065-1180`
- Modify: `edge/src/web/mod.rs:1550-2040`
- Modify: `edge/tests/http_contract.rs`

**Interfaces:**
- Consumes: Task 2 `ConsoleView.output_summary`, live/inactive `ConsoleOutput` items, and complete `ConsoleBinding` fields.
- Produces: `.output-health-summary`, `.output-destinations`, `.output-destination-card`, `.output-rule-list`, `.output-technical`, `.output-add-grid`, and existing mutation-form selectors retained for compatibility.

- [ ] **Step 1: Write failing HTML contract assertions**

Add admin and viewer assertions to `edge/tests/http_contract.rs` using `web::test_support::StubApplication`.

The admin response for `/output` must contain:

```rust
assert!(html.contains("正常に送信中"));
assert!(html.contains("設定が必要"));
assert!(html.contains("配送に問題"));
assert!(html.contains("送信対象"));
assert!(html.contains("最終送信"));
assert!(html.contains("配送待ち"));
assert!(html.contains("<details class=\"output-technical\""));
assert!(html.contains("data-copy-text="));
assert!(html.contains("class=\"output-stop-form\""));
```

The viewer response must contain state and technical information while excluding:

```rust
assert!(!html.contains("class=\"output-add-card\""));
assert!(!html.contains("class=\"output-binding-form\""));
assert!(!html.contains("class=\"prepared-output-start\""));
assert!(!html.contains("class=\"output-stop-form\""));
```

- [ ] **Step 2: Run the HTTP contract and verify RED**

Run:

```bash
cargo test -p iotkit-edge --test http_contract
```

Expected: the new summary, disclosure, and role-safe structure assertions fail against the old flat table.

- [ ] **Step 3: Replace the output template section**

Render, in order:

1. the existing viewer permission note;
2. three summary articles using `view.output_summary`;
3. active destinations under `.output-destinations`;
4. inactive Adapters under `.output-add-grid`.

Each active card must have:

```html
<article class="output-destination-card output-state-{{ output.status_class }}">
  <header>…<span class="status-pill {{ output.status_class }}">{{ output.status_label }}</span></header>
  <dl class="output-destination-summary">…</dl>
  <div class="output-rule-list">…</div>
  <details class="output-technical">…</details>
  {% if is_admin %}<details class="output-stop">…existing stop form…</details>{% endif %}
</article>
```

Use `<time data-unix-ms="{{ timestamp }}">` for real timestamps and `まだ送信されていません` for `None`. Keep `form.output-add-card`, `form.output-binding-form`, `form.prepared-output-start`, and `form.output-stop-form` selectors and existing action URLs unchanged.

Technical disclosure must show `edge_node_id`, `signal_ref`, `series_id`, `rule_id`, topic, provenance, and pretty payload. Add `data-copy-text` buttons for topic and payload; Askama escaping remains the only HTML insertion path.

For a live profile, render `今後追加する対応可能な値も自動で送ります` from `future_rules_enabled`. For an inactive Adapter, keep the existing `auto_bind_future_rules` checkbox, make it `required`, and label the primary action `この内容で送信を開始` so continuous application is explicit before activation.

- [ ] **Step 4: Add status-first and narrow-width CSS**

Extend output CSS so:

- summary uses `repeat(3, minmax(0, 1fr))` on desktop and one column below 780px;
- destination facts use four bounded columns on desktop;
- `.output-rule-list` uses block rows rather than a minimum-width table;
- long topic and payload use `min-width: 0`, `overflow-wrap: anywhere`, `white-space: pre-wrap`;
- below 780px, destination headers and rule rows stack vertically;
- forms and buttons become full width only in the narrow layout;
- no output component declares a fixed or minimum width above its card.

Remove the current `.output-technical > div { min-width: min(560px, 70vw); }` rule because it can force horizontal overflow.

- [ ] **Step 5: Run HTML contracts and frontend build**

Run:

```bash
cargo test -p iotkit-edge --test http_contract
npm --prefix edge/frontend run check
npm --prefix edge/frontend run build
node edge/frontend/scripts/check-generated.mjs
```

Expected: role-safe HTML assertions pass, frontend unit tests pass, and committed `static/console.js` remains generated from TypeScript without drift.

- [ ] **Step 6: Commit the rendered experience**

```bash
git add edge/src/web/templates/console.html edge/frontend/static/edge.css edge/src/web/mod.rs edge/tests/http_contract.rs
git commit -m "feat(console): prioritize output delivery state"
```

### Task 4: Extend browser journeys across output lifecycle and narrow width

**Files:**
- Modify: `edge/frontend/e2e/console-journey.mjs:358-422`
- Modify: `edge/frontend/e2e/console-journey.mjs:506-512`
- Modify: `edge/frontend/e2e/rust-console-journey.mjs:507-531`

**Interfaces:**
- Consumes: Task 3 stable selectors and the existing DevTools `send`, `evaluate`, and `navigate` methods.
- Produces: end-to-end evidence for pre-add, configuration/registration wait, sending, viewer visibility, stop, and 390px overflow behavior.

- [ ] **Step 1: Add lifecycle assertions before changing selectors**

Before the first generic Adapter activation, assert all three summary counts are zero and the inactive generic card is in `.output-add-grid`.

After generic activation, assert its active card contains:

```javascript
card.textContent.includes("正常に送信中") &&
card.textContent.includes("送信対象") &&
card.textContent.includes("最終送信") &&
card.textContent.includes("配送待ち") &&
Boolean(card.querySelector(".output-technical"))
```

After Pinikiet activation, assert the Pinikiet card is classified as `設定が必要` while the generic card remains independently normal. Keep the existing external-registration checkbox and `送信開始` operation.

- [ ] **Step 2: Add viewer and technical-disclosure assertions**

For the viewer `/output` visit, assert:

```javascript
document.body.textContent.includes("閲覧のみ") &&
document.body.textContent.includes("配送待ち") &&
Boolean(document.querySelector(".output-technical")) &&
!document.querySelector(
  "form.output-add-card, form.output-binding-form, form.prepared-output-start, form.output-stop-form"
)
```

- [ ] **Step 3: Add the 390px overflow check**

Use the existing DevTools transport:

```javascript
await devtools.send("Emulation.setDeviceMetricsOverride", {
  width: 390,
  height: 844,
  deviceScaleFactor: 1,
  mobile: true,
});
await devtools.navigate(`${edgeNodeURL}/output`, "/output");
assert(
  await devtools.evaluate(
    "document.documentElement.scrollWidth <= document.documentElement.clientWidth",
  ),
  "output page overflows horizontally at 390px",
);
await devtools.send("Emulation.clearDeviceMetricsOverride");
```

Run this after viewer assertions so no mutation depends on mobile emulation.

- [ ] **Step 4: Update Rust journey selectors without weakening stop coverage**

Keep `.output-stop-form` as the submitted form even though it is nested inside `<details>`. Add assertions for the status summary and the active destination facts before submitting stop. Do not replace the stop mutation with a direct API call.

- [ ] **Step 5: Run browser journeys**

Run the repository frontend and browser integration commands used by CI:

```bash
scripts/test-edge-console-frontend.sh
scripts/test-edge-console-e2e.sh
```

Expected: the production fixture covers pre-add, waiting, sending, viewer, stop, and narrow width with no browser exceptions.

- [ ] **Step 6: Commit browser coverage**

```bash
git add edge/frontend/e2e/console-journey.mjs edge/frontend/e2e/rust-console-journey.mjs
git commit -m "test(console): cover output delivery journey"
```

### Task 5: Public change note and complete verification

**Files:**
- Modify: `CHANGELOG.md`
- Verify: all files changed since `origin/master`

**Interfaces:**
- Consumes: all prior task commits.
- Produces: public Unreleased note and complete local/CI-equivalent evidence for the Draft PR.

- [ ] **Step 1: Add the Unreleased change note**

Under the existing Unreleased section in `CHANGELOG.md`, add a Japanese and English bullet stating that Console external output now prioritizes delivery health, target count, last send, backlog, and role-safe technical detail.

- [ ] **Step 2: Run release and document checks**

Run:

```bash
node --test scripts/tests/release-version.test.mjs
node scripts/check-release-version.mjs
node scripts/check-okf-docs.mjs
git diff --check origin/master...HEAD
```

Expected: version consistency remains `0.1.0`, OKF validation passes, and the complete diff has no whitespace errors.

- [ ] **Step 3: Run the full Rust verification gate**

Run:

```bash
scripts/verify.sh
```

Expected: format, layer rules, source layout, all workspace tests, and Clippy with denied warnings pass.

- [ ] **Step 4: Re-run Console integration after the full gate**

Run:

```bash
scripts/test-edge-console-frontend.sh
scripts/test-edge-console-e2e.sh
```

Expected: both browser journeys pass on the exact final tree.

- [ ] **Step 5: Commit the public note**

```bash
git add CHANGELOG.md
git commit -m "docs: note output delivery console"
```

- [ ] **Step 6: Verify final identity and cleanliness**

Run:

```bash
git status --short
git log --oneline origin/master..HEAD
git diff --stat origin/master...HEAD
```

Expected: the worktree is clean, the design/plan and implementation commits are visible, and only Issue #101 files are in the diff.

- [ ] **Step 7: Push and open a Draft PR**

Push `codex/issue-101-output-delivery-status`, open a Draft PR targeting `master`, include `Closes #101`, list exact verification commands, and include the applicable battle-tested review IDs. Stop for human review; do not merge or release.
