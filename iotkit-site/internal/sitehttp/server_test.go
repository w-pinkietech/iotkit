package sitehttp

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteauth"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/sitesession"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/store"
)

const testOrigin = "https://iotkit.example.test"

func TestAuthenticatedAPIDeniesAnonymousInventoryAndSetsSecurityHeaders(t *testing.T) {
	server := newTestServer(t, false)
	request := httptest.NewRequest(http.MethodGet, "/api/v1/devices", nil)
	response := httptest.NewRecorder()

	server.ServeHTTP(response, request)

	if response.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want 401; body=%s", response.Code, response.Body.String())
	}
	if response.Header().Get("Cache-Control") != "no-store" {
		t.Fatalf("Cache-Control = %q", response.Header().Get("Cache-Control"))
	}
	if !strings.Contains(response.Header().Get("Content-Security-Policy"), "default-src 'self'") {
		t.Fatalf("CSP = %q", response.Header().Get("Content-Security-Policy"))
	}
	if response.Header().Get("Access-Control-Allow-Origin") != "" {
		t.Fatalf("anonymous response enables CORS: %#v", response.Header())
	}
	if response.Header().Get("Referrer-Policy") != "same-origin" {
		t.Fatalf("Referrer-Policy = %q, want same-origin",
			response.Header().Get("Referrer-Policy"))
	}
}

func TestEdgeActivationAPIRequiresAdminAndQueuesActivation(t *testing.T) {
	adminServer, archive := newTestServerFixture(
		t, false, siteapp.AccountRoleAdmin,
	)
	edge := seedDiscoveredEdge(t, archive)

	unauthorized := httptest.NewRequest(
		http.MethodPost,
		"/api/v1/edges/"+edge.EdgeRef+"/activation",
		nil,
	)
	unauthorizedResponse := httptest.NewRecorder()
	adminServer.ServeHTTP(unauthorizedResponse, unauthorized)
	if unauthorizedResponse.Code != http.StatusUnauthorized {
		t.Fatalf("unauthorized status = %d", unauthorizedResponse.Code)
	}

	viewerServer, viewerArchive := newTestServerFixture(
		t, false, siteapp.AccountRoleViewer,
	)
	viewerEdge := seedDiscoveredEdge(t, viewerArchive)
	viewerCookie, viewerCSRF := loginTestAccount(t, viewerServer)
	viewerRequest := httptest.NewRequest(
		http.MethodPost,
		"/api/v1/edges/"+viewerEdge.EdgeRef+"/activation",
		nil,
	)
	viewerRequest.AddCookie(viewerCookie)
	viewerRequest.Header.Set("Origin", testOrigin)
	viewerRequest.Header.Set("X-CSRF-Token", viewerCSRF)
	viewerResponse := httptest.NewRecorder()
	viewerServer.ServeHTTP(viewerResponse, viewerRequest)
	if viewerResponse.Code != http.StatusForbidden {
		t.Fatalf("viewer status = %d, body=%s",
			viewerResponse.Code, viewerResponse.Body.String())
	}

	adminCookie, adminCSRF := loginTestAccount(t, adminServer)
	adminRequest := httptest.NewRequest(
		http.MethodPost,
		"/api/v1/edges/"+edge.EdgeRef+"/activation",
		nil,
	)
	adminRequest.AddCookie(adminCookie)
	adminRequest.Header.Set("Origin", testOrigin)
	adminRequest.Header.Set("X-CSRF-Token", adminCSRF)
	adminRequest.Header.Set("If-Match", revisionETag(edge.Revision))
	adminResponse := httptest.NewRecorder()
	adminServer.ServeHTTP(adminResponse, adminRequest)
	if adminResponse.Code != http.StatusAccepted {
		t.Fatalf("admin status = %d, body=%s",
			adminResponse.Code, adminResponse.Body.String())
	}
	var activated siteapp.Edge
	if err := json.Unmarshal(adminResponse.Body.Bytes(), &activated); err != nil {
		t.Fatal(err)
	}
	if activated.State != siteapp.EdgeActivating ||
		strings.Contains(adminResponse.Body.String(), "activation_id") {
		t.Fatalf("activated Edge = %#v", activated)
	}
	commands, err := archive.ListPendingActivationCommands(
		context.Background(), 10,
	)
	if err != nil || len(commands) != 1 {
		t.Fatalf("commands = %#v, err = %v", commands, err)
	}
}

func TestEdgesConsoleShowsEdgeAsParentOfDevices(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, siteapp.AccountRoleAdmin,
	)
	edge := seedDiscoveredEdge(t, archive)
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(http.MethodGet, "/edges", nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", response.Code, response.Body.String())
	}
	body := response.Body.String()
	for _, want := range []string{
		"Edge管理",
		"未登録",
		"1台のデバイス",
		"1件のセンサー",
		"最終通信",
		edge.LedgerEpoch,
		"Edgeを登録",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("Edge console missing %q: %s", want, body)
		}
	}
	for _, forbidden := range []string{
		"activation_id",
		"request_json",
		"result_json",
		"接続中",
	} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("Edge console exposes forbidden term %q", forbidden)
		}
	}
}

func TestConsoleEquipmentViewsGroupDevicesUnderTheirEdge(t *testing.T) {
	now := time.Date(2026, 7, 18, 12, 0, 0, 0, time.UTC)
	edges := []siteapp.Edge{
		{EdgeNodeID: "edge-a", State: siteapp.EdgeActive},
		{EdgeNodeID: "edge-b", State: siteapp.EdgeDiscovered},
	}
	devices := []siteapp.SetupDevice{{
		Device: siteapp.DeviceSummary{Edge: "edge-a", DeviceRef: "dev_a"},
		State:  siteapp.SetupWaitingForDevice,
		Signals: []siteapp.SetupSignal{{
			Signal:          siteapp.SignalSummary{SignalRef: "sig_a"},
			ProfileComplete: false,
		}},
	}}

	rows := newConsoleEquipmentViews(edges, devices, now)

	if len(rows) != 2 || len(rows[0].Devices) != 1 ||
		rows[0].Name != "edge-a" ||
		rows[0].Devices[0].Device.DeviceRef != "dev_a" ||
		rows[0].DevicePendingCount != 1 ||
		rows[0].SensorPendingCount != 1 ||
		len(rows[1].Devices) != 0 {
		t.Fatalf("equipment rows = %#v", rows)
	}
}

func TestConsoleOrphanDeviceViewsKeepDevicesWithoutMatchingEdge(t *testing.T) {
	now := time.Date(2026, 7, 18, 12, 0, 0, 0, time.UTC)
	edges := []siteapp.Edge{{EdgeNodeID: "edge-a", State: siteapp.EdgeActive}}
	devices := []siteapp.SetupDevice{
		{Device: siteapp.DeviceSummary{Edge: "edge-a", DeviceRef: "dev_a"}},
		{Device: siteapp.DeviceSummary{Edge: "missing-edge", DeviceRef: "dev_orphan"}},
	}

	rows := newConsoleOrphanDeviceViews(edges, devices, now)

	if len(rows) != 1 || rows[0].Device.DeviceRef != "dev_orphan" {
		t.Fatalf("orphan device rows = %#v", rows)
	}
}

