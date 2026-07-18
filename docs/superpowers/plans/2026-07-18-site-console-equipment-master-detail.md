# Site Console Equipment Master Detail Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the long nested equipment page with separate Edge list, Edge detail, and device detail pages.

**Architecture:** Keep existing Site application services and persistence unchanged. Add HTTP view selection by public `edge_ref` and `device_ref`, render three server-side page variants, and preserve all existing activation/profile mutation handlers with validated dynamic return paths.

**Tech Stack:** Go 1.22 `net/http`, `html/template`, embedded CSS/JavaScript, SQLite-backed `siteapp`, `httptest`.

## Global Constraints

- All Console pages remain login-required.
- Viewer remains read-only; admin and system_admin retain existing mutation permissions.
- Do not add a DB table, external API, JavaScript router, or MQTT contract.
- Use public `edge_ref` and `device_ref` in URLs; do not use internal identity.
- Unknown refs return 404.
- Dynamic `return_to` accepts only exact equipment detail paths without query, fragment, host, or extra slash.
- Use `GOCACHE=/tmp/iotkit-go-build` for Go tests.

---

### Task 1: Select one Edge or device with safe detail routes

**Files:**
- Modify: `iotkit-site/internal/sitehttp/server.go`
- Modify: `iotkit-site/internal/sitehttp/console.go`
- Modify: `iotkit-site/internal/sitehttp/console_view.go`
- Modify: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: `ListEdges`, `ListSetupDevices`, `consoleEquipmentEdgeView`, `consoleSetupDeviceView`.
- Produces: authenticated Edge and device detail routes plus safe mutation return targets.

- [ ] **Step 1: Add failing route and selection tests**

Add tests which request:

```go
"/equipment/edges/"+edge.EdgeRef
"/equipment/devices/"+device.DeviceRef
```

Assert the selected view returns 200, an unknown ref returns 404, and a viewer still receives 200. Add table tests for:

```go
valid := []string{
	"/equipment",
	"/equipment/edges/edge_0123456789abcdef0123456789abcdef",
	"/equipment/devices/dev_0123456789abcdef0123456789abcdef",
}
invalid := []string{
	"https://example.test/equipment",
	"/equipment/edges/",
	"/equipment/edges/edge_a/extra",
	"/equipment/devices/dev_a?next=/system",
	"//example.test/equipment",
}
```

The valid values must be returned by `consoleReturnTarget`; invalid values must use the fallback.

- [ ] **Step 2: Run tests and verify RED**

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestEquipmentDetail|TestConsoleReturnTarget' -count=1
```

Expected: detail routes return 404 or redirect, and dynamic return paths are rejected.

- [ ] **Step 3: Add routes and focused view data**

Register:

```go
server.mux.HandleFunc("GET /equipment/edges/{edge_ref}", server.consolePage)
server.mux.HandleFunc("GET /equipment/devices/{device_ref}", server.consolePage)
```

Add to `consoleData`:

```go
EquipmentView string
SelectedEdge  *consoleEquipmentEdgeView
SelectedDevice *consoleSetupDeviceView
```

In `consolePage`, normalize all three paths to `Page = "equipment"`, load `ListEdges` and `ListSetupDevices` once, select the requested public ref, and call `http.NotFound` when no row matches.

- [ ] **Step 4: Validate dynamic return paths**

Add:

```go
func safeEquipmentReturnTarget(target string) bool
```

Accept `/equipment`, or exactly three non-empty path segments for `equipment/edges/{ref}` and `equipment/devices/{ref}`. Reject values containing `?`, `#`, `\`, scheme/host syntax, trailing slash, or an unexpected public-ref prefix.

