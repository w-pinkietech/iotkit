package sitehttp

import (
	"crypto/rand"
	"embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"html/template"
	"io"
	"net"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/sitesession"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/store"
)

const (
	sessionCookieName                = "iotkit_site_session"
	csrfCookieName                   = "iotkit_site_csrf"
	maxJSONBodyBytes                 = 64 * 1024
	defaultSiteStorageWarningPercent = 90
)

//go:embed templates/*.html static/*
var assets embed.FS

type Config struct {
	Store                 *store.Store
	Site                  *siteapp.Service
	Accounts              *siteapp.AccountService
	Sessions              *sitesession.Manager
	PublicOrigin          string
	DevelopmentHTTP       bool
	CertificateFile       string
	StorageWarningPercent int
}

type Server struct {
	store                 *store.Store
	site                  *siteapp.Service
	accounts              *siteapp.AccountService
	semantics             *siteapp.SemanticService
	semanticConfig        *siteapp.SemanticConfigurationService
	sessions              *sitesession.Manager
	publicOrigin          string
	secureCookies         bool
	templates             *template.Template
	mux                   *http.ServeMux
	previewMu             sync.Mutex
	previewCache          map[string]cachedPreviewWindow
	now                   func() time.Time
	certificateFile       string
	storageWarningPercent int
}

type requestAuth struct {
	principal sitesession.Principal
	token     string
	account   siteapp.Account
}

type errorEnvelope struct {
	Error apiError `json:"error"`
}

type apiError struct {
	Code      string  `json:"code"`
	Message   string  `json:"message"`
	Field     *string `json:"field"`
	RequestID string  `json:"request_id"`
}

func New(config Config) (*Server, error) {
	if config.Store == nil || config.Site == nil || config.Accounts == nil ||
		config.Sessions == nil {
		return nil, errors.New("Site HTTP dependencies must not be nil")
	}
	origin, err := url.Parse(config.PublicOrigin)
	if err != nil || origin.Host == "" || origin.Path != "" ||
		origin.RawQuery != "" || origin.Fragment != "" {
		return nil, errors.New("Site public origin must be an origin without a path")
	}
	if config.DevelopmentHTTP {
		if origin.Scheme != "http" {
			return nil, errors.New("development Site public origin must use http")
		}
	} else if origin.Scheme != "https" {
		return nil, errors.New("production Site public origin must use https")
	}
	templates, err := template.ParseFS(assets, "templates/*.html")
	if err != nil {
		return nil, fmt.Errorf("parse Site templates: %w", err)
	}
	storageWarningPercent := config.StorageWarningPercent
	if storageWarningPercent == 0 {
		storageWarningPercent = defaultSiteStorageWarningPercent
	}
	if storageWarningPercent < 50 || storageWarningPercent > 99 {
		return nil, errors.New("Site storage warning percent must be between 50 and 99")
	}
	server := &Server{
		store:                 config.Store,
		site:                  config.Site,
		accounts:              config.Accounts,
		semantics:             siteapp.NewSemanticService(config.Store),
		semanticConfig:        siteapp.NewSemanticConfigurationService(config.Store),
		sessions:              config.Sessions,
		publicOrigin:          strings.TrimSuffix(config.PublicOrigin, "/"),
		secureCookies:         !config.DevelopmentHTTP,
		templates:             templates,
		mux:                   http.NewServeMux(),
		previewCache:          make(map[string]cachedPreviewWindow),
		now:                   time.Now,
		certificateFile:       config.CertificateFile,
		storageWarningPercent: storageWarningPercent,
	}
	server.routes()
	return server, nil
}