func TestEquipmentDetailRoutesResolveKnownResources(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, siteapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	cookie, _ := loginTestAccount(t, server)
	edges, err := archive.ListEdges(context.Background())
	if err != nil || len(edges) != 1 {
		t.Fatalf("edges = %#v, err = %v", edges, err)
	}
	devices, err := archive.ListInventoryDevices(context.Background(), 100, "")
	if err != nil || len(devices) != 1 {
		t.Fatalf("devices = %#v, err = %v", devices, err)
	}

	for _, path := range []string{
		"/equipment/edges/" + edges[0].EdgeRef,
		"/equipment/devices/" + devices[0].DeviceRef,
	} {
		request := httptest.NewRequest(http.MethodGet, path, nil)
		request.AddCookie(cookie)
		response := httptest.NewRecorder()
		server.ServeHTTP(response, request)
		if response.Code != http.StatusOK {
			t.Fatalf("%s status = %d, body=%s", path, response.Code, response.Body.String())
		}
	}
}

func TestEquipmentDetailRoutesReturnNotFoundForUnknownResources(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, siteapp.AccountRoleAdmin,
	)
	seedDiscoveredEdge(t, archive)
	cookie, _ := loginTestAccount(t, server)
	for _, path := range []string{
		"/equipment/edges/edge_00000000000000000000000000000000",
		"/equipment/devices/dev_00000000000000000000000000000000",
	} {
		request := httptest.NewRequest(http.MethodGet, path, nil)
		request.AddCookie(cookie)
		response := httptest.NewRecorder()
		server.ServeHTTP(response, request)
		if response.Code != http.StatusNotFound {
			t.Fatalf("%s status = %d, want 404; body=%s", path, response.Code, response.Body.String())
		}
	}
}

func TestEquipmentDetailRoutesAllowViewerAccess(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, siteapp.AccountRoleViewer,
	)
	seedSetupDevice(t, archive)
	edges, err := archive.ListEdges(context.Background())
	if err != nil || len(edges) != 1 {
		t.Fatalf("edges = %#v, err = %v", edges, err)
	}
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodGet,
		"/equipment/edges/"+edges[0].EdgeRef,
		nil,
	)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", response.Code, response.Body.String())
	}
}

func TestEquipmentListShowsEdgeSummariesWithoutNestedSettings(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, siteapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	seedAdditionalDiscoveredEdge(t, archive)
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(http.MethodGet, "/equipment", nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	body := response.Body.String()
	for _, want := range []string{
		`class="equipment-overview"`,
		"factory-edge-01",
		"assembly-edge-02",
		"1台",
		"1件",
		`href="/equipment/edges/`,
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("equipment list missing %q: %s", want, body)
		}
	}
	for _, forbidden := range []string{
		`action="/console/devices/`,
		`action="/console/signals/`,
		`data-signal-profile`,
	} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("equipment list exposes nested setting %q: %s", forbidden, body)
		}
	}
}

func TestEquipmentEdgeDetailShowsDeviceSummaryLinks(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, siteapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	edges, err := archive.ListEdges(context.Background())
	if err != nil || len(edges) != 1 {
		t.Fatalf("edges = %#v, err = %v", edges, err)
	}
	devices, err := archive.ListInventoryDevices(context.Background(), 100, "")
	if err != nil || len(devices) != 1 {
		t.Fatalf("devices = %#v, err = %v", devices, err)
	}
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodGet,
		"/equipment/edges/"+edges[0].EdgeRef,
		nil,
	)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	body := response.Body.String()
	for _, want := range []string{
		`class="equipment-breadcrumb"`,
		`class="equipment-detail-header`,
		`class="equipment-device-table"`,
		`href="/equipment/devices/` + devices[0].DeviceRef + `"`,
		"名前未設定のデバイス",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("Edge detail missing %q: %s", want, body)
		}
	}
	for _, forbidden := range []string{
		`action="/console/devices/`,
		`action="/console/signals/`,
	} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("Edge detail exposes nested setting %q: %s", forbidden, body)
		}
	}
}

func TestEquipmentDeviceDetailContainsDeviceAndSensorSettings(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, siteapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	devices, err := archive.ListInventoryDevices(context.Background(), 100, "")
	if err != nil || len(devices) != 1 {
		t.Fatalf("devices = %#v, err = %v", devices, err)
	}
	cookie, _ := loginTestAccount(t, server)
	path := "/equipment/devices/" + devices[0].DeviceRef
	request := httptest.NewRequest(http.MethodGet, path, nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	body := response.Body.String()
	for _, want := range []string{
		`class="equipment-breadcrumb"`,
		`class="equipment-sensor-grid"`,
		`action="/console/devices/` + devices[0].DeviceRef + `/profile"`,
		`action="/console/signals/`,
		`name="return_to" value="` + path + `"`,
		"24.8",
		"温度",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("device detail missing %q: %s", want, body)
		}
	}
}

func TestConsoleReturnTargetAllowsOnlyEquipmentDetailPaths(t *testing.T) {
	valid := []string{
		"/equipment",
		"/equipment/edges/edge_0123456789abcdef0123456789abcdef",
		"/equipment/devices/dev_0123456789abcdef0123456789abcdef",
	}
	for _, target := range valid {
		request := httptest.NewRequest(http.MethodPost, "/", strings.NewReader(
			url.Values{"return_to": {target}}.Encode(),
		))
		request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		if got := consoleReturnTarget(request, "/signals"); got != target {
			t.Errorf("consoleReturnTarget(%q) = %q", target, got)
		}
	}

	invalid := []string{
		"https://example.test/equipment",
		"//example.test/equipment",
		"/equipment/edges/",
		"/equipment/devices/",
		"/equipment/edges/edge_",
		"/equipment/devices/dev_not-a-resource-ref",
		"/equipment/edges/edge_a/extra",
		"/equipment/edges/edge_a?changed=1",
	}
	for _, target := range invalid {
		request := httptest.NewRequest(http.MethodPost, "/", strings.NewReader(
			url.Values{"return_to": {target}}.Encode(),
		))
		request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		if got := consoleReturnTarget(request, "/signals"); got != "/signals" {
			t.Errorf("consoleReturnTarget(%q) = %q, want fallback", target, got)
		}
	}
}

func TestSensorRoutesResolveListAndKnownDetail(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, siteapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 100, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err = %v", signals, err)
	}
	cookie, _ := loginTestAccount(t, server)
	for _, path := range []string{
		"/sensors",
		"/sensors/" + signals[0].SignalRef,
	} {
		request := httptest.NewRequest(http.MethodGet, path, nil)
		request.AddCookie(cookie)
		response := httptest.NewRecorder()
		server.ServeHTTP(response, request)
		if response.Code != http.StatusOK {
			t.Fatalf("%s status = %d, body=%s", path, response.Code, response.Body.String())
		}
	}
}

func TestSensorRoutesReturnNotFoundForUnknownDetail(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, siteapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodGet,
		"/sensors/sig_00000000000000000000000000000000",
		nil,
	)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want 404; body=%s", response.Code, response.Body.String())
	}
}

