package edgehttp

import (
	"bytes"
	"context"
	"encoding/json"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"strconv"
	"strings"
	"testing"
	"time"
)

func TestEdgesConsoleShowsEdgeAsParentOfDevices(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleAdmin,
	)
	edgeNode := seedDiscoveredEdge(t, archive)
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(http.MethodGet, "/edge-nodes", nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", response.Code, response.Body.String())
	}
	body := response.Body.String()
	for _, want := range []string{
		"収集ノード管理",
		"未登録",
		"1台のデバイス",
		"1件のセンサー",
		"最終通信",
		edgeNode.LedgerEpoch,
		"収集ノードを登録",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("EdgeNode console missing %q: %s", want, body)
		}
	}
	for _, forbidden := range []string{
		"activation_id",
		"request_json",
		"result_json",
		"接続中",
	} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("EdgeNode console exposes forbidden term %q", forbidden)
		}
	}
}

func TestConsoleEquipmentViewsGroupDevicesUnderTheirEdge(t *testing.T) {
	now := time.Date(2026, 7, 18, 12, 0, 0, 0, time.UTC)
	edgeNodes := []edgeapp.EdgeNode{
		{EdgeNodeID: "edge-a", State: edgeapp.EdgeNodeActive},
		{EdgeNodeID: "edge-b", State: edgeapp.EdgeNodeDiscovered},
	}
	devices := []edgeapp.SetupDevice{{
		Device: edgeapp.DeviceSummary{EdgeNodeID: "edge-a", DeviceRef: "dev_a"},
		State:  edgeapp.SetupWaitingForDevice,
		Signals: []edgeapp.SetupSignal{{
			Signal:          edgeapp.SignalSummary{SignalRef: "sig_a"},
			ProfileComplete: false,
		}},
	}}

	rows := newConsoleEquipmentEdgeNodeViews(edgeNodes, devices, now)

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
	edgeNodes := []edgeapp.EdgeNode{{EdgeNodeID: "edge-a", State: edgeapp.EdgeNodeActive}}
	devices := []edgeapp.SetupDevice{
		{Device: edgeapp.DeviceSummary{EdgeNodeID: "edge-a", DeviceRef: "dev_a"}},
		{Device: edgeapp.DeviceSummary{EdgeNodeID: "missing-edge-node", DeviceRef: "dev_orphan"}},
	}

	rows := newConsoleOrphanDeviceViews(edgeNodes, devices, now)

	if len(rows) != 1 || rows[0].Device.DeviceRef != "dev_orphan" {
		t.Fatalf("orphan device rows = %#v", rows)
	}
}

func TestEquipmentDetailRoutesResolveKnownResources(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	cookie, _ := loginTestAccount(t, server)
	edgeNodes, err := archive.ListEdgeNodes(context.Background())
	if err != nil || len(edgeNodes) != 1 {
		t.Fatalf("edgeNodes = %#v, err = %v", edgeNodes, err)
	}
	devices, err := archive.ListInventoryDevices(context.Background(), 100, "")
	if err != nil || len(devices) != 1 {
		t.Fatalf("devices = %#v, err = %v", devices, err)
	}

	for _, path := range []string{
		"/equipment/edge-nodes/" + edgeNodes[0].EdgeNodeRef,
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
		t, false, edgeapp.AccountRoleAdmin,
	)
	seedDiscoveredEdge(t, archive)
	cookie, _ := loginTestAccount(t, server)
	for _, path := range []string{
		"/equipment/edge-nodes/edge_node_00000000000000000000000000000000",
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
		t, false, edgeapp.AccountRoleViewer,
	)
	seedSetupDevice(t, archive)
	edgeNodes, err := archive.ListEdgeNodes(context.Background())
	if err != nil || len(edgeNodes) != 1 {
		t.Fatalf("edgeNodes = %#v, err = %v", edgeNodes, err)
	}
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodGet,
		"/equipment/edge-nodes/"+edgeNodes[0].EdgeNodeRef,
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
		t, false, edgeapp.AccountRoleAdmin,
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
		`href="/equipment/edge-nodes/`,
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
		t, false, edgeapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	edgeNodes, err := archive.ListEdgeNodes(context.Background())
	if err != nil || len(edgeNodes) != 1 {
		t.Fatalf("edgeNodes = %#v, err = %v", edgeNodes, err)
	}
	devices, err := archive.ListInventoryDevices(context.Background(), 100, "")
	if err != nil || len(devices) != 1 {
		t.Fatalf("devices = %#v, err = %v", devices, err)
	}
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodGet,
		"/equipment/edge-nodes/"+edgeNodes[0].EdgeNodeRef,
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
			t.Fatalf("EdgeNode detail missing %q: %s", want, body)
		}
	}
	for _, forbidden := range []string{
		`action="/console/devices/`,
		`action="/console/signals/`,
	} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("EdgeNode detail exposes nested setting %q: %s", forbidden, body)
		}
	}
}

