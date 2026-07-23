package edgehttp

import (
	"context"
	"encoding/json"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"
)

func TestSemanticConfigurationAPIStoresTwoRulesForOneSignal(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	signalRef := signals[0].SignalRef
	cookie, csrf := loginTestAccount(t, server)

	getConfiguration := func() (semantics.Configuration, string) {
		t.Helper()
		request := httptest.NewRequest(
			http.MethodGet,
			"/api/v1/signals/"+signalRef+"/semantic-configuration",
			nil,
		)
		request.AddCookie(cookie)
		response := httptest.NewRecorder()
		server.ServeHTTP(response, request)
		if response.Code != http.StatusOK {
			t.Fatalf("GET status=%d body=%s", response.Code, response.Body.String())
		}
		var configuration semantics.Configuration
		if err := json.Unmarshal(response.Body.Bytes(), &configuration); err != nil {
			t.Fatal(err)
		}
		return configuration, response.Header().Get("ETag")
	}
	createRule := func(etag, body string) string {
		t.Helper()
		request := httptest.NewRequest(
			http.MethodPost,
			"/api/v1/signals/"+signalRef+"/semantic-rules",
			strings.NewReader(body),
		)
		request.AddCookie(cookie)
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("Origin", testOrigin)
		request.Header.Set("X-CSRF-Token", csrf)
		request.Header.Set("If-Match", etag)
		response := httptest.NewRecorder()
		server.ServeHTTP(response, request)
		if response.Code != http.StatusCreated {
			t.Fatalf("POST status=%d body=%s", response.Code, response.Body.String())
		}
		return response.Header().Get("ETag")
	}

	configuration, etag := getConfiguration()
	if configuration.Calibration.Scale != 1 || len(configuration.Rules) != 0 ||
		etag != revisionETag(configuration.Revision) {
		t.Fatalf("initial configuration=%#v etag=%q", configuration, etag)
	}
	etag = createRule(etag, `{
		"display_name":"生産回数",
		"spec":{
			"kind":"cumulative_counter",
			"detector":{"mode":"boolean_high_active"},
			"trigger":"on_transition"
		}
	}`)
	if etag != revisionETag(configuration.Revision+1) {
		t.Fatalf("created configuration etag=%q", etag)
	}
	createRule(etag, `{
		"display_name":"停止アラーム",
		"spec":{
			"kind":"alarm",
			"detector":{"mode":"boolean_low_active"},
			"trigger":""
		}
	}`)
	configuration, _ = getConfiguration()
	if len(configuration.Rules) != 2 ||
		configuration.Rules[0].ID == configuration.Rules[1].ID {
		t.Fatalf("configuration rules=%#v", configuration.Rules)
	}
}