func TestConsoleNavigationUsesEquipmentJourney(t *testing.T) {
	server := newTestServer(t, false)
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(http.MethodGet, "/status", nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()

	server.ServeHTTP(response, request)

	body := response.Body.String()
	if count := strings.Count(body, `href="/equipment"`); count != 1 {
		t.Fatalf("equipment navigation links = %d, want 1: %s", count, body)
	}
	for _, forbidden := range []string{
		`</span>Edge管理`,
		`</span>デバイス管理`,
		`</span>センサー設定`,
	} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("navigation still exposes %q: %s", forbidden, body)
		}
	}
	if !strings.Contains(body, `</span>機器管理`) ||
		!strings.Contains(body, `</span>値の変換`) {
		t.Fatalf("navigation does not use equipment journey: %s", body)
	}
}

func TestLoginCreatesStrictCookiesAndAllowsInventoryRead(t *testing.T) {
	server := newTestServer(t, false)
	body := `{"login_id":"operator","password":"現場担当者の 十分に長いパスワード"}`
	login := httptest.NewRequest(http.MethodPost, "/api/v1/session", strings.NewReader(body))
	login.Header.Set("Content-Type", "application/json")
	login.Header.Set("Origin", testOrigin)
	loginResponse := httptest.NewRecorder()

	server.ServeHTTP(loginResponse, login)

	if loginResponse.Code != http.StatusCreated {
		t.Fatalf("login status = %d, body=%s", loginResponse.Code, loginResponse.Body.String())
	}
	var sessionResponse struct {
		CSRFToken string          `json:"csrf_token"`
		Account   siteapp.Account `json:"account"`
	}
	if err := json.Unmarshal(loginResponse.Body.Bytes(), &sessionResponse); err != nil {
		t.Fatal(err)
	}
	if sessionResponse.CSRFToken == "" || sessionResponse.Account.LoginID != "operator" {
		t.Fatalf("login response = %#v", sessionResponse)
	}
	var sessionCookie *http.Cookie
	for _, cookie := range loginResponse.Result().Cookies() {
		if cookie.Name == sessionCookieName {
			sessionCookie = cookie
		}
	}
	if sessionCookie == nil || !sessionCookie.Secure || !sessionCookie.HttpOnly ||
		sessionCookie.SameSite != http.SameSiteStrictMode {
		t.Fatalf("session cookie = %#v", sessionCookie)
	}

	inventory := httptest.NewRequest(http.MethodGet, "/api/v1/devices", nil)
	inventory.AddCookie(sessionCookie)
	inventoryResponse := httptest.NewRecorder()
	server.ServeHTTP(inventoryResponse, inventory)
	if inventoryResponse.Code != http.StatusOK {
		t.Fatalf("inventory status = %d, body=%s",
			inventoryResponse.Code, inventoryResponse.Body.String())
	}
	if strings.Contains(inventoryResponse.Body.String(), "password") ||
		strings.Contains(inventoryResponse.Body.String(), "token") {
		t.Fatalf("inventory response exposes secret field: %s", inventoryResponse.Body.String())
	}
}

func TestLoginAcceptsSameOriginRefererWhenBrowserOmitsOrigin(t *testing.T) {
	server := newTestServer(t, false)
	body := `{"login_id":"operator","password":"現場担当者の 十分に長いパスワード"}`
	login := httptest.NewRequest(http.MethodPost, "/api/v1/session", strings.NewReader(body))
	login.Header.Set("Content-Type", "application/json")
	login.Header.Set("Referer", testOrigin+"/login")
	response := httptest.NewRecorder()

	server.ServeHTTP(response, login)

	if response.Code != http.StatusCreated {
		t.Fatalf("login status = %d, want 201; body=%s", response.Code, response.Body.String())
	}
}

func TestMutationRequiresMatchingOriginAndCSRFToken(t *testing.T) {
	server := newTestServer(t, false)
	sessionCookie, csrf := loginTestAccount(t, server)

	for _, test := range []struct {
		name    string
		origin  string
		referer string
		csrf    string
	}{
		{name: "missing origin", csrf: csrf},
		{name: "wrong origin", origin: "https://attacker.example", csrf: csrf},
		{name: "wrong referer", referer: "https://attacker.example/login", csrf: csrf},
		{
			name:    "wrong origin takes precedence over same-origin referer",
			origin:  "https://attacker.example",
			referer: testOrigin + "/login",
			csrf:    csrf,
		},
		{name: "missing csrf", origin: testOrigin},
		{name: "wrong csrf", origin: testOrigin, csrf: "wrong"},
	} {
		t.Run(test.name, func(t *testing.T) {
			request := httptest.NewRequest(http.MethodDelete, "/api/v1/session", nil)
			request.AddCookie(sessionCookie)
			if test.origin != "" {
				request.Header.Set("Origin", test.origin)
			}
			if test.referer != "" {
				request.Header.Set("Referer", test.referer)
			}
			if test.csrf != "" {
				request.Header.Set("X-CSRF-Token", test.csrf)
			}
			response := httptest.NewRecorder()
			server.ServeHTTP(response, request)
			if response.Code != http.StatusForbidden {
				t.Fatalf("status = %d, want 403; body=%s",
					response.Code, response.Body.String())
			}
		})
	}

	request := httptest.NewRequest(http.MethodDelete, "/api/v1/session", nil)
	request.AddCookie(sessionCookie)
	request.Header.Set("Origin", testOrigin)
	request.Header.Set("X-CSRF-Token", csrf)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusNoContent {
		t.Fatalf("valid logout status = %d, body=%s", response.Code, response.Body.String())
	}
}

func TestTemporaryPasswordSessionCanOnlyReadSessionAndChangePassword(t *testing.T) {
	server := newTestServer(t, true)
	sessionCookie, _ := loginTestAccount(t, server)

	request := httptest.NewRequest(http.MethodGet, "/api/v1/devices", nil)
	request.AddCookie(sessionCookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusForbidden {
		t.Fatalf("temporary-password inventory status = %d, want 403", response.Code)
	}

	request = httptest.NewRequest(http.MethodGet, "/api/v1/session", nil)
	request.AddCookie(sessionCookie)
	response = httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatalf("temporary-password session status = %d, want 200", response.Code)
	}
}

func TestViewerCannotChangeProfilesMeaningsOutputsOrAccounts(t *testing.T) {
	server := newTestServer(t, false)
	sessionCookie, csrf := loginTestAccount(t, server)
	for _, test := range []struct {
		method string
		path   string
		body   string
	}{
		{http.MethodPut, "/api/v1/devices/dev_00000000000000000000000000000000/profile",
			`{"display_name":"変更","location":"現場"}`},
		{http.MethodPut, "/api/v1/signals/sig_00000000000000000000000000000000/semantic-definition",
			`{"kind":"numeric","scale":1,"offset":0,"condition":{"mode":"","bool_value":false,"threshold":0,"hysteresis":0},"trigger":""}`},
		{http.MethodPost, "/api/v1/outputs/yokakit",
			`{"definition_id":"sem_example","source_id":"iotkit-01","signal_id":"x","kind":"onoff","reason":""}`},
		{http.MethodPost, "/api/v1/accounts",
			`{"login_id":"other","display_name":"別担当","role":"viewer","temporary_password":"十分に長い 仮パスワード です"}`},
	} {
		request := httptest.NewRequest(test.method, test.path, strings.NewReader(test.body))
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("Origin", testOrigin)
		request.Header.Set("X-CSRF-Token", csrf)
		request.AddCookie(sessionCookie)
		response := httptest.NewRecorder()
		server.ServeHTTP(response, request)
		if response.Code != http.StatusForbidden {
			t.Fatalf("%s %s status=%d body=%s",
				test.method, test.path, response.Code, response.Body.String())
		}
	}
}

