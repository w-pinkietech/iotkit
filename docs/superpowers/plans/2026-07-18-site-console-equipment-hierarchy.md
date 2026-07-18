# Site Console Equipment Hierarchy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the fragmented Edge, device, and sensor setup navigation with one hierarchical equipment management journey.

**Architecture:** Add an HTTP-only equipment view model that joins existing `ListEdges` and `ListSetupDevices` results without adding persistence or bypassing application services. Render the hierarchy on a canonical `/equipment` page, retain legacy pages for compatibility, and reuse existing activation/profile mutation handlers.

**Tech Stack:** Go 1.22 `net/http`, `html/template`, embedded CSS and JavaScript, existing `siteapp` services and `httptest` tests.

## Global Constraints

- All Console pages remain login-required.
- Viewer is read-only; admin and system_admin retain existing mutation permissions.
- Do not add direct SQL, a new persistence model, or a new external HTTP API.
- Do not change Edge activation, raw custody, semantic evaluation, or MQTT contracts.
- Preserve `/edges`, `/setup`, `/devices`, and `/signals` as responding legacy URLs.
- Use `GOCACHE=/tmp/iotkit-go-build` for Go commands in the managed workspace.

---

### Task 1: Build the hierarchical equipment view model

**Files:**
- Modify: `iotkit-site/internal/sitehttp/console_view.go`
- Modify: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: `[]siteapp.Edge`, `[]siteapp.SetupDevice`, `newConsoleEdgeViews`, and `newConsoleSetupDeviceViews`.
- Produces: `newConsoleEquipmentViews(edges, devices, now) []consoleEquipmentEdgeView` and orphan device rows.

- [ ] **Step 1: Write a failing grouping test**

Add a test which creates two Edge values and setup devices belonging to only one of them:

```go
func TestConsoleEquipmentViewsGroupDevicesUnderTheirEdge(t *testing.T) {
	now := time.Date(2026, 7, 18, 12, 0, 0, 0, time.UTC)
	edges := []siteapp.Edge{
		{EdgeNodeID: "edge-a", State: siteapp.EdgeActive},
		{EdgeNodeID: "edge-b", State: siteapp.EdgeDiscovered},
	}
	devices := []siteapp.SetupDevice{{
		Device: siteapp.DeviceSummary{Edge: "edge-a", DeviceRef: "dev_a"},
	}}

	rows := newConsoleEquipmentViews(edges, devices, now)

	if len(rows) != 2 || len(rows[0].Devices) != 1 ||
		rows[0].Devices[0].Device.DeviceRef != "dev_a" ||
		len(rows[1].Devices) != 0 {
		t.Fatalf("equipment rows = %#v", rows)
	}
}
```

Add a second test asserting a setup device whose `Device.Edge` has no matching Edge is returned by `newConsoleOrphanDeviceViews`.

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestConsole(Equipment|Orphan)' -count=1
```

Expected: compilation fails because the equipment view functions do not exist.

- [ ] **Step 3: Implement the minimal grouping view**

Add focused view types and functions:

```go
type consoleEquipmentEdgeView struct {
	consoleEdgeView
	Devices              []consoleSetupDeviceView
	DevicePendingCount   int
	SensorPendingCount   int
}

func newConsoleEquipmentViews(
	edges []siteapp.Edge,
	devices []siteapp.SetupDevice,
	now time.Time,
) []consoleEquipmentEdgeView

func newConsoleOrphanDeviceViews(
	edges []siteapp.Edge,
	devices []siteapp.SetupDevice,
	now time.Time,
) []consoleSetupDeviceView
```

Index devices by `SetupDevice.Device.Edge`, keep the order returned by `ListEdges`, and derive pending counts from `SetupDevice.State` and `SetupSignal.ProfileComplete`. Do not synthesize an Edge identity for orphan devices.

- [ ] **Step 4: Run the focused and package tests**

Run:

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestConsole(Equipment|Orphan)' -count=1
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp
```

Expected: both commands pass.

- [ ] **Step 5: Commit**

```bash
git add iotkit-site/internal/sitehttp/console_view.go iotkit-site/internal/sitehttp/server_test.go
git commit -m "feat(site): group equipment by Edge"
```

### Task 2: Add the canonical equipment journey and navigation

**Files:**
- Modify: `iotkit-site/internal/sitehttp/server.go`
- Modify: `iotkit-site/internal/sitehttp/console.go`
- Modify: `iotkit-site/internal/sitehttp/templates/console.html`
- Modify: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: `newConsoleEquipmentViews`, `newConsoleOrphanDeviceViews`, existing activation/device/signal form handlers.
- Produces: authenticated `GET /equipment` and Console navigation labels `機器管理` and `値の変換`.

- [ ] **Step 1: Write failing Console behavior tests**

Add an authenticated Console test that seeds an Edge, descriptor device, and signal, then requests `/equipment` and asserts:

```go
for _, want := range []string{
	"機器管理",
	"Edgeを登録",
	"デバイス名",
	"センサー名",
	"登録前にEdgeが保持していた値",
	`href="/signals"`,
	"値の変換",
} {
	if !strings.Contains(body, want) {
		t.Fatalf("equipment page missing %q: %s", want, body)
	}
}
```

