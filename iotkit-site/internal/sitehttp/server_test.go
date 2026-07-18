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
			mustContain: []string{"現在の信号", "要確認", "データの流れ"},
		},
		{
			path:        "/monitor",
			pageTitle:   "現在の信号",
			activeLabel: "現在の信号",
			mustContain: []string{"受信状態", "センサー種別", "最終受信"},
		},
		{
			path:        "/signals",
			pageTitle:   "信号の設定",
			activeLabel: "信号の設定",
			mustContain: []string{"表示名と意味付け", "閲覧のみ"},
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
			if !strings.Contains(body, `aria-current="page"`) ||
				!strings.Contains(body, `</span>`+test.activeLabel) {
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
	if len(views) != 1 || views[0].Signal != "乾燥炉 温度" ||
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
			Name: "ライン1 完了信号",
		}},
	)
	if len(options) != 1 || options[0].Name != "ライン1 完了信号" ||
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

	if len(views) != 1 || views[0].Operation != "意味付けを保存" ||
		views[0].Resource != "信号の意味付け" {
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
	return handler
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