func TestSetupAPIRestrictsPhysicalIdentifierToAdministrators(t *testing.T) {
	viewerServer, viewerStore := newTestServerFixture(
		t, false, siteapp.AccountRoleViewer,
	)
	seedSetupDevice(t, viewerStore)
	viewerCookie, _ := loginTestAccount(t, viewerServer)
	viewerRequest := httptest.NewRequest(http.MethodGet, "/api/v1/setup/devices", nil)
	viewerRequest.AddCookie(viewerCookie)
	viewerResponse := httptest.NewRecorder()
	viewerServer.ServeHTTP(viewerResponse, viewerRequest)
	if viewerResponse.Code != http.StatusForbidden {
		t.Fatalf("viewer setup status = %d, want 403; body=%s",
			viewerResponse.Code, viewerResponse.Body.String())
	}

	adminServer, adminStore := newTestServerFixture(
		t, false, siteapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, adminStore)
	adminCookie, _ := loginTestAccount(t, adminServer)
	adminRequest := httptest.NewRequest(http.MethodGet, "/api/v1/setup/devices", nil)
	adminRequest.AddCookie(adminCookie)
	adminResponse := httptest.NewRecorder()
	adminServer.ServeHTTP(adminResponse, adminRequest)
	if adminResponse.Code != http.StatusOK {
		t.Fatalf("admin setup status = %d, body=%s",
			adminResponse.Code, adminResponse.Body.String())
	}
	body := adminResponse.Body.String()
	for _, want := range []string{
		`"identifier":"BP-01234567"`,
		`"display_sensor_type":"temperature"`,
		`"profile_complete":false`,
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("setup response missing %q: %s", want, body)
		}
	}
	for _, forbidden := range []string{"018f0000-0000-7000-8000-000000000001", "series_key"} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("setup response exposes %q: %s", forbidden, body)
		}
	}
}

func TestSignalProfileV2APIStoresDisplayMetadata(t *testing.T) {
	server, archive := newTestServerFixture(t, false, siteapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err=%v", signals, err)
	}
	sessionCookie, csrf := loginTestAccount(t, server)
	body := `{
		"display_name":"乾燥炉入口温度",
		"display_sensor_type":"temperature",
		"display_sensor_type_label":"",
		"display_value_kind":"numeric",
		"display_unit_mode":"unit",
		"display_unit":"°C",
		"decimal_places":1
	}`
	request := httptest.NewRequest(
		http.MethodPut,
		"/api/v1/signals/"+signals[0].SignalRef+"/profile",
		strings.NewReader(body),
	)
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Origin", testOrigin)
	request.Header.Set("X-CSRF-Token", csrf)
	request.Header.Set("If-Match", "*")
	request.AddCookie(sessionCookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatalf("profile status = %d, body=%s", response.Code, response.Body.String())
	}
	if !strings.Contains(response.Body.String(), `"display_unit":"°C"`) ||
		!strings.Contains(response.Body.String(), `"decimal_places":1`) {
		t.Fatalf("profile response = %s", response.Body.String())
	}
}

func TestConsoleRequiresLoginAndUsesJapaneseOperatorLanguage(t *testing.T) {
	server := newTestServer(t, false)
	request := httptest.NewRequest(http.MethodGet, "/status", nil)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusSeeOther ||
		response.Header().Get("Location") != "/login" {
		t.Fatalf("anonymous console response = %d, location=%q",
			response.Code, response.Header().Get("Location"))
	}

	loginPage := httptest.NewRequest(http.MethodGet, "/login", nil)
	loginPageResponse := httptest.NewRecorder()
	server.ServeHTTP(loginPageResponse, loginPage)
	if loginPageResponse.Code != http.StatusOK ||
		!strings.Contains(loginPageResponse.Body.String(), "IoTKitへログイン") {
		t.Fatalf("login page = %d, body=%s",
			loginPageResponse.Code, loginPageResponse.Body.String())
	}
	for _, internal := range []string{"series_key", "ledger_epoch", "active_edge"} {
		if strings.Contains(loginPageResponse.Body.String(), internal) {
			t.Fatalf("login page exposes internal term %q", internal)
		}
	}
}

func TestConsoleLoginFormCreatesSessionAndRedirectsToStatus(t *testing.T) {
	server := newTestServer(t, false)
	form := url.Values{}
	form.Set("login_id", "operator")
	form.Set("password", "現場担当者の 十分に長いパスワード")
	request := httptest.NewRequest(
		http.MethodPost,
		"/login",
		strings.NewReader(form.Encode()),
	)
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	request.Header.Set("Origin", testOrigin)
	response := httptest.NewRecorder()

	server.ServeHTTP(response, request)

	if response.Code != http.StatusSeeOther ||
		response.Header().Get("Location") != "/status" {
		t.Fatalf("login form response = %d, location=%q, body=%s",
			response.Code, response.Header().Get("Location"), response.Body.String())
	}
	var sessionCookie *http.Cookie
	for _, cookie := range response.Result().Cookies() {
		if cookie.Name == sessionCookieName {
			sessionCookie = cookie
		}
	}
	if sessionCookie == nil {
		t.Fatal("login form did not set session cookie")
	}
	statusRequest := httptest.NewRequest(http.MethodGet, "/status", nil)
	statusRequest.AddCookie(sessionCookie)
	statusResponse := httptest.NewRecorder()
	server.ServeHTTP(statusResponse, statusRequest)
	if statusResponse.Code != http.StatusOK ||
		!strings.Contains(statusResponse.Body.String(), "第一工場 担当者") {
		t.Fatalf("status page = %d, body=%s",
			statusResponse.Code, statusResponse.Body.String())
	}
}