func TestOutputAdapterAPIHasNoIndividualRouteCreation(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
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
	rule, err := archive.CreateSemanticRule(
		context.Background(),
		edgeapp.LocalCLIActor(),
		signals[0].SignalRef,
		"生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	cookie, csrf := loginTestAccount(t, server)

	request := httptest.NewRequest(
		http.MethodGet,
		"/api/v1/output-adapters",
		nil,
	)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusOK ||
		!strings.Contains(response.Body.String(), `"id":"iotkit.mqtt-json.v1"`) ||
		!strings.Contains(response.Body.String(), `"id":"pinikiet.mqtt.v1"`) ||
		!strings.Contains(response.Body.String(), `"cumulative_value"`) {
		t.Fatalf("adapters status=%d body=%s",
			response.Code, response.Body.String())
	}

	request = httptest.NewRequest(
		http.MethodPost,
		"/api/v1/output-routes",
		strings.NewReader(`{
			"rule_id":"`+rule.ID+`",
			"adapter_id":"iotkit.mqtt-json.v1",
			"config":{
				"schema_version":1,
				"topic":"factory/line-a/production"
			}
		}`),
	)
	request.AddCookie(cookie)
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Origin", testOrigin)
	request.Header.Set("X-CSRF-Token", csrf)
	response = httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusMethodNotAllowed {
		t.Fatalf("create status=%d body=%s",
			response.Code, response.Body.String())
	}

	request = httptest.NewRequest(
		http.MethodGet,
		"/api/v1/output-routes",
		nil,
	)
	request.AddCookie(cookie)
	response = httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusOK ||
		!strings.Contains(response.Body.String(), `"items":[]`) {
		t.Fatalf("routes status=%d body=%s",
			response.Code, response.Body.String())
	}
}

func TestConsoleDashboardIgnoresIndividualOutputRoutes(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
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
	rule, err := archive.CreateSemanticRule(
		context.Background(),
		edgeapp.LocalCLIActor(),
		signals[0].SignalRef,
		"生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := outputadapter.EncodeGenericMQTTJSONConfig(
		outputadapter.GenericMQTTJSONConfig{
			Topic: "factory/line-a/production",
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ApplyOutputRoute(
		context.Background(),
		edgeapp.LocalCLIActor(),
		rule.ID,
		"iotkit.mqtt-json.v1",
		encoded,
	); err != nil {
		t.Fatal(err)
	}

	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(http.MethodGet, "/status", nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	body := response.Body.String()

	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s", response.Code, body)
	}
	for _, want := range []string{
		"受信した値",
		"Edgeで作る値",
		"外部出力",
		"出力未設定",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("dashboard missing %q: %s", want, body)
		}
	}
	if strings.Contains(body, "1件の出力") {
		t.Fatalf("dashboard exposed an individual output route: %s", body)
	}
}

func TestSensorDetailShowsCompactIdentityAndOutputLink(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	if _, err := archive.UpdateSignalProfile(
		context.Background(),
		edgeapp.LocalCLIActor(),
		signals[0].SignalRef,
		edgeapp.SignalProfileInput{
			DisplayName:       "乾燥炉入口",
			DisplaySensorType: "thermocouple",
			DisplayValueKind:  "numeric",
			DisplayUnitMode:   "unit",
			DisplayUnit:       "%",
			DecimalPlaces:     0,
		},
		edgeapp.RevisionPrecondition{},
	); err != nil {
		t.Fatal(err)
	}
	seedConsoleNumericRule(t, archive, signals[0].SignalRef, "現在温度")
	if _, err := archive.ActivateExportProfile(
		context.Background(),
		edgeapp.LocalCLIActor(),
		"IoTKit MQTT JSON v1",
		"iotkit.mqtt-json.v1",
	); err != nil {
		t.Fatal(err)
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

	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s", response.Code, body)
	}
	for _, want := range []string{
		`class="sensor-detail-identity"`,
		`class="sensor-detail-latest`,
		"現在温度",
		"外部出力あり",
		"Edge全体の外部出力先を見る",
		`href="/output"`,
		`data-source-value="24.8"`,
		`data-source-unit="°C"`,
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("compact sensor detail missing %q: %s", want, body)
		}
	}
	for _, forbidden := range []string{
		"受信した値",
		"Edgeで作る値",
		"外部へ送る",
		"IoTKit MQTT JSON v1",
		"iotkit/v1/sources/edge-",
	} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("compact sensor detail exposes duplicate flow %q: %s", forbidden, body)
		}
	}
}

func TestSensorDetailShowsMissingOutput(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	seedConsoleNumericRule(t, archive, signals[0].SignalRef, "現在温度")

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

	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s", response.Code, body)
	}
	if !strings.Contains(body, "外部出力なし") {
		t.Fatalf("sensor flow hides missing output: %s", body)
	}
}