func TestEquipmentDeviceDetailLinksSensorsToCanonicalSettings(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	devices, err := archive.ListInventoryDevices(context.Background(), 100, "")
	if err != nil || len(devices) != 1 {
		t.Fatalf("devices = %#v, err = %v", devices, err)
	}
	signals, err := archive.ListInventorySignals(context.Background(), 100, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err = %v", signals, err)
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
		`href="/equipment/devices/` + devices[0].DeviceRef +
			`/sensors/` + signals[0].SignalRef + `"`,
		"センサー設定を開く",
		"機器モデル",
		"mcp9600",
		"24.8",
		"temperature_c",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("device detail missing %q: %s", want, body)
		}
	}
	for _, forbidden := range []string{
		`action="/console/signals/`,
		`data-signal-profile`,
		`name="display_sensor_type"`,
		`name="display_value_kind"`,
	} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("device detail duplicates sensor setting %q: %s", forbidden, body)
		}
	}
}

func TestSensorDetailShowsDeviceModelInSourceFacts(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 100, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err = %v", signals, err)
	}
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodGet,
		"/sensors/"+signals[0].SignalRef,
		nil,
	)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	body := response.Body.String()
	for _, want := range []string{"接続元と受信情報を確認する", "機器モデル", "mcp9600"} {
		if !strings.Contains(body, want) {
			t.Fatalf("sensor detail missing %q: %s", want, body)
		}
	}
}

func TestConsoleReturnTargetAllowsOnlyEquipmentDetailPaths(t *testing.T) {
	valid := []string{
		"/equipment",
		"/equipment/edge-nodes/edge_node_0123456789abcdef0123456789abcdef",
		"/equipment/devices/dev_0123456789abcdef0123456789abcdef",
		"/sensors/sig_0123456789abcdef0123456789abcdef",
		"/equipment/devices/dev_0123456789abcdef0123456789abcdef/" +
			"sensors/sig_0123456789abcdef0123456789abcdef",
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
		"/equipment/edge-nodes/",
		"equipment/edge-nodes/edge_node_0123456789abcdef0123456789abcdef",
		"/equipment/devices/",
		"/equipment/edge-nodes/edge_",
		"/equipment/devices/dev_not-a-resource-ref",
		"/equipment/edge-nodes/edge_a/extra",
		"/equipment/edge-nodes/edge_a?changed=1",
		"/sensors/",
		"sensors/sig_0123456789abcdef0123456789abcdef",
		"/sensors/sig_",
		"/sensors/sig_0123456789abcdef0123456789abcdef/extra",
		"/sensors/sig_0123456789abcdef0123456789abcdef?changed=1",
		"/equipment/devices/dev_0123456789abcdef0123456789abcdef/sensors/",
		"/equipment/devices/dev_0123456789abcdef0123456789abcdef/" +
			"sensors/sig_",
		"/equipment/devices/dev_0123456789abcdef0123456789abcdef/" +
			"sensors/sig_0123456789abcdef0123456789abcdef/extra",
		"/equipment/devices/dev_0123456789abcdef0123456789abcdef/" +
			"sensors/sig_0123456789abcdef0123456789abcdef?changed=1",
		"/equipment/devices/dev_not-a-resource-ref/" +
			"sensors/sig_0123456789abcdef0123456789abcdef",
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
		t, false, edgeapp.AccountRoleAdmin,
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

func TestSensorMonitoringDetailKeepsMonitoringNavigationAndHasNoSettingsForms(
	t *testing.T,
) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 100, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err = %v", signals, err)
	}
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodGet,
		"/sensors/"+signals[0].SignalRef,
		nil,
	)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", response.Code, response.Body.String())
	}
	body := response.Body.String()
	for _, want := range []string{
		`data-console-page="sensors"`,
		`href="/sensors" class="active" aria-current="page"`,
		`<a href="/sensors">センサー一覧</a>`,
		"設定内容",
		">設定を変更</a>",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("monitoring detail missing %q: %s", want, body)
		}
	}
	for _, forbidden := range []string{
		`action="/console/signals/`,
		`action="/console/semantic-rules/`,
		`data-setting-tabs`,
	} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("monitoring detail exposes settings %q: %s", forbidden, body)
		}
	}
}

