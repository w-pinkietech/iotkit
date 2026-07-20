# Site Console Sensor Editor Simplification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the sensor editor visually quiet by showing identity, editing controls, and live results once each.

**Architecture:** Keep the existing server-rendered Go template and TypeScript behavior. Reshape only the sensor-detail template and its CSS; preserve all form actions, data attributes, tab behavior, preview behavior, permissions, and return anchors.

**Tech Stack:** Go `html/template`, CSS, TypeScript, Vitest, Go `testing`, headless Chromium

## Global Constraints

- Do not change the Site API, database schema, MQTT contracts, rule semantics, permissions, or authentication.
- Keep Basic, Normal value, and Alarm as local tabs.
- Keep the live preview visible beside the active editor on wide desktop layouts.
- Preserve keyboard tab selection, focus restoration, switch semantics, inline validation, and the accessible chart summary.
- Remove duplicate summaries and use progressive disclosure for secondary controls.

---

### Task 1: Remove duplicate sensor-detail information

**Files:**
- Modify: `iotkit-site/internal/sitehttp/server_test.go`
- Modify: `iotkit-site/internal/sitehttp/templates/console.html`

**Interfaces:**
- Consumes: `SelectedSignal`, `NormalRules`, `AlarmRules`, `IsAdmin`, and the existing form actions/data attributes.
- Produces: `.sensor-detail-header`, `.sensor-setting-workspace`, `[data-setting-tabs]`, and `[data-setting-simulation]`.

- [ ] **Step 1: Write the failing template test**

Add assertions to `TestSensorDetailShowsCurrentValueSourceAndSettingsForAdmin`:

```go
for _, want := range []string{
    `class="sensor-detail-header"`,
    `class="sensor-detail-identity"`,
    `class="sensor-detail-latest`,
} {
    if !strings.Contains(body, want) {
        t.Fatalf("sensor detail missing compact header %q: %s", want, body)
    }
}
for _, forbidden := range []string{
    `class="sensor-data-flow"`,
    `class="semantic-rule-output"`,
    `正常値の使い方`,
} {
    if strings.Contains(body, forbidden) {
        t.Fatalf("sensor detail retains duplicate content %q: %s", forbidden, body)
    }
}
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
env GOCACHE=/tmp/iotkit-next-go-cache go test ./internal/sitehttp -run 'TestSensorDetailShowsCurrentValueSourceAndSettingsForAdmin|TestSensorDetailSeparatesNormalAndAbnormalSemanticRules' -count=1
```

Expected: FAIL because `.sensor-detail-header` is absent and the old flow/summary is present.

- [ ] **Step 3: Replace the persistent flow with a compact identity header**

In the sensor detail template, retain the breadcrumb and replace `.sensor-data-flow` with:

```html
<header class="sensor-detail-header">
  <div class="sensor-detail-identity">
    <span>{{.SensorType}}</span>
    <h2>{{.Name}}</h2>
    <small>{{.Edge}}{{if .DeviceName}} · {{.DeviceName}}{{end}}</small>
  </div>
  <div class="sensor-detail-latest sensor-detail-latest--{{.StatusClass}}">
    <span class="status-pill {{.StatusClass}}">{{.StatusLabel}}</span>
    <strong><span data-source-current-value>{{.SourceValue}}</span>{{if .SourceUnit}} <small>{{.SourceUnit}}</small>{{end}}</strong>
    <small data-source-current-received>最終受信 {{.LastReceived}}</small>
  </div>
</header>
```

Delete `.semantic-rule-output` blocks from normal and alarm rules. Replace the normal group heading with a terse multiple-rule selector label, shown only when needed:

```html
{{if gt (len .NormalRules) 1}}<p class="semantic-rule-picker-label">編集する通常値</p>{{end}}
```

Do the equivalent for alarm rules. Keep each `<details class="semantic-rule-card">`, every form, rule ID, action, return anchor, and destructive disclosure unchanged.

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run the command from Step 2.

Expected: PASS.

- [ ] **Step 5: Commit the template slice**

```bash
git add iotkit-site/internal/sitehttp/server_test.go iotkit-site/internal/sitehttp/templates/console.html
git commit -m "feat: simplify sensor editor hierarchy"
```

### Task 2: Flatten the editor and preview styling

**Files:**
- Modify: `iotkit-site/internal/sitehttp/static/site.css`

**Interfaces:**
- Consumes: `.sensor-detail-header`, `.sensor-detail-identity`, `.sensor-detail-latest`, `.sensor-setting-workspace`, `.sensor-settings-panel`, `.sensor-setting-simulation`, `.semantic-rule-card`.
- Produces: a two-column wide layout and a one-column layout at `max-width: 1100px`.

- [ ] **Step 1: Add a structure check that fails on obsolete CSS**

Extend the sensor-detail test to read `static/site.css` and assert:

```go
for _, forbidden := range []string{
    ".sensor-data-flow {",
    ".semantic-rule-output {",
    ".semantic-rule-group > header {",
} {
    if strings.Contains(css, forbidden) {
        t.Fatalf("sensor editor stylesheet retains obsolete layer %q", forbidden)
    }
}
```

Use the existing repository helper for reading fixture/static files if one exists; otherwise use `os.ReadFile("static/site.css")`.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
env GOCACHE=/tmp/iotkit-next-go-cache go test ./internal/sitehttp -run TestSensorDetailShowsCurrentValueSourceAndSettingsForAdmin -count=1
```