func (server *Server) routes() {
	server.mux.HandleFunc("GET /", server.root)
	server.mux.HandleFunc("GET /login", server.loginPage)
	server.mux.HandleFunc("POST /login", server.loginForm)
	server.mux.HandleFunc("POST /logout", server.logoutForm)
	server.mux.HandleFunc("GET /password", server.passwordPage)
	server.mux.HandleFunc("POST /password", server.passwordForm)
	server.mux.HandleFunc("GET /status", server.statusPage)
	server.mux.HandleFunc("GET /monitor", server.consoleSensorsRedirect)
	server.mux.HandleFunc("GET /sensors", server.consolePage)
	server.mux.HandleFunc("GET /sensors/{signal_ref}", server.consolePage)
	server.mux.HandleFunc("GET /equipment", server.consolePage)
	server.mux.HandleFunc("GET /equipment/edges/{edge_ref}", server.consolePage)
	server.mux.HandleFunc("GET /equipment/devices/{device_ref}", server.consolePage)
	server.mux.HandleFunc(
		"GET /equipment/devices/{device_ref}/sensors/{signal_ref}",
		server.consolePage,
	)
	server.mux.HandleFunc("GET /setup", server.consolePage)
	server.mux.HandleFunc("GET /edges", server.consolePage)
	server.mux.HandleFunc("GET /devices", server.consolePage)
	server.mux.HandleFunc("GET /signals", server.consoleSensorsRedirect)
	server.mux.HandleFunc("GET /logs", server.consolePage)
	server.mux.HandleFunc("GET /output", server.consolePage)
	server.mux.HandleFunc("GET /audit", server.consolePage)
	server.mux.HandleFunc("GET /accounts", server.consolePage)
	server.mux.HandleFunc("GET /system", server.consolePage)
	server.mux.HandleFunc("POST /console/devices/{device_ref}/profile", server.consoleDeviceProfile)
	server.mux.HandleFunc("POST /console/edges/{edge_ref}/activation", server.consoleEdgeActivation)
	server.mux.HandleFunc("POST /console/signals/{signal_ref}/profile", server.consoleSignalProfile)
	server.mux.HandleFunc("POST /console/signals/{signal_ref}/semantic", server.deprecatedConsoleSemanticMutation)
	server.mux.HandleFunc("POST /console/signals/{signal_ref}/semantic-counter/reset", server.deprecatedConsoleSemanticMutation)
	server.mux.HandleFunc("POST /console/signals/{signal_ref}/calibration", server.consoleSignalCalibration)
	server.mux.HandleFunc("POST /console/signals/{signal_ref}/semantic-rules", server.consoleSemanticRuleCreate)
	server.mux.HandleFunc("POST /console/semantic-rules/{rule_id}", server.consoleSemanticRuleUpdate)
	server.mux.HandleFunc("POST /console/semantic-rules/{rule_id}/retire", server.consoleSemanticRuleRetire)
	server.mux.HandleFunc("POST /console/semantic-rules/{rule_id}/counter-resets", server.consoleSemanticRuleCounterReset)
	server.mux.HandleFunc("POST /console/export-profiles", server.consoleActivateExportProfile)
	server.mux.HandleFunc(
		"POST /console/export-profiles/{profile_id}/stop",
		server.consoleStopExportProfile,
	)
	server.mux.HandleFunc(
		"POST /console/output-bindings/{binding_id}",
		server.consoleConfigureOutputBinding,
	)
	server.mux.HandleFunc(
		"POST /console/output-bindings/{binding_id}/start",
		server.consoleStartOutputBinding,
	)
	server.mux.HandleFunc("POST /console/accounts", server.consoleAccount)
	server.mux.HandleFunc("POST /console/accounts/{account_ref}", server.consoleAccountUpdate)
	server.mux.HandleFunc("POST /console/accounts/{account_ref}/disable", server.consoleAccountDisable)
	server.mux.HandleFunc("POST /console/accounts/{account_ref}/password", server.consoleAccountPassword)
	server.mux.HandleFunc("GET /static/site.css", server.stylesheet)
	server.mux.HandleFunc("GET /static/console.js", server.consoleScript)
	server.mux.HandleFunc("GET /static/pinkietech-mark.svg", server.brandMark)
	server.mux.HandleFunc("POST /api/v1/session", server.createSession)
	server.mux.HandleFunc("GET /api/v1/session", server.getSession)
	server.mux.HandleFunc("DELETE /api/v1/session", server.deleteSession)
	server.mux.HandleFunc("GET /api/v1/devices", server.listDevices)
	server.mux.HandleFunc("GET /api/v1/edges", server.listEdges)
	server.mux.HandleFunc("POST /api/v1/edges/{edge_ref}/activation", server.activateEdge)
	server.mux.HandleFunc("GET /api/v1/signals", server.listSignals)
	server.mux.HandleFunc("GET /api/v1/history", server.listHistory)
	server.mux.HandleFunc("GET /api/v1/history/series", server.getHistorySeries)
	server.mux.HandleFunc("GET /api/v1/history.csv", server.exportHistoryCSV)
	server.mux.HandleFunc("GET /api/v1/semantic-history.csv", server.exportSemanticHistoryCSV)
	server.mux.HandleFunc("GET /api/v1/system/storage", server.getStorageStatus)
	server.mux.HandleFunc("GET /api/v1/system/diagnostics", server.getDiagnostics)
	server.mux.HandleFunc("GET /api/v1/setup/devices", server.listSetupDevices)
	server.mux.HandleFunc("PUT /api/v1/devices/{device_ref}/profile", server.putDeviceProfile)
	server.mux.HandleFunc("PUT /api/v1/signals/{signal_ref}/profile", server.putSignalProfile)
	server.mux.HandleFunc("GET /api/v1/semantic-definitions", server.listSemanticDefinitions)
	server.mux.HandleFunc("PUT /api/v1/signals/{signal_ref}/semantic-definition", server.deprecatedSemanticMutation)
	server.mux.HandleFunc("DELETE /api/v1/signals/{signal_ref}/semantic-definition", server.deprecatedSemanticMutation)
	server.mux.HandleFunc("POST /api/v1/signals/{signal_ref}/semantic-counter/reset", server.deprecatedSemanticMutation)
	server.mux.HandleFunc("GET /api/v1/signals/{signal_ref}/semantic-configuration", server.getSemanticConfiguration)
	server.mux.HandleFunc("PUT /api/v1/signals/{signal_ref}/calibration", server.putSignalCalibration)
	server.mux.HandleFunc("POST /api/v1/signals/{signal_ref}/semantic-rules", server.createSemanticRule)
	server.mux.HandleFunc("PUT /api/v1/semantic-rules/{rule_id}", server.updateSemanticRule)
	server.mux.HandleFunc("DELETE /api/v1/semantic-rules/{rule_id}", server.retireSemanticRule)
	server.mux.HandleFunc("POST /api/v1/semantic-rules/{rule_id}/counter-resets", server.requestSemanticCounterReset)
	server.mux.HandleFunc("GET /api/v1/output-adapters", server.listOutputAdapters)
	server.mux.HandleFunc("GET /api/v1/export-profiles", server.listExportProfiles)
	server.mux.HandleFunc(
		"POST /api/v1/export-profiles/preview",
		server.previewExportProfile,
	)
	server.mux.HandleFunc("POST /api/v1/export-profiles", server.activateExportProfile)
	server.mux.HandleFunc(
		"PUT /api/v1/output-bindings/{binding_id}",
		server.configureExportBinding,
	)
	server.mux.HandleFunc(
		"POST /api/v1/export-profiles/{profile_id}/stop",
		server.stopExportProfile,
	)
	server.mux.HandleFunc(
		"GET /api/v1/output-bindings/{binding_id}/publication",
		server.getOutputBindingPublication,
	)
	server.mux.HandleFunc(
		"POST /api/v1/output-bindings/{binding_id}/start",
		server.startOutputBinding,
	)
	server.mux.HandleFunc("GET /api/v1/output-routes", server.listOutputRoutes)
	server.mux.HandleFunc("GET /api/v1/audit-events", server.listAuditEvents)
	server.mux.HandleFunc("GET /api/v1/accounts", server.listAccounts)
	server.mux.HandleFunc("POST /api/v1/accounts", server.createAccount)
	server.mux.HandleFunc("POST /api/v1/session/password", server.changeOwnPassword)
	server.mux.HandleFunc("POST /api/v1/mapping-previews", server.createMappingPreview)
}