func TestViewerCanFollowExistingOutputFromSensorDetail(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleViewer)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	rule := seedConsoleNumericRule(t, archive, signals[0].SignalRef, "現在温度")
	encoded, err := outputadapter.EncodeGenericMQTTJSONConfig(
		outputadapter.GenericMQTTJSONConfig{Topic: "factory/line-a/temperature"},
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ApplyOutputRoute(
		context.Background(),
		edgeapp.LocalCLIActor(),
		rule.ID,
		"iotkit.mqtt-json.v1",
		encoded,
	); err != nil {
		t.Fatal(err)
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

	if response.Code != http.StatusOK ||
		!strings.Contains(body, "Edge全体の外部出力先を見る") ||
		!strings.Contains(body, `href="/output"`) ||
		strings.Contains(body, "外部出力を追加") {
		t.Fatalf("viewer output journey status=%d body=%s", response.Code, body)
	}
}

func TestConsoleRejectsRetiredIndividualOutputCreation(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
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
	rule, err := archive.CreateSemanticRule(
		context.Background(),
		edgeapp.LocalCLIActor(),
		signals[0].SignalRef,
		"生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	cookie, csrf := loginTestAccount(t, server)
	form := url.Values{
		"adapter_id": {"iotkit.mqtt-json.v1"},
		"rule_id":    {rule.ID},
		"topic":      {"factory/line-a/production"},
		"_csrf":      {csrf},
	}
	request := httptest.NewRequest(
		http.MethodPost,
		"/console/output-routes",
		strings.NewReader(form.Encode()),
	)
	request.AddCookie(cookie)
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	request.Header.Set("Origin", testOrigin)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusMethodNotAllowed {
		t.Fatalf("save status=%d location=%q body=%s",
			response.Code, response.Header().Get("Location"), response.Body.String())
	}
	routes, err := archive.ListOutputRoutes(context.Background())
	if err != nil || len(routes) != 0 {
		t.Fatalf("routes=%#v err=%v", routes, err)
	}
}

func TestConsoleOutputRouteViewSeparatesTransformAndDeliveryStatus(t *testing.T) {
	errorAt := int64(1_784_500_000_000)
	now := time.UnixMilli(errorAt)
	oldestPendingAt := now.Add(-6 * time.Minute).UnixMilli()
	recentPendingAt := now.Add(-time.Minute).UnixMilli()
	routes := []edgeapp.OutputRoute{
		{
			RouteID:                "out_error",
			RuleID:                 "rule_error",
			AdapterID:              "iotkit.mqtt-json.v1",
			Active:                 true,
			LastTransformErrorCode: "config_version_mismatch",
			LastTransformErrorAt:   &errorAt,
		},
		{
			RouteID:      "out_pending",
			RuleID:       "rule_pending",
			AdapterID:    "iotkit.mqtt-json.v1",
			Active:       true,
			PendingCount: 3,
		},
		{
			RouteID:         "out_stalled",
			RuleID:          "rule_stalled",
			AdapterID:       "iotkit.mqtt-json.v1",
			Active:          true,
			PendingCount:    4,
			OldestPendingAt: &oldestPendingAt,
		},
		{
			RouteID:         "out_stopped_pending",
			RuleID:          "rule_stopped_pending",
			AdapterID:       "iotkit.mqtt-json.v1",
			Active:          false,
			PendingCount:    2,
			OldestPendingAt: &recentPendingAt,
		},
		{
			RouteID:                "out_stopped_empty",
			RuleID:                 "rule_stopped_empty",
			AdapterID:              "iotkit.mqtt-json.v1",
			Active:                 false,
			LastTransformErrorCode: "transform_failed",
		},
		{
			RouteID:        "out_stopped_published",
			RuleID:         "rule_stopped_published",
			AdapterID:      "iotkit.mqtt-json.v1",
			Active:         false,
			PublishedCount: 2,
		},
	}
	rules := []consoleRuleOption{
		{ID: "rule_error", Name: "異常route"},
		{ID: "rule_pending", Name: "配送待ちroute"},
		{ID: "rule_stalled", Name: "配送停止route"},
		{ID: "rule_stopped_pending", Name: "停止後配送route"},
		{ID: "rule_stopped_empty", Name: "停止済みroute"},
		{ID: "rule_stopped_published", Name: "配送済み停止route"},
	}

	views := newConsoleOutputRouteViews(routes, rules, now)

	if len(views) != 6 {
		t.Fatalf("views=%#v", views)
	}
	if views[0].TransformLabel != "変換エラー" ||
		views[0].DeliveryLabel != "配送実績なし" ||
		views[0].StateLabel != "要確認" {
		t.Fatalf("error view=%#v", views[0])
	}
	if views[1].TransformLabel != "設定済み" ||
		views[1].DeliveryLabel != "配送中" ||
		views[1].DeliveryClass != "in-progress" ||
		views[1].StateLabel != "配送中" ||
		views[1].StateClass != "in-progress" {
		t.Fatalf("pending view=%#v", views[1])
	}
	if views[2].DeliveryLabel != "要確認" ||
		views[2].DeliveryClass != "stale" ||
		views[2].StateLabel != "配送停止の可能性" {
		t.Fatalf("stalled view=%#v", views[2])
	}
	if views[3].StateLabel != "停止" ||
		views[3].TransformLabel != "停止中" ||
		views[3].DeliveryLabel != "配送中" ||
		views[3].DeliveryClass != "in-progress" {
		t.Fatalf("stopped pending view=%#v", views[3])
	}
	if views[4].TransformLabel != "停止中" ||
		views[4].DeliveryLabel != "配送実績なし" ||
		views[4].DeliveryClass != "never" {
		t.Fatalf("stopped empty view=%#v", views[4])
	}
	if views[5].TransformLabel != "停止中" ||
		views[5].DeliveryLabel != "待ちなし" ||
		views[5].DeliveryClass != "configured" {
		t.Fatalf("stopped published view=%#v", views[5])
	}
}

func TestConsoleOutputHealthUsesRouteStateInsteadOfExistence(t *testing.T) {
	now := time.UnixMilli(1_784_500_000_000)
	oldestPendingAt := now.Add(-6 * time.Minute).UnixMilli()
	tests := []struct {
		name       string
		routes     []edgeapp.OutputRoute
		wantLabel  string
		wantClass  string
		wantActive int
	}{
		{
			name:      "not configured",
			wantLabel: "外部出力は未設定です",
			wantClass: "attention",
		},
		{
			name: "transform error wins",
			routes: []edgeapp.OutputRoute{{
				Active: true, LastTransformErrorCode: "transform_failed",
			}},
			wantLabel:  "変換エラーがあります",
			wantClass:  "attention",
			wantActive: 1,
		},
		{
			name: "pending wins over configured",
			routes: []edgeapp.OutputRoute{{
				Active: true, PendingCount: 3,
			}},
			wantLabel:  "配送中のデータがあります",
			wantClass:  "in-progress",
			wantActive: 1,
		},
		{
			name: "stalled delivery wins over pending",
			routes: []edgeapp.OutputRoute{{
				Active: true, PendingCount: 3, OldestPendingAt: &oldestPendingAt,
			}},
			wantLabel:  "MQTT配送が停止している可能性があります",
			wantClass:  "attention",
			wantActive: 1,
		},
		{
			name: "stopped route can still have stalled durable delivery",
			routes: []edgeapp.OutputRoute{{
				Active: false, PendingCount: 3, OldestPendingAt: &oldestPendingAt,
			}},
			wantLabel: "MQTT配送が停止している可能性があります",
			wantClass: "attention",
		},
		{
			name: "all stopped",
			routes: []edgeapp.OutputRoute{{
				Active: false,
			}},
			wantLabel: "外部出力は停止中です",
			wantClass: "attention",
		},
		{
			name: "configured",
			routes: []edgeapp.OutputRoute{{
				Active: true,
			}},
			wantLabel:  "外部出力が設定されています",
			wantClass:  "healthy",
			wantActive: 1,
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			data := consoleData{OutputRoutes: test.routes}
			data.summarizeOutputRoutes(now)
			if data.OutputStatusLabel != test.wantLabel ||
				data.OutputHealthClass != test.wantClass ||
				data.OutputActiveCount != test.wantActive {
				t.Fatalf("output status=%#v", data)
			}
		})
	}
}

func TestExportProfileSummaryShowsRegistrationWaitBeforeNormalDelivery(t *testing.T) {
	data := consoleData{
		OutputPendingCount: 4,
		ExportProfiles: []consoleExportProfileView{{
			ExportProfile: edgeapp.ExportProfile{
				State: edgeapp.ExportProfilePreparing,
			},
			PreparedCount: 1,
		}},
	}

	data.summarizeExportProfiles()

	if data.OutputStatusLabel != "外部アプリへの登録待ちがあります" ||
		data.OutputStatusSummary != "1件の外部登録待ち" {
		t.Fatalf("output status=%#v", data)
	}
}

func TestPinikietConsoleOffersOneRegistrationActionPerSensor(t *testing.T) {
	views := newConsoleExportProfileViews(
		[]edgeapp.ExportProfile{{
			ProfileID: "exp_0123456789abcdef0123456789abcdef",
			AdapterID: "pinikiet.mqtt.v1",
			State:     edgeapp.ExportProfilePreparing,
			Bindings: []edgeapp.OutputProfileRuleBinding{
				{
					BindingID: "bind_0123456789abcdef0123456789abcdef",
					SensorID:  "sen-0123456789abcdef0123456789abcdef",
					State:     edgeapp.OutputBindingPrepared,
				},
				{
					BindingID: "bind_1123456789abcdef0123456789abcdef",
					SensorID:  "sen-0123456789abcdef0123456789abcdef",
					State:     edgeapp.OutputBindingPrepared,
				},
			},
		}},
		nil,
		nil,
	)
	if len(views) != 1 || len(views[0].Bindings) != 2 ||
		!views[0].Bindings[0].RegistrationAction ||
		views[0].Bindings[1].RegistrationAction ||
		!views[0].Bindings[1].SharesSensorRegistration {
		t.Fatalf("views=%#v", views)
	}
}

func TestExportProfileTransformFailureIsCountedAsAttention(t *testing.T) {
	views := newConsoleExportProfileViews(
		[]edgeapp.ExportProfile{{
			ProfileID: "exp_0123456789abcdef0123456789abcdef",
			State:     edgeapp.ExportProfileActive,
			Bindings: []edgeapp.OutputProfileRuleBinding{{
				BindingID: "bind_0123456789abcdef0123456789abcdef",
				State:     edgeapp.OutputBindingActive,
			}},
		}},
		nil,
		[]consoleOutputRouteView{{
			OutputRoute: edgeapp.OutputRoute{
				BindingID:              "bind_0123456789abcdef0123456789abcdef",
				Active:                 true,
				LastTransformErrorCode: "transform_failed",
			},
			TransformClass: "stale",
			DeliveryClass:  "configured",
		}},
	)

	if len(views) != 1 || views[0].AttentionCount != 1 ||
		views[0].TransformErrorCount != 1 ||
		!views[0].Bindings[0].NeedsAttention {
		t.Fatalf("profile views=%#v", views)
	}
}

func TestSensorFlowCountsPreparedAndUnconfiguredOutputBindings(t *testing.T) {
	signals := []consoleSignalView{{
		NormalRules: []consoleSemanticRuleView{{
			Rule: semantics.Rule{ID: "rule-prepared"},
		}},
		AlarmRules: []consoleSemanticRuleView{{
			Rule: semantics.Rule{ID: "rule-needs-config"},
		}},
	}}
	profiles := []edgeapp.ExportProfile{{
		Bindings: []edgeapp.OutputProfileRuleBinding{
			{RuleID: "rule-prepared", State: edgeapp.OutputBindingPrepared},
			{
				RuleID: "rule-needs-config",
				State:  edgeapp.OutputBindingNeedsConfiguration,
			},
		},
	}}

	attachConsoleOutputBindings(signals, profiles)

	if signals[0].FlowPreparedCount != 1 ||
		signals[0].FlowNeedsConfigCount != 1 ||
		signals[0].FlowIneligibleCount != 0 {
		t.Fatalf("sensor flow=%#v", signals[0])
	}
}

func TestOutputConsoleDoesNotExposeIndividualOutputRoutes(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	rule := seedConsoleNumericRule(t, archive, signals[0].SignalRef, "現在温度")
	encoded, err := outputadapter.EncodeGenericMQTTJSONConfig(
		outputadapter.GenericMQTTJSONConfig{Topic: "factory/line-a/temperature"},
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ApplyOutputRoute(
		context.Background(),
		edgeapp.LocalCLIActor(),
		rule.ID,
		"iotkit.mqtt-json.v1",
		encoded,
	); err != nil {
		t.Fatal(err)
	}

	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodGet,
		"/output?rule_id="+rule.ID+
			"&return_to=/sensors/"+signals[0].SignalRef,
		nil,
	)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	body := response.Body.String()

	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s", response.Code, body)
	}
	for _, want := range []string{
		"汎用MQTT JSONで送る",
		"Pinikietへ送る",
		"現在の対応値と、今後追加する対応値を自動で送信する",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("output console missing %q: %s", want, body)
		}
	}
	for _, forbidden := range []string{
		"LEGACY",
		"旧方式",
		"factory/line-a/temperature",
		"/console/output-routes",
		`name="source_id"`,
		`name="signal_id"`,
		`name="rule_id"`,
	} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("output console retains per-rule input %q: %s", forbidden, body)
		}
	}
}