Expected: FAIL because the obsolete selectors are still present.

- [ ] **Step 3: Replace layered card styling with two quiet surfaces**

Remove the obsolete flow, output-summary, and group-header rules. Add:

```css
.sensor-detail-header {
  display: flex;
  align-items: end;
  justify-content: space-between;
  gap: 24px;
  padding: 2px 2px 16px;
  border-bottom: 1px solid var(--line);
}
.sensor-detail-identity { display: grid; gap: 3px; }
.sensor-detail-identity > span,
.sensor-detail-identity > small { color: var(--muted); font-size: .78rem; }
.sensor-detail-identity h2 { margin: 0; font-size: 1.35rem; }
.sensor-detail-latest { display: grid; grid-template-columns: auto auto; align-items: baseline; gap: 2px 12px; text-align: right; }
.sensor-detail-latest > strong { font-family: "JetBrains Mono", ui-monospace, monospace; font-size: 1.45rem; font-weight: 500; }
.sensor-detail-latest > small { grid-column: 1 / -1; color: var(--muted); font-size: .72rem; }
.sensor-setting-workspace { gap: 24px; margin-top: 6px; }
.sensor-settings-panel,
.sensor-setting-simulation { padding: 18px; border-radius: 11px; box-shadow: none; }
.sensor-settings-panel > .section-heading { display: none; }
.semantic-rule-group { overflow: visible; border: 0; border-radius: 0; }
.semantic-rule-card { border: 0; }
.semantic-rule-card > summary { padding: 8px 0; border-bottom: 1px solid var(--line); }
.semantic-rule-card > form { padding: 14px 0 0; }
.semantic-rule-danger { margin: 0; }
.semantic-rule-picker-label { margin: 0 0 6px; color: var(--muted); font-size: .78rem; }
```

Simplify preview copy and framing: keep the current value, chart, legend, status, switch, and test input; remove the preview subtitle and redundant bordered/tinted test wrapper. Keep accessible labels and semantic colors.

- [ ] **Step 4: Run the focused test and verify GREEN**

Run the command from Step 2.

Expected: PASS.

- [ ] **Step 5: Build generated frontend assets and run focused suites**

Run:

```bash
cd iotkit-site/frontend
npm run build
npm run check
cd ..
env GOCACHE=/tmp/iotkit-next-go-cache go test ./internal/sitehttp -count=1
```

Expected: 4 Vitest files and 14 tests pass; Site HTTP tests pass.

- [ ] **Step 6: Commit the visual slice**

```bash
git add iotkit-site/internal/sitehttp/server_test.go iotkit-site/internal/sitehttp/static/site.css iotkit-site/internal/sitehttp/static/console.js
git commit -m "style: reduce sensor editor visual noise"
```

### Task 3: Verify the result at desktop sizes

**Files:**
- No product files expected

**Interfaces:**
- Consumes: the rendered sensor detail page.
- Produces: visual evidence at 1440×1024 and 1100×900.

- [ ] **Step 1: Render a configured normal rule and alarm rule**

Use the existing local browser preview harness or a temporary Go test outside committed product files. Seed one numeric normal rule and one high-threshold alarm rule.

- [ ] **Step 2: Review 1440×1024**

Confirm:

```text
sensor identity and latest value appear once
no persistent three-stage flow
no duplicate rule/output summary above the form
active settings and graph are visible together
save action appears in the initial viewport for common numeric and alarm forms
```

- [ ] **Step 3: Review 1100×900**

Confirm the layout becomes one column without clipped controls, horizontal scrolling, or overlapping text.

- [ ] **Step 4: Run final verification**

Run:

```bash
cd iotkit-site/frontend
npm run build && npm run check
cd ..
env GOCACHE=/tmp/iotkit-next-go-cache go test ./internal/sitehttp -count=1
git diff --check
```

Expected: all commands exit 0.