func TestSensorSettingsDetailUsesEquipmentNavigationAndCanonicalHierarchy(
	t *testing.T,
) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	devices, err := archive.ListInventoryDevices(context.Background(), 100, "")
	if err != nil || len(devices) != 1 {
		t.Fatalf("devices = %#v, err = %v", devices, err)
	}
	signals, err := archive.ListInventorySignals(context.Background(), 100, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err = %v", signals, err)
	}
	path := "/equipment/devices/" + devices[0].DeviceRef +
		"/sensors/" + signals[0].SignalRef
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(http.MethodGet, path, nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", response.Code, response.Body.String())
	}
	body := response.Body.String()
	for _, want := range []string{
		`data-console-page="equipment"`,
		`href="/equipment" class="active" aria-current="page"`,
		`<a href="/equipment">機器管理</a>`,
		`href="/equipment/edge-nodes/`,
		`href="/equipment/devices/` + devices[0].DeviceRef + `"`,
		`action="/console/signals/` + signals[0].SignalRef + `/profile"`,
		`action="/console/signals/` + signals[0].SignalRef + `/calibration"`,
		`action="/console/signals/` + signals[0].SignalRef + `/semantic-rules"`,
		`name="return_to" value="` + path + `"`,
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("settings detail missing %q: %s", want, body)
		}
	}
}

func TestSensorSettingsRouteRejectsADeviceThatDoesNotOwnTheSignal(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 100, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err = %v", signals, err)
	}
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodGet,
		"/equipment/devices/dev_00000000000000000000000000000000/sensors/"+
			signals[0].SignalRef,
		nil,
	)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	if response.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want 404; body=%s", response.Code, response.Body.String())
	}
}

func TestSensorRoutesReturnNotFoundForUnknownDetail(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleAdmin,
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

func TestSensorListShowsCurrentValuesAndLinksToEquipmentSettingsForAdmin(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 100, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err = %v", signals, err)
	}
	if signals[0].DeviceRef == nil {
		t.Fatalf("signal has no device reference: %#v", signals[0])
	}
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(http.MethodGet, "/sensors", nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	body := response.Body.String()
	settingsPath := "/equipment/devices/" + *signals[0].DeviceRef +
		"/sensors/" + signals[0].SignalRef
	for _, want := range []string{
		"センサー一覧",
		"24.8",
		"factory-edge-01",
		`href="` + settingsPath + `"`,
		`data-href="` + settingsPath + `"`,
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("sensor list missing %q: %s", want, body)
		}
	}
	for _, forbidden := range []string{
		`action="/console/signals/`,
		`name="display_name"`,
		`name="kind"`,
	} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("sensor list exposes setting %q: %s", forbidden, body)
		}
	}
}

func TestSensorListLinksToMonitoringDetailForViewer(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleViewer,
	)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 100, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err = %v", signals, err)
	}
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(http.MethodGet, "/sensors", nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	body := response.Body.String()
	monitoringPath := "/sensors/" + signals[0].SignalRef
	if !strings.Contains(body, `href="`+monitoringPath+`"`) {
		t.Fatalf("viewer sensor list missing monitoring link %q: %s",
			monitoringPath, body)
	}
	if strings.Contains(body, "/equipment/devices/") {
		t.Fatalf("viewer sensor list exposes equipment settings link: %s", body)
	}
}

func TestLegacySensorConsoleRoutesRedirectToSensorList(t *testing.T) {
	server := newTestServerWithRole(
		t, false, edgeapp.AccountRoleAdmin,
	)
	cookie, _ := loginTestAccount(t, server)
	for _, path := range []string{"/monitor", "/signals"} {
		request := httptest.NewRequest(http.MethodGet, path, nil)
		request.AddCookie(cookie)
		response := httptest.NewRecorder()
		server.ServeHTTP(response, request)
		if response.Code != http.StatusSeeOther ||
			response.Header().Get("Location") != "/sensors" {
			t.Fatalf("%s response = %d location=%q",
				path, response.Code, response.Header().Get("Location"))
		}
	}
}