func TestOutputConsoleHidesStoppedProfilesAndOffersReAdd(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	seedConsoleNumericRule(t, archive, signals[0].SignalRef, "現在温度")
	profile, err := archive.ActivateExportProfile(
		context.Background(),
		edgeapp.LocalCLIActor(),
		"停止した第一工場向け出力",
		"iotkit.mqtt-json.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.RequestExportProfileStop(
		context.Background(),
		edgeapp.LocalCLIActor(),
		profile.ProfileID,
		profile.Revision,
	); err != nil {
		t.Fatal(err)
	}
	if err := archive.ReconcileExportProfileLifecycle(
		context.Background(),
	); err != nil {
		t.Fatal(err)
	}

	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(http.MethodGet, "/output", nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	body := response.Body.String()

	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s", response.Code, body)
	}
	if strings.Contains(body, profile.DisplayName) {
		t.Fatalf("stopped profile remains in current destinations: %s", body)
	}
	if !strings.Contains(body, "汎用MQTT JSONで送る") {
		t.Fatalf("stopped adapter is not offered for re-add: %s", body)
	}
}

func TestOutputConsoleShowsEdgeWideProfileAndCompletePayloadWithoutIdentityInputs(
	t *testing.T,
) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	seedConsoleNumericRule(t, archive, signals[0].SignalRef, "現在温度")
	if _, err := archive.ActivateExportProfile(
		context.Background(),
		edgeapp.LocalCLIActor(),
		"汎用MQTT JSON",
		"iotkit.mqtt-json.v1",
	); err != nil {
		t.Fatal(err)
	}

	cookie, _ := loginTestAccount(t, server)
	request := httptest.NewRequest(http.MethodGet, "/output", nil)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	body := response.Body.String()

	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s", response.Code, body)
	}
	for _, want := range []string{
		"汎用MQTT JSON",
		"今後追加される対応値も自動追加",
		"現在温度",
		"送信内容のサンプル",
		"iotkit/v1/sources/edge-",
		`&#34;observation_id&#34;`,
		`&#34;series_id&#34;`,
		`&#34;sequence&#34;`,
		`&#34;observed_at&#34;`,
		`&#34;kind&#34;`,
		`&#34;value&#34;`,
		"payload（省略なし）",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("output profile console missing %q: %s", want, body)
		}
	}
	for _, forbidden := range []string{
		`name="source_id"`,
		`name="signal_id"`,
		`name="topic"`,
	} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("output profile console exposes %q: %s", forbidden, body)
		}
	}
}

