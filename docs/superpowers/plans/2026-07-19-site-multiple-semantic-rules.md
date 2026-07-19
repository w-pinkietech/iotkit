# Site Multiple Semantic Rules Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow one Site signal to share one future-only calibration while independently producing multiple numeric, state, cumulative-counter, and alarm observations.

**Architecture:** Add semantic schema v3 beside the preserved v2 tables. Stable rules own immutable kinds, series identity, runtime, sequence, receipts, reset commands, and output routes; calibration and rule revisions own independent accepted-cursor boundaries. Switch the Site application service, HTTP API, preview, projector, and Console to v3 without changing raw custody.

**Tech Stack:** Go 1.24, SQLite, `net/http`, server-rendered HTML, embedded JavaScript/CSS.

## Global Constraints

- Raw records, accepted cursors, Edge registration, descriptors, profiles, accounts, sessions, and Broker profiles must remain intact.
- Semantic v2 tables remain stored but are never inferred into v3 rules or delivered by the v3 runtime.
- A signal has one active calibration and at most 16 active rules.
- Rule kind, rule ID, series ID, and observation sequence are stable across revisions.
- Calibration/rule changes are future-only; counter reset is cursor-ordered and idempotent.
- A failed rule must not block another rule or raw custody.
- Site mutations go through the typed Site application service.
- Run focused tests during implementation and broad verification once after code completion.

---

### Task 1: Semantic v3 domain types and schema

**Files:**
- Modify: `iotkit-site/internal/semantics/types.go`
- Modify: `iotkit-site/internal/semantics/evaluator.go`
- Test: `iotkit-site/internal/semantics/evaluator_test.go`
- Modify: `iotkit-site/internal/store/migrations.go`
- Test: `iotkit-site/internal/store/migrations_test.go`

**Interfaces:**
- Produces: `Calibration`, `RuleSpec`, `Rule`, `Configuration`, `CounterReset`, and v3 `Observation`.
- Produces: `EvaluateRule(spec RuleSpec, state State, calibrated float64, receivedAt int64)`.

- [ ] **Step 1: Write failing type/evaluator tests**