func TestSensorSettingsDetailShowsCurrentValueSourceAndSettingsForAdmin(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	devices, err := archive.ListInventoryDevices(context.Background(), 100, "")
	if err != nil || len(devices) != 1 {
		t.Fatalf("devices = %#v, err = %v", devices, err)
	}
	signals, err := archive.ListInventorySignals(context.Background(), 100, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err = %v", signals, err)
	}
	cookie, _ := loginTestAccount(t, server)
	path := "/equipment/devices/" + devices[0].DeviceRef +
		"/sensors/" + signals[0].SignalRef
	request := httptest.NewRequest(http.MethodGet, path, nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	body := response.Body.String()
	for _, want := range []string{
		`class="sensor-detail"`,
		`class="sensor-detail-header"`,
		`class="sensor-detail-identity"`,
		`class="sensor-detail-latest`,
		`class="sensor-setting-workspace sensor-setting-workspace--settings"`,
		`class="sensor-detail-settings sensor-setting-controls"`,
		`class="content-section sensor-settings-panel"`,
		`class="sensor-setting-preview"`,
		`class="sensor-preview-current`,
		`data-preview-current-value`,
		`data-preview-current-received`,
		`<details class="sensor-source-details">`,
		"factory-edge-01",
		"temperature_c",
		"24.8",
		`action="/console/signals/` + signals[0].SignalRef + `/profile"`,
		`action="/console/signals/` + signals[0].SignalRef + `/calibration"`,
		`action="/console/signals/` + signals[0].SignalRef + `/semantic-rules"`,
		`name="display_name"`,
		`name="kind"`,
		`value="thermocouple"`,
		`>熱電対</option>`,
		`>照度</option>`,
		`入力値の補正`,
		`異常検知`,
		`data-semantic-detector`,
		`name="rise_threshold"`,
		`name="fall_threshold"`,
		`name="rise_debounce_seconds"`,
		`name="fall_debounce_seconds"`,
		`data-semantic-trigger hidden`,
		`OFFからONへ変わったとき`,
		`name="return_to" value="` + path + `"`,
		`data-setting-simulation`,
		`data-preview-chart`,
		`data-preview-toggle`,
		`role="switch" aria-checked="true"`,
		`data-preview-accessible-summary`,
		`data-setting-tabs`,
		`role="tablist" aria-label="設定の種類"`,
		`data-setting-tab="basic"`,
		`data-setting-tab="normal"`,
		`data-setting-tab="alarm"`,
		`data-setting-panel="basic"`,
		`data-setting-panel="normal"`,
		`data-setting-panel="alarm"`,
		`class="semantic-advanced-settings"`,
		`判定の安定化`,
		`name="preview_test_value"`,
		`id="sensor-profile"`,
		`name="return_anchor" value="sensor-profile"`,
		`id="sensor-calibration"`,
		`name="return_anchor" value="sensor-calibration"`,
		`id="rule-create"`,
		`name="return_anchor" value="rule-create"`,
		"受信値と設定結果",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("sensor detail missing %q: %s", want, body)
		}
	}
	if strings.Contains(body, "preview-button") {
		t.Fatalf("sensor detail still contains the start/stop preview: %s", body)
	}
	if strings.Contains(body, `class="setting-flow-step"`) {
		t.Fatalf("sensor detail still contains the space-consuming flow steps: %s", body)
	}
	if strings.Contains(body, "累積するタイミング") {
		t.Fatalf("sensor detail still exposes the internal trigger wording: %s", body)
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
	cssBytes, err := os.ReadFile("static/edge.css")
	if err != nil {
		t.Fatal(err)
	}
	css := string(cssBytes)
	for _, forbidden := range []string{
		".sensor-data-flow {",
		".semantic-rule-output {",
		".semantic-rule-group > header {",
	} {
		if strings.Contains(css, forbidden) {
			t.Fatalf("sensor editor stylesheet retains obsolete layer %q", forbidden)
		}
	}
}

func TestSensorDetailSeparatesNormalAndAbnormalSemanticRules(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	devices, err := archive.ListInventoryDevices(context.Background(), 10, "")
	if err != nil || len(devices) != 1 {
		t.Fatalf("devices=%#v err=%v", devices, err)
	}
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	configuration, err := archive.GetSemanticConfiguration(
		context.Background(),
		signals[0].SignalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	expected := configuration.Revision
	normal, err := archive.CreateSemanticRule(
		context.Background(),
		edgeapp.LocalCLIActor(),
		signals[0].SignalRef,
		"現在温度",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		edgeapp.RevisionPrecondition{Expected: &expected},
	)
	if err != nil {
		t.Fatal(err)
	}
	configuration, err = archive.GetSemanticConfiguration(
		context.Background(),
		signals[0].SignalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	expected = configuration.Revision
	alarm, err := archive.CreateSemanticRule(
		context.Background(),
		edgeapp.LocalCLIActor(),
		signals[0].SignalRef,
		"高温アラーム",
		semantics.RuleSpec{
			Kind: semantics.KindAlarm,
			Detector: semantics.Detector{
				Mode:          semantics.DetectorHighActive,
				RiseThreshold: 40,
				FallThreshold: 38,
			},
		},
		edgeapp.RevisionPrecondition{Expected: &expected},
	)
	if err != nil {
		t.Fatal(err)
	}

	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodGet,
		"/equipment/devices/"+devices[0].DeviceRef+
			"/sensors/"+signals[0].SignalRef+
			"?error=threshold_order&focus=rule-"+alarm.ID,
		nil,
	)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	body := response.Body.String()

	for _, want := range []string{
		"入力値の補正",
		"異常検知",
		"現在温度",
		"高温アラーム",
		`action="/console/semantic-rules/` + normal.ID + `"`,
		`action="/console/semantic-rules/` + alarm.ID + `"`,
		`action="/console/signals/` + signals[0].SignalRef + `/calibration"`,
		`action="/console/signals/` + signals[0].SignalRef + `/semantic-rules"`,
		`data-focus-target="rule-` + alarm.ID + `"`,
		`id="rule-` + normal.ID + `"`,
		`name="return_anchor" value="rule-` + normal.ID + `"`,
		`class="semantic-rule-danger"`,
		`data-confirm-message="現在温度を終了します。`,
		"立ち下がりしきい値は、立ち上がりしきい値以下にしてください。",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("sensor detail missing %q: %s", want, body)
		}
	}
	if strings.Contains(body, "受信した値をどうするか") {
		t.Fatalf("sensor detail still presents a single semantic choice: %s", body)
	}
}

func TestConsoleCreatesNormalAndAlarmRulesIndependently(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleAdmin,
	)
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
	settingsPath := "/equipment/devices/" + devices[0].DeviceRef +
		"/sensors/" + signals[0].SignalRef
	configuration, err := archive.GetSemanticConfiguration(
		context.Background(),
		signals[0].SignalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	postRule := func(values url.Values) {
		t.Helper()
		values.Set("_csrf", csrf)
		values.Set("return_to", settingsPath)
		request := httptest.NewRequest(
			http.MethodPost,
			"/console/signals/"+signals[0].SignalRef+"/semantic-rules",
			strings.NewReader(values.Encode()),
		)
		request.AddCookie(cookie)
		request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		request.Header.Set("Origin", testOrigin)
		response := httptest.NewRecorder()
		server.ServeHTTP(response, request)
		if response.Code != http.StatusSeeOther ||
			response.Header().Get("Location") != settingsPath+"?saved=1" {
			t.Fatalf("status=%d location=%q", response.Code, response.Header().Get("Location"))
		}
	}
	postRule(url.Values{
		"revision":     {strconv.FormatInt(configuration.Revision, 10)},
		"display_name": {"現在温度"},
		"kind":         {"numeric"},
	})
	configuration, err = archive.GetSemanticConfiguration(
		context.Background(),
		signals[0].SignalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	postRule(url.Values{
		"revision":              {strconv.FormatInt(configuration.Revision, 10)},
		"display_name":          {"高温アラーム"},
		"kind":                  {"alarm"},
		"detector_mode":         {"high_active"},
		"rise_threshold":        {"40"},
		"fall_threshold":        {"38"},
		"rise_debounce_seconds": {"2"},
		"fall_debounce_seconds": {"5"},
	})
	configuration, err = archive.GetSemanticConfiguration(
		context.Background(),
		signals[0].SignalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	if len(configuration.Rules) != 2 ||
		configuration.Rules[0].Kind != semantics.KindNumeric ||
		configuration.Rules[1].Kind != semantics.KindAlarm {
		t.Fatalf("rules=%#v", configuration.Rules)
	}
}

func TestSensorDetailViewerSeesFactsWithoutMutationControls(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleViewer,
	)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 100, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err = %v", signals, err)
	}
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodGet,
		"/sensors/"+signals[0].SignalRef,
		nil,
	)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	body := response.Body.String()
	for _, want := range []string{
		"factory-edge-01",
		"temperature_c",
		"24.8",
		`data-setting-simulation`,
		`data-preview-chart`,
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("viewer sensor detail missing %q: %s", want, body)
		}
	}
	for _, forbidden := range []string{
		`action="/console/signals/`,
		`name="display_name"`,
		`name="kind"`,
		`name="preview_test_value"`,
		">設定を変更</a>",
	} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("viewer sensor detail exposes %q: %s", forbidden, body)
		}
	}
}