Add assertions that the sidebar contains exactly one equipment link and does not render visible links labelled `Edge管理`, `デバイス管理`, or `センサー設定`.

Add a viewer request asserting the hierarchy and values are visible but `Edgeを登録`, `name="display_name"`, and the physical identifier are absent.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestEquipmentConsole|TestConsoleNavigationUsesEquipmentJourney' -count=1
```

Expected: `/equipment` returns 404 or the required hierarchy/navigation text is absent.

- [ ] **Step 3: Wire the page and read models**

Register:

```go
server.mux.HandleFunc("GET /equipment", server.consolePage)
```

Extend `consoleData`:

```go
EquipmentRows  []consoleEquipmentEdgeView
OrphanDevices []consoleSetupDeviceView
```

For page `equipment`, call `ListEdges` and `ListSetupDevices`, then build both view collections. Add `equipment` to the title and description maps. Extend `consoleReturnTarget` so `/equipment` is a safe mutation return target.

- [ ] **Step 4: Render the hierarchy and simplify navigation**

Replace the three visible setup navigation entries with:

```html
<a href="/equipment" {{if eq .Page "equipment"}}class="active" aria-current="page"{{end}}>
  <span class="nav-symbol">◇</span>機器管理
</a>
<a href="/signals" {{if eq .Page "signals"}}class="active" aria-current="page"{{end}}>
  <span class="nav-symbol">⌁</span>値の変換
</a>
```

Render an `equipment` section containing:

- a four-step non-blocking journey strip;
- Edge header/status/communication/counts;
- activation warning and existing activation form for eligible admins;
- nested device profile form;
- nested sensor raw facts and profile form;
- `現在値を見る` and `値の変換` links after basic setup;
- an explicit orphan device warning group.

Every existing mutation form must set `return_to=/equipment`. Keep legacy page blocks unchanged for direct URL compatibility.

- [ ] **Step 5: Update overview links**

Change overview setup links from `/setup` to `/equipment`. Count unregistered Edge separately from registered Edge and make the first unresolved setup action the primary status message.

- [ ] **Step 6: Run focused and package tests**

Run:

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestEquipmentConsole|TestConsoleNavigationUsesEquipmentJourney' -count=1
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp
```

Expected: both commands pass.

- [ ] **Step 7: Commit**

```bash
git add iotkit-site/internal/sitehttp/server.go iotkit-site/internal/sitehttp/console.go iotkit-site/internal/sitehttp/templates/console.html iotkit-site/internal/sitehttp/server_test.go
git commit -m "feat(site): unify equipment setup journey"
```

### Task 3: Make the hierarchy visually usable and verify the operator journey

**Files:**
- Modify: `iotkit-site/internal/sitehttp/static/site.css`
- Modify: `iotkit-site/internal/sitehttp/static/console.js`
- Modify: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: semantic HTML and data attributes rendered by Task 2.
- Produces: responsive hierarchy, accessible disclosure behavior, and preservation of profile field behavior.

- [ ] **Step 1: Add a failing asset/render contract test**

Extend the equipment Console test to require stable class and data hooks:

```go
for _, want := range []string{
	`class="equipment-journey"`,
	`class="equipment-edge`,
	`class="equipment-device`,
	`class="equipment-sensor`,
	`data-signal-profile`,
} {
	if !strings.Contains(body, want) {
		t.Fatalf("equipment page missing visual contract %q: %s", want, body)
	}
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run TestEquipmentConsole -count=1
```

Expected: the new hierarchy class contract is absent.

- [ ] **Step 3: Implement responsive equipment styling**

Add CSS for:

```text
.equipment-journey
.equipment-step
.equipment-list
.equipment-edge
.equipment-edge-header
.equipment-device-list
.equipment-device
.equipment-device-header
.equipment-sensor-list
.equipment-sensor
.equipment-sensor-reading
.equipment-orphans
```

Use border weight, indentation, and surface color to express ancestry. At widths below 760px, collapse all multi-column facts and forms to one column, remove indentation that causes horizontal overflow, and keep buttons at least 44px high.

- [ ] **Step 4: Preserve form interaction**

Use existing `data-signal-profile` behavior for conditional sensor fields. Add only progressive enhancement for disclosure labels and do not require JavaScript for viewing or saving any setup form.

- [ ] **Step 5: Run package tests and formatting checks**

Run:

```bash
env GOCACHE=/tmp/iotkit-go-build gofmt -w internal/sitehttp/console.go internal/sitehttp/console_view.go internal/sitehttp/server_test.go
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp
git diff --check
```

Expected: tests pass and `git diff --check` produces no output.

- [ ] **Step 6: Start a local fixture Console and review**

Start the repository-supported development Console with seeded Edge, device, temperature sensor, contact sensor, admin account, and development HTTP. Verify:

- login works;
- `/equipment` shows the complete ancestry;
- unregistered and registered Edge states are distinguishable;
- viewer cannot see mutation controls;
- admin forms retain values after save;
- `/monitor` shows configured names and units;
- layout works at desktop and narrow widths.

- [ ] **Step 7: Commit**

```bash
git add iotkit-site/internal/sitehttp/static/site.css iotkit-site/internal/sitehttp/static/console.js iotkit-site/internal/sitehttp/server_test.go
git commit -m "style(site): clarify equipment hierarchy"
```