func seedConsoleNumericRule(
	t *testing.T,
	archive *store.Store,
	signalRef string,
	displayName string,
) semantics.Rule {
	t.Helper()
	configuration, err := archive.GetSemanticConfiguration(
		context.Background(),
		signalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	rule, err := archive.CreateSemanticRule(
		context.Background(),
		edgeapp.LocalCLIActor(),
		signalRef,
		displayName,
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	return rule
}

func TestSemanticConfigurationMutationRequiresIfMatch(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	cookie, csrf := loginTestAccount(t, server)
	send := func(ifMatch string) *httptest.ResponseRecorder {
		t.Helper()
		request := httptest.NewRequest(
			http.MethodPost,
			"/api/v1/signals/"+signals[0].SignalRef+"/semantic-rules",
			strings.NewReader(`{
				"display_name":"測定値",
				"spec":{"kind":"numeric","detector":{},"trigger":""}
			}`),
		)
		request.AddCookie(cookie)
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("Origin", testOrigin)
		request.Header.Set("X-CSRF-Token", csrf)
		if ifMatch != "" {
			request.Header.Set("If-Match", ifMatch)
		}
		response := httptest.NewRecorder()
		server.ServeHTTP(response, request)
		return response
	}
	if response := send(""); response.Code != http.StatusPreconditionRequired {
		t.Fatalf("missing If-Match status=%d body=%s", response.Code, response.Body.String())
	}
	if response := send(`"99"`); response.Code != http.StatusPreconditionFailed {
		t.Fatalf("stale If-Match status=%d body=%s", response.Code, response.Body.String())
	}
}

func TestMultiRulePreviewReturnsIndependentCounterAndAlarmResults(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	cookie, csrf := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodPost,
		"/api/v1/mapping-previews",
		strings.NewReader(`{
			"signal_ref":"`+signals[0].SignalRef+`",
			"calibration":{"scale":1,"offset":0},
			"rules":[
				{
					"rule_id":"draft-counter",
					"display_name":"生産回数",
					"spec":{
						"kind":"cumulative_counter",
						"detector":{
							"mode":"high_active",
							"rise_threshold":20,
							"fall_threshold":19
						},
						"trigger":"on_transition"
					}
				},
				{
					"rule_id":"draft-alarm",
					"display_name":"停止アラーム",
					"spec":{
						"kind":"alarm",
						"detector":{
							"mode":"low_active",
							"rise_threshold":30,
							"fall_threshold":29
						},
						"trigger":""
					}
				}
			],
			"test_value":1
		}`),
	)
	request.AddCookie(cookie)
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Origin", testOrigin)
	request.Header.Set("X-CSRF-Token", csrf)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s", response.Code, response.Body.String())
	}
	var preview struct {
		Rules []struct {
			RuleID string         `json:"rule_id"`
			Kind   semantics.Kind `json:"kind"`
		} `json:"rules"`
	}
	if err := json.Unmarshal(response.Body.Bytes(), &preview); err != nil {
		t.Fatal(err)
	}
	if len(preview.Rules) != 2 ||
		preview.Rules[0].RuleID != "draft-counter" ||
		preview.Rules[1].RuleID != "draft-alarm" {
		t.Fatalf("multi-rule preview=%#v", preview)
	}
}