func TestConsoleUsesOperatorFocusedInformationArchitecture(t *testing.T) {
	server := newTestServer(t, false)
	sessionCookie, _ := loginTestAccount(t, server)

	tests := []struct {
		path        string
		pageTitle   string
		activeLabel string
		mustContain []string
	}{
		{
			path:        "/status",
			pageTitle:   "現場の概要",
			activeLabel: "概要",
			mustContain: []string{"センサーの現在値", "要確認", "データの流れ"},
		},
		{
			path:        "/monitor",
			pageTitle:   "センサーの現在値",
			activeLabel: "センサー",
			mustContain: []string{
				"<th>センサー</th>", "<th>種類</th>", "<th>現在値</th>",
				"受信状態", "最終受信",
			},
		},
		{
			path:        "/equipment",
			pageTitle:   "機器管理",
			activeLabel: "機器管理",
			mustContain: []string{"確認や設定を行うEdgeを選んでください", "閲覧のみ"},
		},
		{
			path:        "/setup",
			pageTitle:   "デバイス管理",
			activeLabel: "",
			mustContain: []string{"デバイスごとに", "閲覧のみ"},
		},
		{
			path:        "/signals",
			pageTitle:   "値の変換",
			activeLabel: "値の変換",
			mustContain: []string{"数値・状態・累積値・アラーム", "閲覧のみ"},
		},
	}
	for _, test := range tests {
		t.Run(test.path, func(t *testing.T) {
			request := httptest.NewRequest(http.MethodGet, test.path, nil)
			request.AddCookie(sessionCookie)
			response := httptest.NewRecorder()

			server.ServeHTTP(response, request)

			if response.Code != http.StatusOK {
				t.Fatalf("status = %d, body=%s", response.Code, response.Body.String())
			}
			body := response.Body.String()
			if !strings.Contains(body, "<h1>"+test.pageTitle+"</h1>") {
				t.Fatalf("page does not use title %q: %s", test.pageTitle, body)
			}
			if test.activeLabel != "" &&
				(!strings.Contains(body, `aria-current="page"`) ||
					!strings.Contains(body, `</span>`+test.activeLabel)) {
				t.Fatalf("page does not mark %q as active navigation", test.activeLabel)
			}
			for _, text := range test.mustContain {
				if !strings.Contains(body, text) {
					t.Fatalf("page does not contain %q", text)
				}
			}
		})
	}
}