func (server *Server) ServeHTTP(response http.ResponseWriter, request *http.Request) {
	setSecurityHeaders(response.Header())
	server.mux.ServeHTTP(response, request)
}

func setSecurityHeaders(header http.Header) {
	header.Set("Cache-Control", "no-store")
	header.Set("Content-Security-Policy",
		"default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; "+
			"object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'")
	header.Set("X-Content-Type-Options", "nosniff")
	header.Set("X-Frame-Options", "DENY")
	header.Set("Referrer-Policy", "same-origin")
}

func (server *Server) root(response http.ResponseWriter, request *http.Request) {
	http.Redirect(response, request, "/status", http.StatusSeeOther)
}

func (server *Server) loginPage(response http.ResponseWriter, _ *http.Request) {
	response.Header().Set("Content-Type", "text/html; charset=utf-8")
	if err := server.templates.ExecuteTemplate(response, "login.html", nil); err != nil {
		http.Error(response, "画面を表示できません", http.StatusInternalServerError)
	}
}

func (server *Server) loginForm(response http.ResponseWriter, request *http.Request) {
	if !server.validOrigin(request) {
		http.Error(response, "この接続元からログインできません。", http.StatusForbidden)
		return
	}
	request.Body = http.MaxBytesReader(response, request.Body, maxJSONBodyBytes)
	if err := request.ParseForm(); err != nil {
		http.Error(response, "入力内容を確認してください。", http.StatusBadRequest)
		return
	}
	session, err := server.sessions.Login(
		request.Context(),
		requestSource(request),
		request.Form.Get("login_id"),
		request.Form.Get("password"),
	)
	if err != nil {
		response.Header().Set("Content-Type", "text/html; charset=utf-8")
		response.WriteHeader(http.StatusUnauthorized)
		_ = server.templates.ExecuteTemplate(response, "login.html", struct {
			Error string
		}{Error: "ログインIDまたはパスワードが正しくありません。"})
		return
	}
	server.setSessionCookies(response, session)
	target := "/status"
	if session.Principal.MustChangePassword {
		target = "/password"
	}
	http.Redirect(response, request, target, http.StatusSeeOther)
}

