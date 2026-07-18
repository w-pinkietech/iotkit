# Site Console Sensor Master-Detail Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the separate current-value and value-conversion screens with a sensor list and a permission-aware sensor detail screen.

**Architecture:** Add canonical `/sensors` and `/sensors/{signal_ref}` console routes while keeping the existing signal mutation APIs. `consolePage` loads the existing signal summaries, semantic definitions, and device summaries into one selected sensor view; templates render a compact list or a focused detail page. Legacy GET routes redirect to the canonical list.

**Tech Stack:** Go 1.25, `net/http`, `html/template`, embedded CSS/JavaScript, SQLite-backed Site services.

## Global Constraints

- The canonical list URL is `/sensors`.
- The canonical detail URL is `/sensors/{signal_ref}`.
- Viewer accounts can read details but cannot see or submit setting forms.
- Admin and system-admin accounts can edit profile and semantic settings.
- Existing POST and API URLs remain compatible.
- Unknown sensor refs return HTTP 404.
- Existing `/monitor` and `/signals` GET routes redirect to `/sensors`.
- No history graph, bulk edit, per-sensor ACL, or Edge/device registration is added.

---

### Task 1: Canonical Sensor Routes and Selection

**Files:**
- Modify: `iotkit-site/internal/sitehttp/server.go`
- Modify: `iotkit-site/internal/sitehttp/console.go`
- Test: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: `siteapp.Service.ListSignals`, `siteapp.Service.ListDevices`, `SemanticService.List`.
- Produces: `consoleData.SensorView string`, `consoleData.SelectedSignal *consoleSignalView`, GET `/sensors`, and GET `/sensors/{signal_ref}`.

- [ ] **Step 1: Write failing route tests**

Add tests which seed one device and assert:

```go
func TestSensorRoutesResolveListAndKnownDetail(t *testing.T) {
	server, archive := newTestServerFixture(t, false, siteapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 100, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err = %v", signals, err)
	}
	cookie, _ := loginTestAccount(t, server)
	for _, path := range []string{"/sensors", "/sensors/" + signals[0].SignalRef} {
		request := httptest.NewRequest(http.MethodGet, path, nil)
		request.AddCookie(cookie)
		response := httptest.NewRecorder()
		server.ServeHTTP(response, request)
		if response.Code != http.StatusOK {
			t.Fatalf("%s status = %d, body=%s", path, response.Code, response.Body.String())
		}
	}
}
```

Add a second test asserting `/sensors/sig_00000000000000000000000000000000` returns 404.

- [ ] **Step 2: Run the route tests and confirm RED**

Run:

```bash
cd iotkit-site
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestSensorRoutes' -count=1
```

Expected: FAIL because `/sensors` is handled by the root redirect and the detail route does not exist.

- [ ] **Step 3: Add routes and selection data**

Register:

```go
server.mux.HandleFunc("GET /sensors", server.consolePage)
server.mux.HandleFunc("GET /sensors/{signal_ref}", server.consolePage)
```

Extend `consoleData`:

```go
SensorView     string
SelectedSignal *consoleSignalView
```

Normalize any request with `request.PathValue("signal_ref") != ""` to page `sensors`. Add title `センサー一覧` and description `各センサーの現在値を確認し、詳細から設定できます。`

Load summaries and definitions, build `SignalRows`, then select by `SignalRef`. Return `http.NotFound` when the ref is not present.

- [ ] **Step 4: Run tests and confirm GREEN**

