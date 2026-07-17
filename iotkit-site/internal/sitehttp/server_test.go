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

func TestMutationRequiresMatchingOriginAndCSRFToken(t *testing.T) {
	server := newTestServer(t, false)
	sessionCookie, csrf := loginTestAccount(t, server)

	for _, test := range []struct {
		name   string
		origin string
		csrf   string
	}{
		{name: "missing origin", csrf: csrf},
		{name: "wrong origin", origin: "https://attacker.example", csrf: csrf},
		{name: "missing csrf", origin: testOrigin},
		{name: "wrong csrf", origin: testOrigin, csrf: "wrong"},
	} {
		t.Run(test.name, func(t *testing.T) {
			request := httptest.NewRequest(http.MethodDelete, "/api/v1/session", nil)
			request.AddCookie(sessionCookie)
			if test.origin != "" {
				request.Header.Set("Origin", test.origin)
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

func newTestServer(t *testing.T, mustChangePassword bool) http.Handler {
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
			Role:               siteapp.AccountRoleViewer,
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