```go
func TestEvaluateRuleUsesAlreadyCalibratedInput(t *testing.T) {
    result, _, err := EvaluateRule(
        RuleSpec{Kind: KindNumeric},
        State{},
        21.5,
        1000,
    )
    if err != nil || result.Number == nil || *result.Number != 21.5 {
        t.Fatalf("result=%#v err=%v", result, err)
    }
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `env GOCACHE=/tmp/iotkit-go-build go test ./internal/semantics -run 'TestEvaluateRule'`
Expected: FAIL because `RuleSpec` and `EvaluateRule` do not exist.

- [ ] **Step 3: Add the v3 domain types and split calibration from rule evaluation**

`Calibration.Apply(value)` validates finite scale/offset/input/output. `RuleSpec.Validate()` validates kind-specific detector and trigger fields without scale/offset. Keep the v2 `DefinitionSpec` adapter temporarily so preserved v2 tests compile.

- [ ] **Step 4: Add migration 14 with v3 tables**

Create calibration revisions/starts, stable rules, rule revisions/starts/ends, runtime, receipts, observations, failures, resets, and v3 YokaKit routes/outbox. Add uniqueness for one active calibration revision and one active rule revision, but not one active rule per signal.

- [ ] **Step 5: Run semantic and migration tests**

Run: `env GOCACHE=/tmp/iotkit-go-build go test ./internal/semantics ./internal/store -run 'TestEvaluateRule|TestMigration'`
Expected: PASS.

### Task 2: Calibration and stable rule lifecycle

**Files:**
- Create: `iotkit-site/internal/store/semantic_v3.go`
- Create: `iotkit-site/internal/store/semantic_v3_test.go`
- Modify: `iotkit-site/internal/siteapp/semantics.go`
- Modify: `iotkit-site/internal/siteapp/service_test.go`

**Interfaces:**
- Produces: `GetSemanticConfiguration`, `UpdateSignalCalibration`, `CreateSemanticRule`, `UpdateSemanticRule`, `RetireSemanticRule`.
- Consumes: accepted cursors as future-only boundaries and typed `siteapp.Actor`.

- [ ] **Step 1: Write failing Store tests**

Test identity calibration creation, two rules on one signal, 16-rule limit, duplicate display-name rejection, immutable kind, stable ID/series ID across update, future-only start/end rows, and audit records.

- [ ] **Step 2: Run Store tests and verify RED**

Run: `env GOCACHE=/tmp/iotkit-go-build go test ./internal/store -run 'TestSemanticV3'`
Expected: FAIL because v3 lifecycle methods do not exist.

- [ ] **Step 3: Implement v3 lifecycle transactionally**

Each operation resolves `signal_ref`, applies revision preconditions, records all current accepted cursors, writes the semantic change and audit in one transaction, and returns the current configuration.

- [ ] **Step 4: Add application-service authorization and validation**

Viewer may read. Admin/system-admin may change. Every resource ref is validated before repository dispatch.

- [ ] **Step 5: Run Store and application tests**

Run: `env GOCACHE=/tmp/iotkit-go-build go test ./internal/store ./internal/siteapp -run 'TestSemanticV3|TestSemanticService'`
Expected: PASS.

### Task 3: Independent projection and ordered counter reset

**Files:**
- Modify: `iotkit-site/internal/store/semantic_v3.go`
- Modify: `iotkit-site/internal/store/semantic_v3_test.go`

**Interfaces:**
- Produces: `ProjectSemanticRules(ctx, limit)`, `ListSemanticObservationsV3`, `RequestSemanticCounterReset`.
- Consumes: `semantics.Calibration.Apply` and `semantics.EvaluateRule`.

- [ ] **Step 1: Write failing projection tests**

Cover numeric/count/alarm from one signal, stable sequence across revision, re-baseline after calibration/rule update, poison-rule isolation, receipt idempotency, persisted debounce, and active-rule retirement.

- [ ] **Step 2: Write failing reset tests**

Request reset while projection is behind, verify input through the accepted cursor is counted first, then an explicit zero observation is emitted, then later input counts from zero. Repeating the same reset ID must not emit another zero.

- [ ] **Step 3: Run focused tests and verify RED**

Run: `env GOCACHE=/tmp/iotkit-go-build go test ./internal/store -run 'TestSemanticV3Projection|TestSemanticV3CounterReset'`
Expected: FAIL because projection/reset are not implemented.

- [ ] **Step 4: Implement rule-isolated candidates, receipts, runtime, failures, and reset barriers**

Process each rule/raw candidate in its own SQLite transaction. Store failures per rule/cursor and continue other rules. Before reading input after a pending reset boundary, apply the reset only after all eligible input through the boundary has receipts.

- [ ] **Step 5: Run projection/reset tests**

Run: `env GOCACHE=/tmp/iotkit-go-build go test ./internal/store -run 'TestSemanticV3'`
Expected: PASS.

### Task 4: Configuration API and multi-rule preview

**Files:**
- Modify: `iotkit-site/internal/sitehttp/server.go`
- Modify: `iotkit-site/internal/sitehttp/api_v1.go`
- Modify: `iotkit-site/internal/sitehttp/preview.go`
- Modify: `iotkit-site/internal/semantics/preview.go`
- Modify: `iotkit-site/internal/semantics/preview_test.go`
- Modify: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Produces: the approved calibration/rule/reset endpoints and the compatible `/api/v1/mapping-previews` multi-rule response.

- [ ] **Step 1: Write failing API tests**

Test authenticated configuration read, admin-only calibration/rule mutations, ETag/If-Match, 16-rule validation, rule-targeted delete, idempotency-key reset, secret-free DTOs, and multiple preview results.

- [ ] **Step 2: Run HTTP tests and verify RED**

Run: `env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestSemanticConfiguration|TestMultiRulePreview'`
Expected: FAIL because the new routes do not exist.

- [ ] **Step 3: Implement handlers and DTO validation**

Map all mutations through `SemanticService`. Preview applies one calibration, independently evaluates each draft/saved rule, returns rule-scoped field errors, and performs no writes.