- [ ] **Step 5: Run focused and package tests**

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestEquipmentDetail|TestConsoleReturnTarget' -count=1
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp
```

Expected: all commands pass.

- [ ] **Step 6: Commit**

```bash
git add iotkit-site/internal/sitehttp/server.go iotkit-site/internal/sitehttp/console.go iotkit-site/internal/sitehttp/console_view.go iotkit-site/internal/sitehttp/server_test.go
git commit -m "feat(site): add equipment detail routes"
```

### Task 2: Render Edge list, Edge detail, and device detail

**Files:**
- Modify: `iotkit-site/internal/sitehttp/templates/console.html`
- Modify: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: `EquipmentView`, `EquipmentRows`, `SelectedEdge`, `SelectedDevice`.
- Produces: three distinct operator pages without nested forms on the Edge list.

- [ ] **Step 1: Add failing page responsibility tests**

For `/equipment`, assert:

```go
required := []string{"Edge一覧", "詳細を見る", edge.EdgeRef}
forbidden := []string{`action="/console/devices/`, `data-signal-profile`, "Adapterから届いた情報"}
```

For one Edge detail, assert its name, breadcrumb, device summary, and device detail link are present, while sensor form is absent.

For one device detail, assert breadcrumb, device profile form for admin, current value, sensor summary, and `data-signal-profile` are present.

For viewer device detail, assert value and breadcrumb are visible while profile forms and physical identifier are absent.

- [ ] **Step 2: Run tests and verify RED**

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestEquipment(List|EdgeDetail|DeviceDetail)' -count=1
```

Expected: the current single nested page violates at least the list responsibility assertions.

- [ ] **Step 3: Render the Edge list**

Render a compact Edge list with:

```html
<a href="/equipment/edges/{{.EdgeRef}}" class="button secondary">詳細を見る</a>
```

Show state, device count, sensor count, last communication, and pending configuration count. Do not render device or sensor details on this variant.

- [ ] **Step 4: Render Edge detail**

Render breadcrumb, selected Edge header, activation state/action, diagnostic disclosure, and a compact device list. For each active Edge device, link:

```html
<a href="/equipment/devices/{{.Device.DeviceRef}}">詳細を見る</a>
```

Use hidden `return_to=/equipment/edges/{edge_ref}` for activation. Do not render device or sensor forms.

- [ ] **Step 5: Render device detail**

Render breadcrumb back to the selected Edge, device identity/status/facts, one device profile disclosure, and sibling sensor cards. Each sensor shows current value and one profile disclosure. Use hidden `return_to=/equipment/devices/{device_ref}` for both device and signal forms.

- [ ] **Step 6: Run focused and package tests**

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestEquipment(List|EdgeDetail|DeviceDetail)' -count=1
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp
```

Expected: all commands pass.

- [ ] **Step 7: Commit**

```bash
git add iotkit-site/internal/sitehttp/templates/console.html iotkit-site/internal/sitehttp/server_test.go
git commit -m "feat(site): split equipment list and details"
```

### Task 3: Replace nested-card styling and verify the live Console

**Files:**
- Modify: `iotkit-site/internal/sitehttp/static/site.css`
- Modify: `iotkit-site/internal/sitehttp/templates/console.html`
- Modify: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: the semantic list/detail markup from Task 2.
- Produces: compact desktop tables/cards and single-column mobile details.

- [ ] **Step 1: Add failing visual contract assertions**

Require these hooks on the corresponding pages:

```text
equipment-overview
equipment-row
equipment-breadcrumb
equipment-detail-header
equipment-device-table
equipment-sensor-grid
```

Assert the Edge list does not contain `equipment-sensor-grid`.

- [ ] **Step 2: Run the visual contract tests and verify RED**

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestEquipment(List|EdgeDetail|DeviceDetail)' -count=1
```

Expected: one or more visual hooks are absent.

- [ ] **Step 3: Implement the visual system**

Replace the large nested equipment styles with:

- compact bordered list rows for Edge and device overviews;
- one breadcrumb line above page content;
- a detail header with state and primary action;
- sensor cards at one sibling depth;
- editor disclosures with a neutral surface;
- 44px minimum controls;
- table-to-card transformation below 780px;
- no horizontal overflow at 390px.

- [ ] **Step 4: Verify package behavior**

```bash
gofmt -w internal/sitehttp/console.go internal/sitehttp/console_view.go internal/sitehttp/server_test.go
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp
git diff --check
```

Expected: tests pass and diff check emits no output.

- [ ] **Step 5: Render desktop and mobile fixtures**

Serve an authenticated fixture containing one active Edge with a configured temperature sensor and one unregistered Edge. Capture 1440px and 390px screenshots. Verify list scanability, breadcrumb placement, form disclosure, mobile wrapping, and no pre-activation device form.

- [ ] **Step 6: Run Site-wide tests**

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./...
```

Expected: every Site package passes.

- [ ] **Step 7: Commit**

```bash
git add iotkit-site/internal/sitehttp/static/site.css iotkit-site/internal/sitehttp/templates/console.html iotkit-site/internal/sitehttp/server_test.go
git commit -m "style(site): simplify equipment management"
```