func (server *Server) logoutForm(response http.ResponseWriter, request *http.Request) {
	auth, err := server.authenticate(request)
	if err != nil {
		server.clearSessionCookies(response)
		http.Redirect(response, request, "/login", http.StatusSeeOther)
		return
	}
	if !server.authorizeMutation(response, request, auth.token) {
		return
	}
	if err := server.sessions.Logout(request.Context(), auth.token); err != nil {
		http.Error(response, "ログアウトできませんでした。もう一度お試しください。",
			http.StatusInternalServerError)
		return
	}
	server.clearSessionCookies(response)
	http.Redirect(response, request, "/login", http.StatusSeeOther)
}

func (server *Server) statusPage(response http.ResponseWriter, request *http.Request) {
	server.consolePage(response, request)
}

func (server *Server) stylesheet(response http.ResponseWriter, _ *http.Request) {
	content, err := assets.ReadFile("static/site.css")
	if err != nil {
		http.NotFound(response, nil)
		return
	}
	response.Header().Set("Content-Type", "text/css; charset=utf-8")
	_, _ = response.Write(content)
}

func (server *Server) consoleScript(response http.ResponseWriter, _ *http.Request) {
	content, err := assets.ReadFile("static/console.js")
	if err != nil {
		http.NotFound(response, nil)
		return
	}
	response.Header().Set("Content-Type", "text/javascript; charset=utf-8")
	_, _ = response.Write(content)
}

func (server *Server) brandMark(response http.ResponseWriter, _ *http.Request) {
	content, err := assets.ReadFile("static/pinkietech-mark.svg")
	if err != nil {
		http.NotFound(response, nil)
		return
	}
	response.Header().Set("Content-Type", "image/svg+xml")
	_, _ = response.Write(content)
}

func (server *Server) createSession(response http.ResponseWriter, request *http.Request) {
	if !server.validOrigin(request) {
		server.writeError(response, http.StatusForbidden, "origin_forbidden",
			"この接続元からログインできません。", nil)
		return
	}
	var input struct {
		LoginID  string `json:"login_id"`
		Password string `json:"password"`
	}
	if err := decodeJSON(response, request, &input); err != nil {
		server.writeError(response, http.StatusBadRequest, "invalid_request",
			"入力内容を確認してください。", nil)
		return
	}
	session, err := server.sessions.Login(
		request.Context(),
		requestSource(request),
		input.LoginID,
		input.Password,
	)
	if err != nil {
		status := http.StatusUnauthorized
		code := "invalid_credentials"
		message := "ログインIDまたはパスワードが正しくありません。"
		if errors.Is(err, sitesession.ErrRateLimited) ||
			errors.Is(err, sitesession.ErrBusy) {
			status = http.StatusTooManyRequests
			code = "login_rate_limited"
			message = "ログインを続けて試行できません。しばらく待ってください。"
		}
		server.writeError(response, status, code, message, nil)
		return
	}
	account, err := server.store.GetSiteAccount(request.Context(), session.Principal.AccountRef)
	if err != nil {
		server.writeError(response, http.StatusInternalServerError, "internal_error",
			"ログイン処理を完了できません。", nil)
		return
	}
	server.setSessionCookies(response, session)
	response.Header().Set("Content-Type", "application/json; charset=utf-8")
	response.WriteHeader(http.StatusCreated)
	_ = json.NewEncoder(response).Encode(struct {
		CSRFToken string          `json:"csrf_token"`
		Account   siteapp.Account `json:"account"`
	}{
		CSRFToken: session.CSRFToken,
		Account:   account,
	})
}