func TestMultiRulePreviewFailureDoesNotHideAnotherRule(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	cookie, csrf := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodPost,
		"/api/v1/mapping-previews",
		strings.NewReader(`{
			"signal_ref":"`+signals[0].SignalRef+`",
			"calibration":{"scale":1,"offset":0},
			"rules":[
				{
					"rule_id":"draft-numeric",
					"display_name":"温度",
					"spec":{"kind":"numeric","detector":{},"trigger":""}
				},
				{
					"rule_id":"draft-boolean",
					"display_name":"接点",
					"spec":{
						"kind":"boolean",
						"detector":{"mode":"boolean_high_active"},
						"trigger":""
					}
				}
			]
		}`),
	)
	request.AddCookie(cookie)
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Origin", testOrigin)
	request.Header.Set("X-CSRF-Token", csrf)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s", response.Code, response.Body.String())
	}
	var preview struct {
		Rules []struct {
			RuleID     string `json:"rule_id"`
			InputCount int    `json:"input_count"`
			Error      string `json:"error"`
		} `json:"rules"`
	}
	if err := json.Unmarshal(response.Body.Bytes(), &preview); err != nil {
		t.Fatal(err)
	}
	if len(preview.Rules) != 2 ||
		preview.Rules[0].InputCount != 1 ||
		preview.Rules[0].Error != "" ||
		preview.Rules[1].Error == "" {
		t.Fatalf("preview=%#v", preview)
	}
}

