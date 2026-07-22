package edgehttp

import (
	"context"
	"encoding/json"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"
)

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
		Account   edgeapp.Account `json:"account"`
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
		t, false, edgeapp.AccountRoleViewer,
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
		t, false, edgeapp.AccountRoleAdmin,
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
		`"display_sensor_type":"thermocouple"`,
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
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
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
			pageTitle:   "システム概要",
			activeLabel: "概要",
			mustContain: []string{"センサーの現在値", "要確認", "データの流れ"},
		},
		{
			path:        "/sensors",
			pageTitle:   "センサー一覧",
			activeLabel: "センサー一覧",
			mustContain: []string{
				"<th>センサー</th>", "<th>種類</th>", "<th>現在値</th>",
				"受信状態", "最終受信",
			},
		},
		{
			path:        "/equipment",
			pageTitle:   "機器管理",
			activeLabel: "機器管理",
			mustContain: []string{"確認や設定を行う収集ノードを選んでください", "閲覧のみ"},
		},
		{
			path:        "/setup",
			pageTitle:   "デバイス管理",
			activeLabel: "",
			mustContain: []string{"デバイスごとに", "閲覧のみ"},
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
		"/status", "/sensors", "/equipment", "/setup", "/logs", "/output", "/system",
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
		t, false, edgeapp.AccountRoleViewer,
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
		"収集ノードから届いた情報",
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
		t, false, edgeapp.AccountRoleAdmin,
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
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleViewer)
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

func TestStatusShowsAdministratorsOneOnboardingActionAtATime(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
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
		"利用開始までの設定",
		"1 / 4",
		"次に行うこと",
		"デバイス名と設置場所を設定",
		"外部アプリへ送る場合は、利用開始後に",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("status missing %q: %s", want, body)
		}
	}
	if strings.Count(body, `class="onboarding-step current"`) != 1 {
		t.Fatalf("status must emphasize exactly one next step: %s", body)
	}
}

func TestFreshStatusDoesNotClaimSensorDataIsBeingReceived(t *testing.T) {
	server, _ := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
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
		"収集ノードがまだ登録されていません",
		"収集ノードを登録",
		"0 / 4",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("fresh status missing %q: %s", want, body)
		}
	}
	if strings.Contains(body, "センサーデータを受信しています") {
		t.Fatalf("fresh status claims data is being received: %s", body)
	}
}

func TestDeviceStopsBeingRegistrationPendingAfterDeviceProfileSave(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
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
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
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
	sensorsRequest := httptest.NewRequest(http.MethodGet, "/sensors", nil)
	sensorsRequest.AddCookie(cookie)
	sensorsResponse := httptest.NewRecorder()
	server.ServeHTTP(sensorsResponse, sensorsRequest)
	for _, want := range []string{"乾燥炉入口 温度", "24.80", "°C"} {
		if !strings.Contains(sensorsResponse.Body.String(), want) {
			t.Fatalf("sensor list missing %q: %s", want, sensorsResponse.Body.String())
		}
	}
}