func (server *Server) getSession(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAPIAuth(response, request, true)
	if !ok {
		return
	}
	writeJSON(response, http.StatusOK, struct {
		Account siteapp.Account `json:"account"`
	}{Account: auth.account})
}

func (server *Server) deleteSession(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAPIAuth(response, request, true)
	if !ok {
		return
	}
	if !server.authorizeMutation(response, request, auth.token) {
		return
	}
	if err := server.sessions.Logout(request.Context(), auth.token); err != nil {
		server.writeError(response, http.StatusUnauthorized, "unauthenticated",
			"ログインが必要です。", nil)
		return
	}
	server.clearSessionCookies(response)
	response.WriteHeader(http.StatusNoContent)
}

func (server *Server) listDevices(response http.ResponseWriter, request *http.Request) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	page, err := inventoryPage(request)
	if err != nil {
		server.writeError(response, http.StatusBadRequest, "invalid_page",
			"一覧の指定が正しくありません。", nil)
		return
	}
	devices, err := server.site.ListDevices(request.Context(), page)
	if err != nil {
		server.writeError(response, http.StatusInternalServerError, "internal_error",
			"デバイス一覧を取得できません。", nil)
		return
	}
	writeJSON(response, http.StatusOK, struct {
		Items []siteapp.DeviceSummary `json:"items"`
	}{Items: devices})
}

func (server *Server) listSignals(response http.ResponseWriter, request *http.Request) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	page, err := inventoryPage(request)
	if err != nil {
		server.writeError(response, http.StatusBadRequest, "invalid_page",
			"一覧の指定が正しくありません。", nil)
		return
	}
	signals, err := server.site.ListSignals(request.Context(), page)
	if err != nil {
		server.writeError(response, http.StatusInternalServerError, "internal_error",
			"センサー一覧を取得できません。", nil)
		return
	}
	writeJSON(response, http.StatusOK, struct {
		Items []siteapp.SignalSummary `json:"items"`
	}{Items: signals})
}

func (server *Server) requireBrowserAuth(
	response http.ResponseWriter,
	request *http.Request,
) (requestAuth, bool) {
	auth, err := server.authenticate(request)
	if err != nil || auth.principal.MustChangePassword {
		http.Redirect(response, request, "/login", http.StatusSeeOther)
		return requestAuth{}, false
	}
	return auth, true
}

func (server *Server) requireAPIAuth(
	response http.ResponseWriter,
	request *http.Request,
	allowPasswordChangeRequired bool,
) (requestAuth, bool) {
	auth, err := server.authenticate(request)
	if err != nil {
		server.writeError(response, http.StatusUnauthorized, "unauthenticated",
			"ログインが必要です。", nil)
		return requestAuth{}, false
	}
	if auth.principal.MustChangePassword && !allowPasswordChangeRequired {
		server.writeError(response, http.StatusForbidden, "password_change_required",
			"先にパスワードを変更してください。", nil)
		return requestAuth{}, false
	}
	return auth, true
}

func (server *Server) authenticate(request *http.Request) (requestAuth, error) {
	cookie, err := request.Cookie(sessionCookieName)
	if err != nil {
		return requestAuth{}, sitesession.ErrUnauthenticated
	}
	principal, err := server.sessions.Authenticate(request.Context(), cookie.Value)
	if err != nil {
		return requestAuth{}, err
	}
	account, err := server.store.GetSiteAccount(request.Context(), principal.AccountRef)
	if err != nil {
		return requestAuth{}, err
	}
	return requestAuth{principal: principal, token: cookie.Value, account: account}, nil
}

