# Site Console Sensor Navigation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Separate sensor monitoring from sensor configuration so the URL, sidebar selection, breadcrumb, and available actions always describe the current operator task.

**Architecture:** Keep the existing sensor read model, preview API, mutation endpoints, and editor controls. Add a canonical device-owned settings route, render a read-only monitoring detail at the existing sensor route, and pass one validated settings return path through every editor form.

**Tech Stack:** Go 1.24 `net/http` and `html/template`, SQLite-backed Site services, TypeScript/Vitest console behavior, plain CSS.

## Global Constraints

- `/sensors/{signal_ref}` contains no mutation forms.
- `/equipment/devices/{device_ref}/sensors/{signal_ref}` is the canonical settings URL.
- A settings URL is valid only when the signal belongs to the named device.
- Monitoring selects `センサー一覧`; settings selects `機器管理`.
- Existing APIs, database schema, MQTT contracts, rule semantics, permissions, and authentication remain unchanged.
- No legacy or compatibility route is added.

---

### Task 1: Route context and ownership

**Files:**
- Modify: `iotkit-site/internal/sitehttp/server.go`
- Modify: `iotkit-site/internal/sitehttp/console.go`
- Test: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: `device_ref` and `signal_ref` path values, `siteapp.SignalSummary.DeviceRef`.
- Produces: `consoleData.NavigationPage`, `consoleData.SensorSettingsPath`, `SensorView == "settings"`, and validated selected device/edge context.

- [ ] **Step 1: Write the failing route tests**

Add tests that request both canonical pages and assert:

```go
settingsPath := "/equipment/devices/" + devices[0].DeviceRef +
    "/sensors/" + signals[0].SignalRef
```

The monitoring response has `data-console-page="sensors"`, while the settings response has `data-console-page="equipment"` and returns `404` when the device and signal do not match.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
TMPDIR=/home/kenta/dev/iot/.tmp/iotkit-next-build \
GOCACHE=/tmp/iotkit-next-go-cache \
go test ./internal/sitehttp -run 'TestSensor(Monitoring|Settings)Route' -count=1
```

Expected: FAIL because the nested settings route is not registered and the monitoring page still contains the editor.

- [ ] **Step 3: Implement the route context**

Register:

```go
server.mux.HandleFunc(
    "GET /equipment/devices/{device_ref}/sensors/{signal_ref}",
    server.consolePage,
)
```

Add explicit navigation state to `consoleData`, load the sensor through the existing signal view pipeline, load the equipment hierarchy for a settings request, and reject a settings URL unless `SelectedSignal.DeviceRef` equals `device_ref`.

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run the command from Step 2. Expected: PASS.

### Task 2: Monitoring and settings presentation

**Files:**
- Modify: `iotkit-site/internal/sitehttp/templates/console.html`
- Modify: `iotkit-site/internal/sitehttp/static/site.css`
- Test: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: `NavigationPage`, `SensorView`, `SensorSettingsPath`, `SelectedSignal`, `SelectedDevice`, and `SelectedDeviceEdge`.
- Produces: read-only monitoring detail and editable equipment settings detail with distinct breadcrumbs and actions.

- [ ] **Step 1: Write failing presentation tests**

Assert that monitoring contains:

```text
センサー一覧 / {sensor}
設定内容
設定を変更
```

for an administrator, but contains no `/console/signals/` or `/console/semantic-rules/` form actions. Assert that settings contains all existing editor actions, `機器管理` as the active navigation item, and the equipment hierarchy breadcrumb. Assert that viewers do not see `設定を変更`.

- [ ] **Step 2: Run the presentation tests and verify RED**

Run:

```bash
TMPDIR=/home/kenta/dev/iot/.tmp/iotkit-next-build \
GOCACHE=/tmp/iotkit-next-go-cache \
go test ./internal/sitehttp -run 'TestSensor(MonitoringDetail|SettingsDetail)' -count=1
```

Expected: FAIL because the current detail combines monitoring and editing.

- [ ] **Step 3: Split the template by responsibility**

Use `NavigationPage` for sidebar selection. Render the common sensor header and preview for both modes. In monitoring mode render a concise configuration summary and the administrator-only settings link. In settings mode render the existing tabbed forms and use `SensorSettingsPath` for every `return_to`.

- [ ] **Step 4: Add restrained summary styling**

Add CSS only for the monitoring summary and the header settings action. Reuse existing spacing, typography, status pills, graph, responsive breakpoint, and content-section styles.

- [ ] **Step 5: Run the presentation tests and verify GREEN**

Run the command from Step 2. Expected: PASS.

### Task 3: Canonical mutation returns

**Files:**
- Modify: `iotkit-site/internal/sitehttp/console.go`
- Modify: `iotkit-site/internal/sitehttp/templates/console.html`
- Test: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: form field `return_to`.
- Produces: `safeSensorSettingsReturnTarget(string) bool` and redirects back to the nested equipment settings URL with query and anchor restoration.

- [ ] **Step 1: Write failing return-target tests**

Accept:

```text
/equipment/devices/dev_0123456789abcdef0123456789abcdef/sensors/sig_0123456789abcdef0123456789abcdef
```

Reject missing IDs, wrong prefixes, query strings, fragments, backslashes, protocol-relative URLs, and extra path segments.

- [ ] **Step 2: Run the return-target tests and verify RED**

Run:

```bash
TMPDIR=/home/kenta/dev/iot/.tmp/iotkit-next-build \
GOCACHE=/tmp/iotkit-next-go-cache \
go test ./internal/sitehttp -run 'TestConsoleReturnTarget' -count=1
```

Expected: FAIL because the nested path is not accepted.

- [ ] **Step 3: Implement strict nested-path validation**

Split the path into exactly five segments and validate both resource references:

```go
return len(parts) == 5 &&
    parts[0] == "equipment" &&
    parts[1] == "devices" &&
    validConsoleResourceRef(parts[2], "dev_") &&
    parts[3] == "sensors" &&
    validConsoleResourceRef(parts[4], "sig_")
```

Replace every sensor editor form's `return_to` with `SensorSettingsPath`.

- [ ] **Step 4: Run the return-target and mutation tests**

Run:

```bash
TMPDIR=/home/kenta/dev/iot/.tmp/iotkit-next-build \
GOCACHE=/tmp/iotkit-next-go-cache \
go test ./internal/sitehttp -run 'TestConsole(ReturnTarget|MutationResult|CreatesNormalAndAlarmRules)' -count=1
```

Expected: PASS.

### Task 4: Full verification and browser review

**Files:**
- Modify only if verification reveals a defect in files already in scope.

**Interfaces:**
- Consumes: completed monitoring and settings routes.
- Produces: verified desktop console ready for user inspection.

- [ ] **Step 1: Build the tracked TypeScript bundle**

Run the repository's existing frontend build command and verify `static/console.js` matches the TypeScript source.

- [ ] **Step 2: Run frontend tests**

Run the existing Vitest suite. Expected: all tests pass.

- [ ] **Step 3: Run the Site HTTP suite**

Run:

```bash
TMPDIR=/home/kenta/dev/iot/.tmp/iotkit-next-build \
GOCACHE=/tmp/iotkit-next-go-cache \
go test ./internal/sitehttp -count=1
```

Expected: PASS.

- [ ] **Step 4: Browser-review both routes**

Review the monitoring and settings pages at 1440×1024 and 1100×900. Verify sidebar state, breadcrumb, absence/presence of forms, graph rendering, keyboard focus, and no horizontal clipping.

- [ ] **Step 5: Commit the implementation**

Commit the tested route, template, CSS, and test changes together with a message describing the navigation separation.