func TestMappingPreviewReturnsBoundedHistoryAndTestValueWithoutWriting(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleAdmin,
	)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 100, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err = %v", signals, err)
	}
	beforeRaw, err := archive.ListRawRecords(context.Background(), 100)
	if err != nil {
		t.Fatal(err)
	}
	beforeAudit, err := archive.ListAuditEvents(context.Background(), 100)
	if err != nil {
		t.Fatal(err)
	}
	cookie, csrf := loginTestAccount(t, server)
	body := bytes.NewBufferString(`{
		"signal_ref":"` + signals[0].SignalRef + `",
		"spec":{"kind":"numeric","scale":2,"offset":1,
			"condition":{"mode":"","bool_value":false,"threshold":0,"hysteresis":0},
			"trigger":""},
		"test_value":10
	}`)
	request := httptest.NewRequest(
		http.MethodPost,
		"/api/v1/mapping-previews",
		body,
	)
	request.AddCookie(cookie)
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Origin", testOrigin)
	request.Header.Set("X-CSRF-Token", csrf)
	response := httptest.NewRecorder()

	server.ServeHTTP(response, request)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", response.Code, response.Body.String())
	}
	var preview struct {
		Kind       semantics.Kind           `json:"kind"`
		InputCount int                      `json:"input_count"`
		PlotCount  int                      `json:"plot_count"`
		Points     []semantics.PreviewPoint `json:"points"`
		TestResult *semantics.Result        `json:"test_result"`
	}
	if err := json.Unmarshal(response.Body.Bytes(), &preview); err != nil {
		t.Fatal(err)
	}
	if preview.Kind != semantics.KindNumeric ||
		preview.InputCount != 1 ||
		preview.PlotCount != 1 ||
		len(preview.Points) != 1 ||
		preview.Points[0].Input != 24.8 ||
		preview.Points[0].Calibrated != 50.6 ||
		preview.TestResult == nil ||
		preview.TestResult.Calibrated != 21 {
		t.Fatalf("preview = %#v", preview)
	}
	afterRaw, err := archive.ListRawRecords(context.Background(), 100)
	if err != nil {
		t.Fatal(err)
	}
	afterAudit, err := archive.ListAuditEvents(context.Background(), 100)
	if err != nil {
		t.Fatal(err)
	}
	if len(afterRaw) != len(beforeRaw) || len(afterAudit) != len(beforeAudit)+1 {
		t.Fatalf(
			"preview mutated state: raw %d -> %d, audit before login %d -> after preview %d",
			len(beforeRaw), len(afterRaw), len(beforeAudit), len(afterAudit),
		)
	}
}

func TestConsoleRejectsRetiredSingleDefinitionMutation(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	cookie, csrf := loginTestAccount(t, server)
	form := url.Values{
		"_csrf":                 {csrf},
		"return_to":             {"/sensors/" + signals[0].SignalRef},
		"kind":                  {"cumulative_counter"},
		"scale":                 {"1"},
		"offset":                {"0"},
		"detector_mode":         {"low_active"},
		"rise_threshold":        {"80"},
		"fall_threshold":        {"20"},
		"rise_debounce_seconds": {"1.5"},
		"fall_debounce_seconds": {"2.5"},
		"trigger":               {"on_transition"},
	}
	request := httptest.NewRequest(
		http.MethodPost,
		"/console/signals/"+signals[0].SignalRef+"/semantic",
		strings.NewReader(form.Encode()),
	)
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	request.Header.Set("Origin", testOrigin)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusGone {
		t.Fatalf("status=%d body=%s", response.Code, response.Body.String())
	}
	definitions, err := archive.ListSemanticDefinitions(context.Background())
	if err != nil || len(definitions) != 0 {
		t.Fatalf("definitions=%#v err=%v", definitions, err)
	}
}