func TestMappingPreviewViewerUsesSavedDefinitionButCannotSubmitDraft(t *testing.T) {
	server, archive := newTestServerFixture(
		t, false, edgeapp.AccountRoleViewer,
	)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 100, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err = %v", signals, err)
	}
	if _, err := archive.ApplySemanticDefinition(
		context.Background(),
		edgeapp.LocalCLIActor(),
		signals[0].SignalRef,
		semantics.DefinitionSpec{Kind: semantics.KindNumeric, Scale: 1},
		edgeapp.RevisionPrecondition{},
	); err != nil {
		t.Fatal(err)
	}
	cookie, csrf := loginTestAccount(t, server)

	for _, test := range []struct {
		name       string
		body       string
		wantStatus int
	}{
		{
			name:       "saved definition",
			body:       `{"signal_ref":"` + signals[0].SignalRef + `"}`,
			wantStatus: http.StatusOK,
		},
		{
			name: "draft",
			body: `{"signal_ref":"` + signals[0].SignalRef +
				`","spec":{"kind":"numeric","scale":1,"offset":0,` +
				`"condition":{"mode":"","bool_value":false,"threshold":0,"hysteresis":0},` +
				`"trigger":""}}`,
			wantStatus: http.StatusForbidden,
		},
	} {
		t.Run(test.name, func(t *testing.T) {
			request := httptest.NewRequest(
				http.MethodPost,
				"/api/v1/mapping-previews",
				strings.NewReader(test.body),
			)
			request.AddCookie(cookie)
			request.Header.Set("Content-Type", "application/json")
			request.Header.Set("Origin", testOrigin)
			request.Header.Set("X-CSRF-Token", csrf)
			response := httptest.NewRecorder()
			server.ServeHTTP(response, request)
			if response.Code != test.wantStatus {
				t.Fatalf(
					"status = %d, want %d; body=%s",
					response.Code,
					test.wantStatus,
					response.Body.String(),
				)
			}
		})
	}
}

