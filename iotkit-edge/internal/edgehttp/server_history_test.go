package edgehttp

import (
	"bytes"
	"context"
	"encoding/json"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strconv"
	"strings"
	"testing"
	"time"
)

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

func TestHistoryAPIAndCSVUseTheSameAuthenticatedFilter(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleViewer)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	cookie, _ := loginTestAccount(t, server)
	now := time.Now().UnixMilli()
	query := "?signal_ref=" + url.QueryEscape(signals[0].SignalRef) +
		"&from=" + strconv.FormatInt(now-int64(time.Hour/time.Millisecond), 10) +
		"&to=" + strconv.FormatInt(now+int64(time.Hour/time.Millisecond), 10)

	anonymous := httptest.NewRequest(http.MethodGet, "/api/v1/history"+query, nil)
	anonymousResponse := httptest.NewRecorder()
	server.ServeHTTP(anonymousResponse, anonymous)
	if anonymousResponse.Code != http.StatusUnauthorized {
		t.Fatalf("anonymous status=%d", anonymousResponse.Code)
	}

	request := httptest.NewRequest(http.MethodGet, "/api/v1/history"+query+"&limit=20", nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatalf("history status=%d body=%s", response.Code, response.Body.String())
	}
	var page struct {
		Records []store.HistoryRecord `json:"records"`
		HasMore bool                  `json:"has_more"`
		Next    string                `json:"next_cursor"`
	}
	if err := json.Unmarshal(response.Body.Bytes(), &page); err != nil {
		t.Fatal(err)
	}
	if len(page.Records) != 1 || page.HasMore || page.Next != "" ||
		page.Records[0].SignalRef != signals[0].SignalRef {
		t.Fatalf("history page=%#v", page)
	}

	// CSV owns its export bound; a browser query parameter must not silently
	// truncate the file.
	csvRequest := httptest.NewRequest(http.MethodGet, "/api/v1/history.csv"+query+"&limit=0", nil)
	csvRequest.AddCookie(cookie)
	csvResponse := httptest.NewRecorder()
	server.ServeHTTP(csvResponse, csvRequest)
	if csvResponse.Code != http.StatusOK {
		t.Fatalf("csv status=%d body=%s", csvResponse.Code, csvResponse.Body.String())
	}
	if contentType := csvResponse.Header().Get("Content-Type"); !strings.Contains(contentType, "text/csv") {
		t.Fatalf("csv content type=%q", contentType)
	}
	for _, want := range []string{
		"received_at,observed_at,edge_node_id,signal_ref,series_key,sensor_name,values,unit",
		signals[0].SignalRef,
		"factory-edge-01",
		"[24.8]",
	} {
		if !strings.Contains(csvResponse.Body.String(), want) {
			t.Fatalf("CSV missing %q: %s", want, csvResponse.Body.String())
		}
	}
}

func TestSemanticHistoryCSVExportsPersistedProcessedObservations(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleViewer)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	configuration, err := archive.GetSemanticConfiguration(
		context.Background(), signals[0].SignalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.CreateSemanticRule(
		context.Background(), edgeapp.LocalCLIActor(), signals[0].SignalRef,
		"補正温度", semantics.RuleSpec{Kind: semantics.KindNumeric},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	); err != nil {
		t.Fatal(err)
	}
	record := json.RawMessage(
		`{"family":"measurement","schema_version":1,"epoch":"epoch-01","pub_seq":2,` +
			`"series_key":"018f0000-0000-7000-8000-000000000001:temperature_c:na:primary",` +
			`"values":[25.5],"event_time":2000,"event_time_source":"received_at",` +
			`"time_source":"edge_node","time_quality":"unsynced","received_at":2000,` +
			`"device_time":null}`,
	)
	batch := contract.RecordBatch{
		SchemaVersion: 1, EdgeNodeID: "factory-edge-01", LedgerEpoch: "epoch-01",
		PublicationID: contract.PublicationID("factory-edge-01", "epoch-01", 2, 2),
		CursorStart:   2, CursorEnd: 2, Records: []json.RawMessage{record},
	}
	if _, err := archive.AcceptBatch(context.Background(), batch); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ProjectSemanticRules(context.Background(), 100); err != nil {
		t.Fatal(err)
	}

	path := "/api/v1/semantic-history.csv?signal_ref=" +
		url.QueryEscape(signals[0].SignalRef) + "&from=0&to=3000"
	anonymous := httptest.NewRequest(http.MethodGet, path, nil)
	anonymousResponse := httptest.NewRecorder()
	server.ServeHTTP(anonymousResponse, anonymous)
	if anonymousResponse.Code != http.StatusUnauthorized {
		t.Fatalf("anonymous status=%d", anonymousResponse.Code)
	}

	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(http.MethodGet, path, nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s", response.Code, response.Body.String())
	}
	if !bytes.HasPrefix(response.Body.Bytes(), []byte{0xef, 0xbb, 0xbf}) {
		t.Fatalf("semantic CSV does not start with UTF-8 BOM: %q", response.Body.Bytes())
	}
	for _, want := range []string{
		"observed_at,processed_at,edge_node_id,signal_ref,sensor_name,rule_name,kind,value,unit,series_id,sequence,observation_id,rule_revision,calibration_revision,source_pub_seq",
		"補正温度", "numeric", "25.5", signals[0].SignalRef,
	} {
		if !strings.Contains(response.Body.String(), want) {
			t.Fatalf("semantic CSV missing %q: %s", want, response.Body.String())
		}
	}
}