Run:

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestSensorRoutes' -count=1
```

Expected: PASS.

- [ ] **Step 5: Commit the route slice**

```bash
git add iotkit-site/internal/sitehttp/server.go iotkit-site/internal/sitehttp/console.go iotkit-site/internal/sitehttp/server_test.go
git commit -m "feat(site): add sensor list and detail routes"
```

---

### Task 2: Sensor List and Legacy Navigation

**Files:**
- Modify: `iotkit-site/internal/sitehttp/server.go`
- Modify: `iotkit-site/internal/sitehttp/templates/console.html`
- Modify: `iotkit-site/internal/sitehttp/static/site.css`
- Test: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: `consoleData.SignalRows`.
- Produces: a linked table at `/sensors`; redirects from `/monitor` and `/signals`.

- [ ] **Step 1: Write failing list and redirect tests**

Assert the list contains `センサー一覧`, current value `24.8`, Edge `factory-edge-01`, and `href="/sensors/{signal_ref}"`, but no profile or semantic form.

Assert:

```go
for _, path := range []string{"/monitor", "/signals"} {
	request := httptest.NewRequest(http.MethodGet, path, nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusSeeOther ||
		response.Header().Get("Location") != "/sensors" {
		t.Fatalf("%s response = %d location=%q", path, response.Code, response.Header().Get("Location"))
	}
}
```

- [ ] **Step 2: Run tests and confirm RED**

Run:

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestSensorList|TestLegacySensorConsoleRoutes' -count=1
```

Expected: FAIL because the new list template and redirects are absent.

- [ ] **Step 3: Implement the list and redirects**

Change the sidebar link to:

```html
<a href="/sensors" {{if eq .Page "sensors"}}class="active" aria-current="page"{{end}}><span class="nav-symbol">●</span>センサー一覧</a>
```

Render the toolbar and a linked sensor table for `Page == "sensors"` and `SensorView == "list"`. Each row uses:

```html
<tr data-status="{{.StatusClass}}" data-href="/sensors/{{.SignalRef}}">
  <td><a href="/sensors/{{.SignalRef}}"><strong>{{.Name}}</strong></a></td>
  <td><span class="sensor-type">{{.SensorType}}</span></td>
  <td>{{.Edge}}</td>
  <td><span class="measurement">{{.Value}}</span>{{if .Unit}} <span class="measurement-unit">{{.Unit}}</span>{{end}}</td>
  <td><span class="status-pill {{.StatusClass}}">{{.StatusLabel}}</span></td>
  <td title="{{.LastReceivedTitle}}">{{.LastReceived}}</td>
</tr>
```

Register `/monitor` and `/signals` with a browser-authenticated handler that redirects to `/sensors`. Update status and equipment links from `/monitor` or `/signals` to `/sensors`.

- [ ] **Step 4: Run tests and confirm GREEN**

Run:

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestSensorList|TestLegacySensorConsoleRoutes|TestConsoleNavigation' -count=1
```

Expected: PASS.

- [ ] **Step 5: Commit the list slice**

```bash
git add iotkit-site/internal/sitehttp/server.go iotkit-site/internal/sitehttp/templates/console.html iotkit-site/internal/sitehttp/static/site.css iotkit-site/internal/sitehttp/server_test.go
git commit -m "feat(site): make sensors a linked current-value list"
```

---

### Task 3: Permission-Aware Sensor Detail and Settings

**Files:**
- Modify: `iotkit-site/internal/sitehttp/console.go`
- Modify: `iotkit-site/internal/sitehttp/console_view.go`
- Modify: `iotkit-site/internal/sitehttp/templates/console.html`
- Modify: `iotkit-site/internal/sitehttp/static/console.js`
- Test: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: `consoleData.SelectedSignal`, existing profile candidate fields, and existing `semantics.Definition`.
- Produces: one detail page containing read-only facts plus admin-only profile and semantic forms.

- [ ] **Step 1: Write failing detail and permission tests**

For an admin detail response, assert all of:

```go
[]string{
	`class="sensor-detail"`,
	`class="sensor-detail-current"`,
	"factory-edge-01",
	"temperature_c",
	"24.8",
	`action="/console/signals/`,
	`name="display_name"`,
	`name="kind"`,
	`name="return_to" value="/sensors/`,
}
```

For a viewer, assert current value and source facts are present while `name="display_name"`, `name="kind"`, and internal signal refs are absent.

- [ ] **Step 2: Run tests and confirm RED**

Run:

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestSensorDetail' -count=1
```

Expected: FAIL because the detail template is absent.

- [ ] **Step 3: Extend the view and safe return target**

Add source presentation fields to `consoleSignalView`:

```go
SourceSensorType string
SourceValueType  string
SourceUnit       string
ChannelLabel     string
DeviceName       string
DeviceLocation   string
```

Populate them from `SignalSummary.SensorType`, `ValueType`, `Unit`, and `ChannelIndex`. Load devices for the sensors page and match `DeviceRef` to supply a human-readable device name and location.

Extend `consoleReturnTarget` to accept only a valid `/sensors/{sig_<32 lowercase hex>}` path using `validConsoleResourceRef`.

- [ ] **Step 4: Render the detail**

Render:

- breadcrumb back to `/sensors`;
- current-value hero with status and last received;
- source facts with Edge and linked device detail when `DeviceRef` is known;
- admin-only profile form posting to `/console/signals/{signal_ref}/profile`;
- admin-only semantic form posting to `/console/signals/{signal_ref}/semantic`;
- each form carrying `return_to=/sensors/{signal_ref}`.

Keep existing `data-signal-profile`, `data-semantic-kind`, conditional fields, and preview-button hooks so the current JavaScript behavior remains available.

- [ ] **Step 5: Return semantic saves to the detail**

Change `consoleSemantic` to use:

```go
server.consoleMutationResult(
	response,
	request,
	consoleReturnTarget(request, "/sensors/"+request.PathValue("signal_ref")),
	err,
)
```

- [ ] **Step 6: Run tests and confirm GREEN**

Run:

```bash
env GOCACHE=/tmp/iotkit-go-build go test ./internal/sitehttp -run 'TestSensorDetail|TestConsoleReturnTarget' -count=1
```

Expected: PASS.

- [ ] **Step 7: Commit the detail slice**

```bash
git add iotkit-site/internal/sitehttp/console.go iotkit-site/internal/sitehttp/console_view.go iotkit-site/internal/sitehttp/templates/console.html iotkit-site/internal/sitehttp/static/console.js iotkit-site/internal/sitehttp/server_test.go
git commit -m "feat(site): add sensor detail settings"
```

---

### Task 4: Responsive Polish and Verification

**Files:**
- Modify: `iotkit-site/internal/sitehttp/static/site.css`
- Test: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Consumes: list/detail class hooks from Tasks 2 and 3.
- Produces: usable desktop and mobile list/detail layouts.

- [ ] **Step 1: Add responsive visual contracts**

Add CSS hooks:

```css
.sensor-list-table tr[data-href] { cursor: pointer; }
.sensor-list-table a { color: inherit; text-decoration: none; }
.sensor-detail { display: grid; gap: 20px; }
.sensor-detail-current { border-left: 5px solid var(--teal); }
.sensor-detail-settings { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
@media (max-width: 780px) {
  .sensor-detail-settings { grid-template-columns: 1fr; }
}
```

Use existing surface, line, orange, teal, typography, and status-pill tokens.

- [ ] **Step 2: Run Site tests**

Run:

```bash
cd iotkit-site
env GOCACHE=/tmp/iotkit-go-build go test ./...
```

Expected: all packages PASS.

- [ ] **Step 3: Check the patch**

Run:

```bash
git diff --check
git status --short
```

Expected: no whitespace errors and only intended Site Console files changed.

- [ ] **Step 4: Inspect real pages**

Start the Tailscale-bound preview with no test timeout. Capture and inspect:

- `/sensors` at 1440×1000 and 390×844;
- one `/sensors/{signal_ref}` at 1440×1000 and 390×844;
- viewer detail to confirm no mutation controls.

Verify login, list-to-detail navigation, profile save return, semantic save return, and legacy redirects.

- [ ] **Step 5: Commit the visual and verification slice**

```bash
git add iotkit-site/internal/sitehttp/static/site.css iotkit-site/internal/sitehttp/server_test.go
git commit -m "style(site): polish sensor master detail console"
```