func (server *Server) authorizeMutation(
	response http.ResponseWriter,
	request *http.Request,
	sessionToken string,
) bool {
	if !server.validOrigin(request) {
		server.writeError(response, http.StatusForbidden, "origin_forbidden",
			"この接続元から変更できません。", nil)
		return false
	}
	csrf := request.Header.Get("X-CSRF-Token")
	if csrf == "" {
		csrf = request.FormValue("_csrf")
	}
	if !server.sessions.ValidateSessionCSRF(request.Context(), sessionToken, csrf) {
		server.writeError(response, http.StatusForbidden, "csrf_forbidden",
			"画面を再読み込みして、もう一度操作してください。", nil)
		return false
	}
	return true
}

func (server *Server) validOrigin(request *http.Request) bool {
	if origin := request.Header.Get("Origin"); origin != "" {
		return origin == server.publicOrigin
	}
	referer, err := url.Parse(request.Referer())
	if err != nil || referer.Scheme == "" || referer.Host == "" {
		return false
	}
	return referer.Scheme+"://"+referer.Host == server.publicOrigin
}

func (server *Server) setSessionCookies(response http.ResponseWriter, session sitesession.Session) {
	http.SetCookie(response, &http.Cookie{
		Name:     sessionCookieName,
		Value:    session.Token,
		Path:     "/",
		MaxAge:   int((24 * 60 * 60)),
		Secure:   server.secureCookies,
		HttpOnly: true,
		SameSite: http.SameSiteStrictMode,
	})
	http.SetCookie(response, &http.Cookie{
		Name:     csrfCookieName,
		Value:    session.CSRFToken,
		Path:     "/",
		MaxAge:   int((24 * 60 * 60)),
		Secure:   server.secureCookies,
		HttpOnly: false,
		SameSite: http.SameSiteStrictMode,
	})
}

func (server *Server) clearSessionCookies(response http.ResponseWriter) {
	for _, name := range []string{sessionCookieName, csrfCookieName} {
		http.SetCookie(response, &http.Cookie{
			Name:     name,
			Value:    "",
			Path:     "/",
			MaxAge:   -1,
			Secure:   server.secureCookies,
			HttpOnly: name == sessionCookieName,
			SameSite: http.SameSiteStrictMode,
		})
	}
}

func (server *Server) writeError(
	response http.ResponseWriter,
	status int,
	code string,
	message string,
	field *string,
) {
	writeJSON(response, status, errorEnvelope{Error: apiError{
		Code:      code,
		Message:   message,
		Field:     field,
		RequestID: newRequestID(),
	}})
}

func writeJSON(response http.ResponseWriter, status int, value any) {
	response.Header().Set("Content-Type", "application/json; charset=utf-8")
	response.WriteHeader(status)
	_ = json.NewEncoder(response).Encode(value)
}

func decodeJSON(response http.ResponseWriter, request *http.Request, target any) error {
	request.Body = http.MaxBytesReader(response, request.Body, maxJSONBodyBytes)
	decoder := json.NewDecoder(request.Body)
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(target); err != nil {
		return err
	}
	if err := decoder.Decode(&struct{}{}); !errors.Is(err, io.EOF) {
		return errors.New("request body must contain one JSON value")
	}
	return nil
}

func inventoryPage(request *http.Request) (siteapp.PageRequest, error) {
	limit := 50
	if raw := request.URL.Query().Get("limit"); raw != "" {
		parsed, err := strconv.Atoi(raw)
		if err != nil {
			return siteapp.PageRequest{}, err
		}
		limit = parsed
	}
	return siteapp.PageRequest{
		Limit:    limit,
		AfterRef: request.URL.Query().Get("after"),
	}, nil
}

func requestSource(request *http.Request) string {
	host, _, err := net.SplitHostPort(request.RemoteAddr)
	if err == nil {
		return host
	}
	return request.RemoteAddr
}

func newRequestID() string {
	value := make([]byte, 8)
	if _, err := rand.Read(value); err != nil {
		return "req_unavailable"
	}
	return "req_" + hex.EncodeToString(value)
}