func TestSemanticHistoryCSVRefusesTruncatedSuccess(t *testing.T) {
	response := httptest.NewRecorder()
	writeSemanticHistoryCSV(response, store.SemanticHistoryPage{HasMore: true})
	if response.Code != http.StatusUnprocessableEntity {
		t.Fatalf("status=%d body=%s", response.Code, response.Body.String())
	}
	if strings.Contains(response.Header().Get("Content-Type"), "text/csv") ||
		strings.Contains(response.Body.String(), "observed_at") {
		t.Fatalf("oversized export returned a partial CSV: headers=%#v body=%s",
			response.Header(), response.Body.String())
	}
}

func TestHistoryAPIRejectsOversizedJSONPage(t *testing.T) {
	server := newTestServer(t, false)
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodGet,
		"/api/v1/history?from=0&to=1000&limit=1001",
		nil,
	)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusBadRequest {
		t.Fatalf("status=%d body=%s", response.Code, response.Body.String())
	}
}

func TestHistorySeriesAPIRejectsAnUnboundedOrMissingSignal(t *testing.T) {
	server := newTestServer(t, false)
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodGet,
		"/api/v1/history/series?from=0&to="+
			strconv.FormatInt(int64(31*24*time.Hour/time.Millisecond)+1, 10)+
			"&bucket_ms=1000",
		nil,
	)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusBadRequest {
		t.Fatalf("status=%d body=%s", response.Code, response.Body.String())
	}
}

func TestHistoryConsoleKeepsFilterChartTableAndExportOnOnePage(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleViewer)
	seedSetupDevice(t, archive)
	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(http.MethodGet, "/logs?range=1h", nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s", response.Code, response.Body.String())
	}
	body := response.Body.String()
	for _, want := range []string{
		`id="history-filter"`,
		`name="signal_ref"`,
		`name="edge_node_id"`,
		`name="range"`,
		`class="history-chart"`,
		`aria-label="受信値の推移`,
		`加工後CSV`,
		`受信した生データCSV`,
		`/api/v1/semantic-history.csv?`,
		`/api/v1/history.csv?`,
		`補正・判定・累積ルールを適用した結果`,
		`24.8`,
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("history console missing %q: %s", want, body)
		}
	}
}

