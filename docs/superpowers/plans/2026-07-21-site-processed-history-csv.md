# Site Processed History CSV Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Export Site's durably stored semantic observations as the default operator CSV while retaining raw history CSV as a clearly labeled diagnostic option.

**Architecture:** Add a bounded read model over `semantic_observations_v3`, joined only to stable Site signal/rule/profile metadata. Expose it through a dedicated authenticated CSV endpoint, then point the Console primary action to it without changing output-adapter contracts or recomputing historical semantics.

**Tech Stack:** Go, SQLite, `net/http`, `encoding/csv`, OpenAPI, generated TypeScript declarations, server-rendered HTML, browser E2E.

## Global Constraints

- The maximum time range is 31 days.
- The export must fail with HTTP 422 before writing CSV when more than 100,000 observations match.
- Persisted semantic values and revisions are authoritative; historical rules must not be re-evaluated.
- Output-adapter topics and payloads are out of scope.
- Site mutations remain behind typed `siteapp` dispatch; this feature is read-only.

---

### Task 1: Semantic history read model

**Files:**
- Modify: `iotkit-site/internal/store/history.go`
- Modify: `iotkit-site/internal/store/history_test.go`
- Modify: `iotkit-site/internal/store/migrations.go`
- Modify: `iotkit-site/internal/store/migrations_test.go`

**Interfaces:**
- Produces: `Store.QuerySemanticHistory(context.Context, SemanticHistoryQuery) (SemanticHistoryPage, error)`.
- Produces: rows containing observation identity, rule/calibration revisions, sensor/rule labels, kind, scalar value, unit, Edge, and timestamps.

- [ ] Write failing Store tests for multiple rules, filters, unit behavior, ordering, and limit-plus-one detection.
- [ ] Run `go test ./internal/store -run 'SemanticHistory|Migrations'` and confirm the new tests fail because the read model and migration are absent.
- [ ] Add the semantic history types/query and an `observed_at` history index migration.
- [ ] Run the focused Store tests and confirm they pass.

### Task 2: Authenticated processed CSV API

**Files:**
- Modify: `iotkit-site/internal/sitehttp/history.go`
- Modify: `iotkit-site/internal/sitehttp/server.go`
- Modify: `iotkit-site/internal/sitehttp/server_test.go`
- Modify: `iotkit-site/openapi/site-console-v1.yaml`
- Regenerate: `iotkit-site/frontend/src/generated/site-api.d.ts`

**Interfaces:**
- Consumes: `Store.QuerySemanticHistory`.
- Produces: `GET /api/v1/semantic-history.csv` with the exact 15-column contract from the design.

- [ ] Write failing HTTP tests for auth, BOM/header/rows, formula-safe strings, and 100,000-row refusal.
- [ ] Run the focused HTTP tests and confirm 404 or missing-handler failures.
- [ ] Add the route and CSV writer, reusing bounded history request parsing and `csvSafeCell`.
- [ ] Add the OpenAPI operation and schema-independent CSV response contract.
- [ ] Regenerate TypeScript API declarations and run focused Go tests.

### Task 3: Console operator boundary

**Files:**
- Modify: `iotkit-site/internal/sitehttp/console.go`
- Modify: `iotkit-site/internal/sitehttp/console_view.go`
- Modify: `iotkit-site/internal/sitehttp/templates/console.html`
- Modify: `iotkit-site/internal/sitehttp/console_view_test.go`
- Modify: `iotkit-site/frontend/e2e/console-journey.mjs`
- Modify: `docs/redesign/decisions/D13-ui-scope.md`

**Interfaces:**
- Consumes: processed and raw CSV endpoints with identical filter parameters.
- Produces: a primary `加工後CSV` action and a secondary `受信した生データCSV` action.

- [ ] Write failing Console view and browser assertions for the two clearly labeled actions.
- [ ] Run focused Console tests and confirm the semantic link/copy is absent.
- [ ] Add separate processed/raw export URLs and update the history explanation without changing the raw graph/table.
- [ ] Update D13 to make processed generic export the default and raw export diagnostic-only.
- [ ] Run focused Console tests.

### Task 4: Final verification

**Files:**
- Verify all files changed above.

**Interfaces:**
- Confirms the generated contract, backend behavior, and real browser path agree.

- [ ] Run `gofmt` on modified Go files.
- [ ] Run `npm run generate:api` and `npm run check` in `iotkit-site/frontend`.
- [ ] Run `scripts/verify.sh` once after implementation.
- [ ] Run `scripts/test-site-console-e2e.sh` in the local network-enabled environment.
- [ ] Run `git diff --check` and inspect `git status --short`.
- [ ] Do not commit or push until the user explicitly approves it.