func TestConsoleSignalProfileIgnoresHiddenUnitWhenUnitDisplayIsDisabled(t *testing.T) {
	server, archive := newTestServerFixture(t, false, edgeapp.AccountRoleAdmin)
	seedSetupDevice(t, archive)
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	cookie, csrf := loginTestAccount(t, server)
	returnTo := "/sensors/" + signals[0].SignalRef
	values := url.Values{
		"_csrf":               {csrf},
		"return_to":           {returnTo},
		"display_name":        {"乾燥炉入口 温度"},
		"display_sensor_type": {"temperature"},
		"display_value_kind":  {"numeric"},
		"display_unit_mode":   {"dimensionless"},
		"display_unit":        {"Cel"},
		"decimal_places":      {"1"},
	}
	request := httptest.NewRequest(
		http.MethodPost,
		"/console/signals/"+signals[0].SignalRef+"/profile",
		strings.NewReader(values.Encode()),
	)
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	request.Header.Set("Origin", testOrigin)
	request.AddCookie(cookie)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)

	if response.Code != http.StatusSeeOther ||
		response.Header().Get("Location") != returnTo+"?saved=1" {
		t.Fatalf("signal save=%d location=%q body=%s",
			response.Code, response.Header().Get("Location"), response.Body.String())
	}
	updated, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(updated) != 1 || updated[0].Profile == nil {
		t.Fatalf("updated signals=%#v err=%v", updated, err)
	}
	if updated[0].Profile.DisplayUnitMode != "dimensionless" ||
		updated[0].Profile.DisplayUnit != "" {
		t.Fatalf("profile=%#v", updated[0].Profile)
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
	temperature := newConsoleSignalView(edgeapp.SignalSummary{
		EdgeNodeID:  "factory-edge-01",
		DisplayName: "乾燥炉 温度",
		Unit:        &unit,
		ValueType:   &valueType,
		SensorType:  &sensorType,
		Latest: &edgeapp.LatestMeasurement{
			Values:         json.RawMessage(`[24.900000000000002]`),
			EdgeReceivedAt: receivedAt,
		},
		LastReceivedAt: &receivedAt,
		ReceiptStatus:  "receiving",
	}, now)
	if temperature.Value != "24.9" || temperature.Unit != "°C" ||
		temperature.SensorType != "熱電対" || temperature.LastReceived != "2分前" ||
		temperature.StatusLabel != "受信中" {
		t.Fatalf("temperature view = %#v", temperature)
	}

	boolType := "bool"
	contactType := "contact_state"
	contact := newConsoleSignalView(edgeapp.SignalSummary{
		ValueType:  &boolType,
		SensorType: &contactType,
		Latest:     &edgeapp.LatestMeasurement{Values: json.RawMessage(`[1]`)},
	}, now)
	if contact.Value != "ON" || contact.SensorType != "接点入力" {
		t.Fatalf("contact view = %#v", contact)
	}

	illuminanceType := "illuminance_lux"
	illuminance := newConsoleSignalView(edgeapp.SignalSummary{
		SensorType: &illuminanceType,
	}, now)
	if illuminance.SensorType != "照度" {
		t.Fatalf("illuminance view = %#v", illuminance)
	}

	dimensionless := "1"
	waiting := newConsoleSignalView(edgeapp.SignalSummary{
		Unit: &dimensionless,
	}, now)
	if waiting.Value != "—" || waiting.Unit != "" ||
		waiting.SourceUnit != "1" {
		t.Fatalf("waiting signal view = %#v", waiting)
	}
}

func TestConsoleSignalViewUsesCompletedProfileForEffectiveDisplay(t *testing.T) {
	now := time.Date(2026, 7, 18, 9, 0, 0, 0, time.Local)
	descriptorUnit := "Cel"
	descriptorType := "temperature_c"
	descriptorValueType := "float"
	view := newConsoleSignalView(edgeapp.SignalSummary{
		DisplayName: "電圧入力",
		Unit:        &descriptorUnit,
		SensorType:  &descriptorType,
		ValueType:   &descriptorValueType,
		Profile: &edgeapp.SignalProfile{
			DisplayName:       "電圧入力",
			DisplaySensorType: "voltage",
			DisplayValueKind:  "numeric",
			DisplayUnitMode:   "unit",
			DisplayUnit:       "V",
			DecimalPlaces:     2,
			Revision:          1,
		},
		Latest: &edgeapp.LatestMeasurement{Values: json.RawMessage(`[24.8]`)},
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
			SignalSummary: edgeapp.SignalSummary{
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
	server := newTestServerWithRole(t, false, edgeapp.AccountRoleSystemAdmin)
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

func TestConsoleAuditViewsExplainAdministrativeChanges(t *testing.T) {
	views := newConsoleAuditViews([]edgeapp.AuditEvent{{
		OccurredAt:  1_752_800_000_000,
		ActorClass:  edgeapp.ActorSettingsSession,
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
	views := newConsoleAuditViews([]edgeapp.AuditEvent{
		{Operation: "session.login", ResourceRef: "edge", Outcome: "success"},
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