func TestStorageStatusAPIAndConsoleReportFactsInsteadOfAssumedHealth(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleViewer)
	seedSetupDevice(t, archive)
	cookie, _ := loginTestAccount(t, server)

	apiRequest := httptest.NewRequest(http.MethodGet, "/api/v1/system/storage", nil)
	apiRequest.AddCookie(cookie)
	apiResponse := httptest.NewRecorder()
	server.ServeHTTP(apiResponse, apiRequest)
	if apiResponse.Code != http.StatusOK {
		t.Fatalf("storage API status=%d body=%s", apiResponse.Code, apiResponse.Body.String())
	}
	var status store.StorageStatus
	if err := json.Unmarshal(apiResponse.Body.Bytes(), &status); err != nil {
		t.Fatal(err)
	}
	if !status.FilesystemAvailable || status.RawRecordCount != 1 || status.DatabaseBytes == 0 {
		t.Fatalf("storage status=%#v", status)
	}
	diagnosticRequest := httptest.NewRequest(http.MethodGet, "/api/v1/system/diagnostics", nil)
	diagnosticRequest.AddCookie(cookie)
	diagnosticResponse := httptest.NewRecorder()
	server.ServeHTTP(diagnosticResponse, diagnosticRequest)
	if diagnosticResponse.Code != http.StatusOK {
		t.Fatalf("diagnostic API status=%d body=%s", diagnosticResponse.Code, diagnosticResponse.Body.String())
	}
	var diagnostics store.DiagnosticReport
	if err := json.Unmarshal(diagnosticResponse.Body.Bytes(), &diagnostics); err != nil {
		t.Fatal(err)
	}
	if diagnostics.GeneratedAt == 0 || len(diagnostics.Limitations) == 0 {
		t.Fatalf("diagnostics=%#v", diagnostics)
	}

	pageRequest := httptest.NewRequest(http.MethodGet, "/system", nil)
	pageRequest.AddCookie(cookie)
	pageResponse := httptest.NewRecorder()
	server.ServeHTTP(pageResponse, pageRequest)
	if pageResponse.Code != http.StatusOK {
		t.Fatalf("system status=%d body=%s", pageResponse.Code, pageResponse.Body.String())
	}
	body := pageResponse.Body.String()
	for _, want := range []string{
		"Console応答中", "保存容量", "ディスク使用率", "raw受信データ",
		"未配送の外部出力", "容量対策として未配送データを自動削除しません",
		"確認が必要なこと", "この画面だけでは判別できないこと",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("system page missing %q: %s", want, body)
		}
	}
	if strings.Contains(body, "Consoleとデータ保存サービスは動作しています") {
		t.Fatalf("system page still assumes storage health: %s", body)
	}
}

func TestEdgeNodeActivationAPIRequiresAdminAndQueuesActivation(t *testing.T) {
	adminServer, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleAdmin,
	)
	edgeNode := seedDiscoveredEdge(t, archive)

	unauthorized := httptest.NewRequest(
		http.MethodPost,
		"/api/v1/edge-nodes/"+edgeNode.EdgeNodeRef+"/activation",
		nil,
	)
	unauthorizedResponse := httptest.NewRecorder()
	adminServer.ServeHTTP(unauthorizedResponse, unauthorized)
	if unauthorizedResponse.Code != http.StatusUnauthorized {
		t.Fatalf("unauthorized status = %d", unauthorizedResponse.Code)
	}

	viewerServer, viewerArchive := newTestServerFixture(
		t, false, edgeapp.AccountRoleViewer,
	)
	viewerEdge := seedDiscoveredEdge(t, viewerArchive)
	viewerCookie, viewerCSRF := loginTestAccount(t, viewerServer)
	viewerRequest := httptest.NewRequest(
		http.MethodPost,
		"/api/v1/edge-nodes/"+viewerEdge.EdgeNodeRef+"/activation",
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
		"/api/v1/edge-nodes/"+edgeNode.EdgeNodeRef+"/activation",
		nil,
	)
	adminRequest.AddCookie(adminCookie)
	adminRequest.Header.Set("Origin", testOrigin)
	adminRequest.Header.Set("X-CSRF-Token", adminCSRF)
	adminRequest.Header.Set("If-Match", revisionETag(edgeNode.Revision))
	adminResponse := httptest.NewRecorder()
	adminServer.ServeHTTP(adminResponse, adminRequest)
	if adminResponse.Code != http.StatusAccepted {
		t.Fatalf("admin status = %d, body=%s",
			adminResponse.Code, adminResponse.Body.String())
	}
	var activated edgeapp.EdgeNode
	if err := json.Unmarshal(adminResponse.Body.Bytes(), &activated); err != nil {
		t.Fatal(err)
	}
	if activated.State != edgeapp.EdgeNodeActivating ||
		strings.Contains(adminResponse.Body.String(), "activation_id") {
		t.Fatalf("activated EdgeNode = %#v", activated)
	}
	commands, err := archive.ListPendingActivationCommands(
		context.Background(), 10,
	)
	if err != nil || len(commands) != 1 {
		t.Fatalf("commands = %#v, err = %v", commands, err)
	}
}