func TestConsoleCallsSignalsSensorsAndKeepsDevicesDistinct(t *testing.T) {
	server := newTestServer(t, false)
	sessionCookie, _ := loginTestAccount(t, server)
	for _, path := range []string{
		"/status", "/monitor", "/equipment", "/setup", "/signals", "/logs", "/output", "/system",
	} {
		t.Run(path, func(t *testing.T) {
			request := httptest.NewRequest(http.MethodGet, path, nil)
			request.AddCookie(sessionCookie)
			response := httptest.NewRecorder()
			server.ServeHTTP(response, request)
			if response.Code != http.StatusOK {
				t.Fatalf("status=%d body=%s", response.Code, response.Body.String())
			}
			body := response.Body.String()
			if strings.Contains(body, "信号") {
				t.Fatalf("%s exposes signal term: %s", path, body)
			}
		})
	}

	request := httptest.NewRequest(http.MethodGet, "/setup", nil)
	request.AddCookie(sessionCookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if !strings.Contains(response.Body.String(), "デバイス管理") {
		t.Fatalf("setup does not preserve device terminology: %s", response.Body.String())
	}

	request = httptest.NewRequest(http.MethodGet, "/monitor", nil)
	request.AddCookie(sessionCookie)
	response = httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if strings.Contains(response.Body.String(), "<th>値</th>") {
		t.Fatalf("monitor incorrectly splits a sensor into a separate value entity: %s",
			response.Body.String())
	}
}

func TestSetupConsoleShowsLiveFactsAndLimitsPhysicalIdentifier(t *testing.T) {
	viewerServer, viewerStore := newTestServerFixture(
		t, false, siteapp.AccountRoleViewer,
	)
	seedSetupDevice(t, viewerStore)
	viewerCookie, _ := loginTestAccount(t, viewerServer)
	viewerRequest := httptest.NewRequest(http.MethodGet, "/setup", nil)
	viewerRequest.AddCookie(viewerCookie)
	viewerResponse := httptest.NewRecorder()
	viewerServer.ServeHTTP(viewerResponse, viewerRequest)
	if viewerResponse.Code != http.StatusOK {
		t.Fatalf("viewer setup status=%d body=%s",
			viewerResponse.Code, viewerResponse.Body.String())
	}
	viewerBody := viewerResponse.Body.String()
	for _, want := range []string{
		"デバイス管理",
		"24.8",
		"temperature_c",
		"Adapterから届いた情報",
		"閲覧のみ",
	} {
		if !strings.Contains(viewerBody, want) {
			t.Fatalf("viewer setup missing %q: %s", want, viewerBody)
		}
	}
	if strings.Contains(viewerBody, "BP-01234567") ||
		strings.Contains(viewerBody, `name="display_sensor_type"`) {
		t.Fatalf("viewer setup exposes admin controls or identifier: %s", viewerBody)
	}

	adminServer, adminStore := newTestServerFixture(
		t, false, siteapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, adminStore)
	adminCookie, _ := loginTestAccount(t, adminServer)
	adminRequest := httptest.NewRequest(http.MethodGet, "/setup", nil)
	adminRequest.AddCookie(adminCookie)
	adminResponse := httptest.NewRecorder()
	adminServer.ServeHTTP(adminResponse, adminRequest)
	if adminResponse.Code != http.StatusOK {
		t.Fatalf("admin setup status=%d body=%s",
			adminResponse.Code, adminResponse.Body.String())
	}
	adminBody := adminResponse.Body.String()
	for _, want := range []string{
		"BP-01234567",
		`name="display_sensor_type"`,
		`name="display_value_kind"`,
		`name="display_unit_mode"`,
		`name="decimal_places"`,
	} {
		if !strings.Contains(adminBody, want) {
			t.Fatalf("admin setup missing %q: %s", want, adminBody)
		}
	}
}

func TestStatusHighlightsNewDeviceWithoutCallingItBroken(t *testing.T) {
	server, archive := newTestServerFixture(t, false, siteapp.AccountRoleViewer)
	seedSetupDevice(t, archive)
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(http.MethodGet, "/status", nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	body := response.Body.String()
	for _, want := range []string{
		"新しいデバイスが見つかりました",
		"機器管理",
		`href="/equipment"`,
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("status missing %q: %s", want, body)
		}
	}
	if strings.Contains(body, "大きな問題は見つかっていません") {
		t.Fatalf("status contradicts pending setup: %s", body)
	}
}

func TestDeviceStopsBeingRegistrationPendingAfterDeviceProfileSave(t *testing.T) {
	server, archive := newTestServerFixture(t, false, siteapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	devices, err := archive.ListInventoryDevices(context.Background(), 10, "")
	if err != nil || len(devices) != 1 {
		t.Fatalf("devices=%#v err=%v", devices, err)
	}
	cookie, csrf := loginTestAccount(t, server)
	form := url.Values{
		"_csrf":        {csrf},
		"return_to":    {"/setup"},
		"display_name": {"BravePI Mainboard 1"},
		"location":     {"第1工場 乾燥工程"},
	}
	saveRequest := httptest.NewRequest(
		http.MethodPost,
		"/console/devices/"+devices[0].DeviceRef+"/profile",
		strings.NewReader(form.Encode()),
	)
	saveRequest.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	saveRequest.Header.Set("Origin", testOrigin)
	saveRequest.AddCookie(cookie)
	saveResponse := httptest.NewRecorder()
	server.ServeHTTP(saveResponse, saveRequest)
	if saveResponse.Code != http.StatusSeeOther {
		t.Fatalf("device save=%d body=%s", saveResponse.Code, saveResponse.Body.String())
	}

	setupRequest := httptest.NewRequest(http.MethodGet, "/setup", nil)
	setupRequest.AddCookie(cookie)
	setupResponse := httptest.NewRecorder()
	server.ServeHTTP(setupResponse, setupRequest)
	setupBody := setupResponse.Body.String()
	for _, want := range []string{
		"0<small>台のデバイスが登録待ち",
		"センサーを設定",
		"確認して保存",
	} {
		if !strings.Contains(setupBody, want) {
			t.Fatalf("setup missing %q after device save: %s", want, setupBody)
		}
	}

	statusRequest := httptest.NewRequest(http.MethodGet, "/status", nil)
	statusRequest.AddCookie(cookie)
	statusResponse := httptest.NewRecorder()
	server.ServeHTTP(statusResponse, statusRequest)
	statusBody := statusResponse.Body.String()
	if strings.Contains(statusBody, "新しいデバイスが見つかりました") {
		t.Fatalf("saved device is still reported as new: %s", statusBody)
	}
}

func TestSetupConsoleSavesProfilesAndUpdatesMonitor(t *testing.T) {
	server, archive := newTestServerFixture(t, false, siteapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	devices, err := archive.ListInventoryDevices(context.Background(), 10, "")
	if err != nil || len(devices) != 1 {
		t.Fatalf("devices=%#v err=%v", devices, err)
	}
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	cookie, csrf := loginTestAccount(t, server)
	postForm := func(path string, values url.Values) *httptest.ResponseRecorder {
		t.Helper()
		values.Set("_csrf", csrf)
		values.Set("return_to", "/setup")
		request := httptest.NewRequest(
			http.MethodPost,
			path,
			strings.NewReader(values.Encode()),
		)
		request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		request.Header.Set("Origin", testOrigin)
		request.AddCookie(cookie)
		response := httptest.NewRecorder()
		server.ServeHTTP(response, request)
		return response
	}
	deviceResponse := postForm(
		"/console/devices/"+devices[0].DeviceRef+"/profile",
		url.Values{
			"display_name": {"乾燥炉入口センサー"},
			"location":     {"第1工場 乾燥工程"},
		},
	)
	if deviceResponse.Code != http.StatusSeeOther ||
		deviceResponse.Header().Get("Location") != "/setup?saved=1" {
		t.Fatalf("device save=%d location=%q body=%s",
			deviceResponse.Code, deviceResponse.Header().Get("Location"),
			deviceResponse.Body.String())
	}
	signalResponse := postForm(
		"/console/signals/"+signals[0].SignalRef+"/profile",
		url.Values{
			"display_name":        {"乾燥炉入口 温度"},
			"display_sensor_type": {"temperature"},
			"display_value_kind":  {"numeric"},
			"display_unit_mode":   {"unit"},
			"display_unit":        {"°C"},
			"decimal_places":      {"2"},
		},
	)
	if signalResponse.Code != http.StatusSeeOther ||
		signalResponse.Header().Get("Location") != "/setup?saved=1" {
		t.Fatalf("signal save=%d location=%q body=%s",
			signalResponse.Code, signalResponse.Header().Get("Location"),
			signalResponse.Body.String())
	}
	setupRequest := httptest.NewRequest(http.MethodGet, "/setup", nil)
	setupRequest.AddCookie(cookie)
	setupResponse := httptest.NewRecorder()
	server.ServeHTTP(setupResponse, setupRequest)
	if !strings.Contains(setupResponse.Body.String(), "登録済み") {
		t.Fatalf("setup did not become ready: %s", setupResponse.Body.String())
	}
	monitorRequest := httptest.NewRequest(http.MethodGet, "/monitor", nil)
	monitorRequest.AddCookie(cookie)
	monitorResponse := httptest.NewRecorder()
	server.ServeHTTP(monitorResponse, monitorRequest)
	for _, want := range []string{"乾燥炉入口 温度", "24.80", "°C"} {
		if !strings.Contains(monitorResponse.Body.String(), want) {
			t.Fatalf("monitor missing %q: %s", want, monitorResponse.Body.String())
		}
	}
}

func TestConsoleServesOfficialBrandMark(t *testing.T) {
	server := newTestServer(t, false)
	request := httptest.NewRequest(http.MethodGet, "/static/pinkietech-mark.svg", nil)
	response := httptest.NewRecorder()

	server.ServeHTTP(response, request)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", response.Code, response.Body.String())
	}
	if response.Header().Get("Content-Type") != "image/svg+xml" ||
		!strings.Contains(response.Body.String(), `viewBox="0 0 296 297"`) {
		t.Fatalf("brand mark response headers=%v body=%s",
			response.Header(), response.Body.String())
	}
}

func TestConsoleSignalViewFormatsValuesForOperators(t *testing.T) {
	now := time.Date(2026, 7, 18, 9, 0, 0, 0, time.Local)
	unit := "Cel"
	valueType := "float"
	sensorType := "temperature_c"
	receivedAt := now.Add(-2 * time.Minute).UnixMilli()
	temperature := newConsoleSignalView(siteapp.SignalSummary{
		Edge:        "factory-edge-01",
		DisplayName: "乾燥炉 温度",
		Unit:        &unit,
		ValueType:   &valueType,
		SensorType:  &sensorType,
		Latest: &siteapp.LatestMeasurement{
			Values:         json.RawMessage(`[24.900000000000002]`),
			SiteReceivedAt: receivedAt,
		},
		LastReceivedAt: &receivedAt,
		ReceiptStatus:  "receiving",
	}, now)
	if temperature.Value != "24.9" || temperature.Unit != "°C" ||
		temperature.SensorType != "温度" || temperature.LastReceived != "2分前" ||
		temperature.StatusLabel != "受信中" {
		t.Fatalf("temperature view = %#v", temperature)
	}

	boolType := "bool"
	contactType := "contact_state"
	contact := newConsoleSignalView(siteapp.SignalSummary{
		ValueType:  &boolType,
		SensorType: &contactType,
		Latest:     &siteapp.LatestMeasurement{Values: json.RawMessage(`[1]`)},
	}, now)
	if contact.Value != "ON" || contact.SensorType != "接点入力" {
		t.Fatalf("contact view = %#v", contact)
	}
}

func TestConsoleSignalViewUsesCompletedProfileForEffectiveDisplay(t *testing.T) {
	now := time.Date(2026, 7, 18, 9, 0, 0, 0, time.Local)
	descriptorUnit := "Cel"
	descriptorType := "temperature_c"
	descriptorValueType := "float"
	view := newConsoleSignalView(siteapp.SignalSummary{
		DisplayName: "電圧入力",
		Unit:        &descriptorUnit,
		SensorType:  &descriptorType,
		ValueType:   &descriptorValueType,
		Profile: &siteapp.SignalProfile{
			DisplayName:       "電圧入力",
			DisplaySensorType: "voltage",
			DisplayValueKind:  "numeric",
			DisplayUnitMode:   "unit",
			DisplayUnit:       "V",
			DecimalPlaces:     2,
			Revision:          1,
		},
		Latest: &siteapp.LatestMeasurement{Values: json.RawMessage(`[24.8]`)},
	}, now)
	if view.Value != "24.80" || view.Unit != "V" ||
		view.SensorType != "電圧" || view.SettingLabel != "設定済み" {
		t.Fatalf("profile display view = %#v", view)
	}
}

func TestConsoleLogViewUsesSignalNameAndUnit(t *testing.T) {
	unit := "Cel"
	valueType := "float"
	seriesKey := "device-01:temperature_c:na:primary"
	views := newConsoleLogViews(
		[]store.RawRecord{{
			EdgeNodeID: "factory-edge-01",
			Record: json.RawMessage(
				`{"series_key":"device-01:temperature_c:na:primary","values":[24.8]}`,
			),
		}},
		[]consoleSignalView{{
			SignalSummary: siteapp.SignalSummary{
				SeriesKey: seriesKey,
				Unit:      &unit,
				ValueType: &valueType,
			},
			Name: "乾燥炉 温度",
			Unit: "°C",
		}},
	)
	if len(views) != 1 || views[0].Sensor != "乾燥炉 温度" ||
		views[0].Value != "24.8" || views[0].Unit != "°C" {
		t.Fatalf("log views = %#v", views)
	}
}

func TestSystemAdministratorCanSeeAccountMaintenanceActions(t *testing.T) {
	server := newTestServerWithRole(t, false, siteapp.AccountRoleSystemAdmin)
	sessionCookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(http.MethodGet, "/accounts", nil)
	request.AddCookie(sessionCookie)
	response := httptest.NewRecorder()

	server.ServeHTTP(response, request)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", response.Code, response.Body.String())
	}
	for _, text := range []string{
		"名前と権限を変更",
		"仮パスワードを再発行",
		"アカウントを無効にする",
	} {
		if !strings.Contains(response.Body.String(), text) {
			t.Fatalf("account page does not contain %q", text)
		}
	}
}

func TestConsoleLogoutRevokesSession(t *testing.T) {
	server := newTestServer(t, false)
	sessionCookie, csrf := loginTestAccount(t, server)
	form := url.Values{}
	form.Set("_csrf", csrf)
	request := httptest.NewRequest(http.MethodPost, "/logout", strings.NewReader(form.Encode()))
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	request.Header.Set("Origin", testOrigin)
	request.AddCookie(sessionCookie)
	response := httptest.NewRecorder()

	server.ServeHTTP(response, request)

	if response.Code != http.StatusSeeOther || response.Header().Get("Location") != "/login" {
		t.Fatalf("logout response = %d, location=%q, body=%s",
			response.Code, response.Header().Get("Location"), response.Body.String())
	}
	status := httptest.NewRequest(http.MethodGet, "/status", nil)
	status.AddCookie(sessionCookie)
	statusResponse := httptest.NewRecorder()
	server.ServeHTTP(statusResponse, status)
	if statusResponse.Code != http.StatusSeeOther ||
		statusResponse.Header().Get("Location") != "/login" {
		t.Fatalf("revoked session status = %d, location=%q",
			statusResponse.Code, statusResponse.Header().Get("Location"))
	}
}

func TestConsoleOutputOptionsUseOperatorNamesInsteadOfResourceRefs(t *testing.T) {
	options := newConsoleDefinitionOptions(
		[]semantics.Definition{{
			ID:        "sem_example",
			SignalRef: "sig_00000000000000000000000000000001",
			DefinitionSpec: semantics.DefinitionSpec{
				Kind: semantics.KindCumulativeCounter,
			},
			Active: true,
		}},
		[]consoleSignalView{{
			SignalSummary: siteapp.SignalSummary{
				SignalRef: "sig_00000000000000000000000000000001",
			},
			Name: "ライン1 完了",
		}},
	)
	if len(options) != 1 || options[0].Name != "ライン1 完了" ||
		options[0].Kind != "累積値" ||
		strings.Contains(options[0].Name, "sig_") {
		t.Fatalf("options = %#v", options)
	}
}

func TestConsoleOutputRowsUseOperatorLanguage(t *testing.T) {
	rows := newConsoleOutputViews([]store.YokaKitRoute{{
		SourceID: "line-1", SignalID: "completed-count",
		Kind: outputadapter.YokaKitProduction, Active: true,
		PendingCount: 2, PublishedCount: 41,
	}})

	if len(rows) != 1 || rows[0].KindLabel != "生産の累積値" ||
		rows[0].StateLabel != "使用中" || rows[0].StateClass != "receiving" {
		t.Fatalf("rows = %#v", rows)
	}
}

func TestConsoleAuditViewsExplainAdministrativeChanges(t *testing.T) {
	views := newConsoleAuditViews([]siteapp.AuditEvent{{
		OccurredAt:  1_752_800_000_000,
		ActorClass:  siteapp.ActorSettingsSession,
		Operation:   "account.password_replace",
		ResourceRef: "acct_00000000000000000000000000000001",
		Outcome:     "success",
	}})

	if len(views) != 1 || views[0].Operation != "パスワードを変更" ||
		views[0].Resource != "アカウント" || views[0].Outcome != "完了" {
		t.Fatalf("views = %#v", views)
	}
}

func TestConsoleChangeHistoryOmitsLoginNoiseAndNamesSemanticChanges(t *testing.T) {
	views := newConsoleAuditViews([]siteapp.AuditEvent{
		{Operation: "session.login", ResourceRef: "site", Outcome: "success"},
		{
			Operation:   "semantic_definition.put",
			ResourceRef: "sem_00000000000000000000000000000001",
			Outcome:     "success",
		},
	})

	if len(views) != 1 || views[0].Operation != "センサー設定を保存" ||
		views[0].Resource != "センサー設定" {
		t.Fatalf("views = %#v", views)
	}
}

func newTestServer(t *testing.T, mustChangePassword bool) http.Handler {
	return newTestServerWithRole(t, mustChangePassword, siteapp.AccountRoleViewer)
}

func newTestServerWithRole(
	t *testing.T,
	mustChangePassword bool,
	role siteapp.AccountRole,
) http.Handler {
	server, _ := newTestServerFixture(t, mustChangePassword, role)
	return server
}

func newTestServerFixture(
	t *testing.T,
	mustChangePassword bool,
	role siteapp.AccountRole,
) (http.Handler, *store.Store) {
	t.Helper()
	archive, err := store.Open(filepath.Join(t.TempDir(), "site.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = archive.Close() })
	passwordPHC, err := siteauth.HashPassword("現場担当者の 十分に長いパスワード")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.CreateSiteAccount(
		context.Background(),
		siteapp.LocalCLIActor(),
		siteapp.AccountProvision{
			LoginID:            "operator",
			DisplayName:        "第一工場 担当者",
			Role:               role,
			PasswordPHC:        passwordPHC,
			MustChangePassword: mustChangePassword,
		},
	); err != nil {
		t.Fatal(err)
	}
	sessions, err := sitesession.NewManager(archive, sitesession.Options{
		Delay: func(context.Context, time.Duration) error { return nil },
	})
	if err != nil {
		t.Fatal(err)
	}
	handler, err := New(Config{
		Store:        archive,
		Site:         siteapp.NewService(archive),
		Accounts:     siteapp.NewAccountService(archive),
		Sessions:     sessions,
		PublicOrigin: testOrigin,
	})
	if err != nil {
		t.Fatal(err)
	}
	return handler, archive
}

func seedDiscoveredEdge(t *testing.T, archive *store.Store) siteapp.Edge {
	t.Helper()
	snapshot := contract.DescriptorSnapshot{
		SchemaVersion:      1,
		EdgeNodeID:         "factory-edge-01",
		LedgerEpoch:        "epoch-01",
		DescriptorRevision: 1,
		Complete:           true,
		Devices: []contract.DescriptorDevice{{
			SystemID: "018f0000-0000-7000-8000-000000000001",
			State:    "active",
		}},
		Signals: []contract.DescriptorSignal{{
			SeriesKey:      "018f0000-0000-7000-8000-000000000001:temperature_c:na:primary",
			SystemID:       "018f0000-0000-7000-8000-000000000001",
			MeasurementKey: "temperature_c",
			Variant:        "primary",
			ValueType:      "float",
		}},
	}
	if _, err := archive.ApplyDescriptorSnapshot(
		context.Background(), snapshot,
	); err != nil {
		t.Fatal(err)
	}
	edges, err := archive.ListEdges(context.Background())
	if err != nil || len(edges) != 1 {
		t.Fatalf("edges = %#v, err = %v", edges, err)
	}
	return edges[0]
}

func seedAdditionalDiscoveredEdge(t *testing.T, archive *store.Store) siteapp.Edge {
	t.Helper()
	const systemID = "018f0000-0000-7000-8000-000000000002"
	identifier := "BP-87654321"
	unit := "1"
	if _, err := archive.ApplyDescriptorSnapshot(
		context.Background(),
		contract.DescriptorSnapshot{
			SchemaVersion:      1,
			EdgeNodeID:         "assembly-edge-02",
			LedgerEpoch:        "epoch-02",
			DescriptorRevision: 1,
			Complete:           true,
			Devices: []contract.DescriptorDevice{{
				SystemID:   systemID,
				Identifier: &identifier,
				State:      "active",
			}},
			Signals: []contract.DescriptorSignal{{
				SeriesKey:      systemID + ":contact_state:na:primary",
				SystemID:       systemID,
				MeasurementKey: "contact_state",
				Variant:        "primary",
				Unit:           &unit,
				ValueType:      "bool",
			}},
		},
	); err != nil {
		t.Fatal(err)
	}
	edges, err := archive.ListEdges(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	for _, edge := range edges {
		if edge.EdgeNodeID == "assembly-edge-02" {
			return edge
		}
	}
	t.Fatal("additional discovered Edge was not listed")
	return siteapp.Edge{}
}

func seedSetupDevice(t *testing.T, archive *store.Store) {
	t.Helper()
	const (
		systemID  = "018f0000-0000-7000-8000-000000000001"
		seriesKey = systemID + ":temperature_c:na:primary"
	)
	identifier := "BP-01234567"
	unit := "Cel"
	snapshot := contract.DescriptorSnapshot{
		SchemaVersion:      1,
		EdgeNodeID:         "factory-edge-01",
		LedgerEpoch:        "epoch-01",
		DescriptorRevision: 1,
		Complete:           true,
		Devices: []contract.DescriptorDevice{{
			SystemID:   systemID,
			Identifier: &identifier,
			State:      "active",
		}},
		Signals: []contract.DescriptorSignal{{
			SeriesKey:      seriesKey,
			SystemID:       systemID,
			MeasurementKey: "temperature_c",
			Variant:        "primary",
			Unit:           &unit,
			ValueType:      "float",
		}},
	}
	if _, err := archive.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	edges, err := archive.ListEdges(context.Background())
	if err != nil || len(edges) != 1 {
		t.Fatalf("edges = %#v, err = %v", edges, err)
	}
	expected := edges[0].Revision
	if _, err := archive.RequestEdgeActivation(
		context.Background(),
		siteapp.LocalCLIActor(),
		edges[0].EdgeRef,
		siteapp.RevisionPrecondition{Expected: &expected},
	); err != nil {
		t.Fatal(err)
	}
	commands, err := archive.ListPendingActivationCommands(context.Background(), 1)
	if err != nil || len(commands) != 1 {
		t.Fatalf("activation commands = %#v, err = %v", commands, err)
	}
	activationRequest, err := contract.DecodeActivationRequest(commands[0].PayloadJSON)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ApplyActivationResult(
		context.Background(),
		contract.ActivationResult{
			SchemaVersion:            contract.SchemaVersion,
			ActivationID:             activationRequest.ActivationID,
			SiteID:                   activationRequest.SiteID,
			EdgeNodeID:               activationRequest.EdgeNodeID,
			LedgerEpoch:              activationRequest.ExpectedLedgerEpoch,
			Status:                   "applied",
			DiscardThroughReadingSeq: 0,
			FirstPublicationSeq:      1,
			AppliedAt:                activationRequest.IssuedAt + 1,
		},
	); err != nil {
		t.Fatal(err)
	}
	record := json.RawMessage(
		`{"family":"measurement","schema_version":1,"epoch":"epoch-01","pub_seq":1,` +
			`"series_key":"` + seriesKey + `","values":[24.8],"event_time":1000}`,
	)
	batch := contract.RecordBatch{
		SchemaVersion: 1,
		EdgeNodeID:    "factory-edge-01",
		LedgerEpoch:   "epoch-01",
		PublicationID: contract.PublicationID("factory-edge-01", "epoch-01", 1, 1),
		CursorStart:   1,
		CursorEnd:     1,
		Records:       []json.RawMessage{record},
	}
	if _, err := archive.AcceptBatch(context.Background(), batch); err != nil {
		t.Fatal(err)
	}
	if processed, err := archive.ReconcileInventorySources(context.Background(), 100); err != nil ||
		processed != 1 {
		t.Fatalf("reconcile processed=%d err=%v", processed, err)
	}
}

func loginTestAccount(t *testing.T, server http.Handler) (*http.Cookie, string) {
	t.Helper()
	body := bytes.NewBufferString(
		`{"login_id":"operator","password":"現場担当者の 十分に長いパスワード"}`,
	)
	request := httptest.NewRequest(http.MethodPost, "/api/v1/session", body)
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Origin", testOrigin)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusCreated {
		t.Fatalf("login status = %d, body=%s", response.Code, response.Body.String())
	}
	var payload struct {
		CSRFToken string `json:"csrf_token"`
	}
	if err := json.Unmarshal(response.Body.Bytes(), &payload); err != nil {
		t.Fatal(err)
	}
	for _, cookie := range response.Result().Cookies() {
		if cookie.Name == sessionCookieName {
			return cookie, payload.CSRFToken
		}
	}
	t.Fatal("login response did not set the session cookie")
	return nil, ""
}