func TestMappingPreviewInvalidDraftNamesTheField(t *testing.T) {
	server, _ := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	cookie, csrf := loginTestAccount(t, server)
	request := httptest.NewRequest(
		http.MethodPost,
		"/api/v1/mapping-previews",
		strings.NewReader(`{
			"signal_ref":"sig_00000000000000000000000000000000",
			"spec":{"kind":"numeric","scale":0,"offset":0,
				"condition":{"mode":"","bool_value":false,"threshold":0,"hysteresis":0},
				"trigger":""}
		}`),
	)
	request.AddCookie(cookie)
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Origin", testOrigin)
	request.Header.Set("X-CSRF-Token", csrf)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, body=%s", response.Code, response.Body.String())
	}
	var failure errorEnvelope
	if err := json.Unmarshal(response.Body.Bytes(), &failure); err != nil {
		t.Fatal(err)
	}
	if failure.Error.Field == nil || *failure.Error.Field != "scale" {
		t.Fatalf("error = %#v", failure.Error)
	}
}

func TestConsoleScriptSupportsAutomaticSettingSimulation(t *testing.T) {
	server := newTestServer(t, false)
	request := httptest.NewRequest(http.MethodGet, "/static/console.js", nil)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatalf("status = %d", response.Code)
	}
	script := response.Body.String()
	for _, want := range []string{
		"[data-setting-simulation]",
		"/api/v1/mapping-previews",
		"AbortController",
		"setTimeout(refresh, 300)",
		"setInterval(() =>",
		"createElementNS",
		"chart-range-result",
		"chart-line-counter",
		"point.received_at",
		"formatCurrentValue",
		`[name="display_value_kind"]`,
		`document.visibilityState === "visible"`,
		"previewUnavailable",
		"result.error?.error.field",
	} {
		if !strings.Contains(script, want) {
			t.Fatalf("console script missing %q", want)
		}
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
		`</span>EdgeNode管理`,
		`</span>デバイス管理`,
		`</span>センサー設定`,
	} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("navigation still exposes %q: %s", forbidden, body)
		}
	}
	if !strings.Contains(body, `</span>機器管理`) ||
		!strings.Contains(body, `</span>センサー一覧`) ||
		strings.Contains(body, `</span>値の変換`) {
		t.Fatalf("navigation does not use equipment journey: %s", body)
	}
}