- [ ] **Step 4: Run semantic preview and HTTP tests**

Run: `env GOCACHE=/tmp/iotkit-go-build go test ./internal/semantics ./internal/sitehttp -run 'TestSemanticConfiguration|TestMultiRulePreview|TestBuildPreview'`
Expected: PASS.

### Task 5: Console multiple-rule setting experience

**Files:**
- Modify: `iotkit-site/internal/sitehttp/console.go`
- Modify: `iotkit-site/internal/sitehttp/console_view.go`
- Modify: `iotkit-site/internal/sitehttp/templates/console.html`
- Modify: `iotkit-site/internal/sitehttp/static/console.js`
- Modify: `iotkit-site/internal/sitehttp/static/site.css`
- Modify: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: v3 configuration and preview API.
- Produces: shared input correction, normal-value cards, abnormal-detection cards, and one expanded selected rule.

- [ ] **Step 1: Write failing Console tests**

Assert two rules are visible on one sensor, normal/alarm grouping is presentation-only, forms target rule IDs, viewer sees no mutation controls, and reset targets one counter rule.

- [ ] **Step 2: Run Console tests and verify RED**

Run: `env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestConsoleSensorMultipleRules'`
Expected: FAIL because the page still renders one definition form.

- [ ] **Step 3: Implement view models, handlers, template, JavaScript, and CSS**

Keep the chart visible beside the settings. Save calibration independently. Render compact cards; only the selected rule expands. Preview all rules but emphasize the selected card.

- [ ] **Step 4: Run Console tests**

Run: `env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestConsoleSensor|TestSemanticConfiguration|TestMultiRulePreview'`
Expected: PASS.

### Task 6: Stable-rule external output

**Files:**
- Modify: `iotkit-site/internal/store/output_v2.go`
- Modify: `iotkit-site/internal/store/semantic_v3.go`
- Modify: `iotkit-site/internal/store/semantic_v3_test.go`
- Modify: `iotkit-site/internal/sitehttp/api_v1.go`
- Modify: `iotkit-site/internal/sitehttp/console.go`
- Modify: `iotkit-site/internal/sitehttp/console_view.go`

**Interfaces:**
- Produces: YokaKit routes bound to stable `rule_id`; v2 route tables remain isolated.

- [ ] **Step 1: Write failing route tests**

Create a route for one rule, revise the rule, project a new observation, and verify the route still exports it exactly once. Retiring the rule must preserve already queued output.

- [ ] **Step 2: Run route tests and verify RED**

Run: `env GOCACHE=/tmp/iotkit-go-build go test ./internal/store -run 'TestSemanticV3Output'`
Expected: FAIL because output still binds `definition_id`.

- [ ] **Step 3: Implement v3 route/outbox queries and switch API/Console wording**

Do not infer v2 definitions into v3 rules. Keep stable observation IDs and route+observation uniqueness.

- [ ] **Step 4: Run output and HTTP tests**

Run: `env GOCACHE=/tmp/iotkit-go-build go test ./internal/store ./internal/sitehttp -run 'TestSemanticV3Output|TestYokaKitOutput'`
Expected: PASS.

### Task 7: Focused review and final verification

**Files:**
- Modify only files required by review findings.

- [ ] **Step 1: Review the diff against the approved specs**

Check rule independence, future-only boundaries, reset ordering, data preservation, typed mutation paths, role checks, output identity, and Console terminology.

- [ ] **Step 2: Run format and focused tests**

Run: `gofmt -w <changed-go-files>`
Run: `env GOCACHE=/tmp/iotkit-go-build go test ./internal/semantics ./internal/store ./internal/siteapp ./internal/sitehttp`
Expected: PASS.

- [ ] **Step 3: Run the one broad pre-PR verification**

Run: `env GOCACHE=/tmp/iotkit-go-build go test ./...`
Expected: PASS.

- [ ] **Step 4: Inspect the final diff**

Run: `git diff --check && git status --short`
Expected: no whitespace errors; only intentional source, tests, docs, and the pre-existing `.review-runs/` remain.
