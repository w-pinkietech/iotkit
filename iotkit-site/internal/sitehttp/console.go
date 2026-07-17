package sitehttp

import (
	"crypto/x509"
	"encoding/pem"
	"net/http"
	"os"
	"strconv"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/store"
)

type consoleData struct {
	Page               string
	Title              string
	DisplayName        string
	Role               siteapp.AccountRole
	IsAdmin            bool
	IsOwner            bool
	CSRF               string
	Devices            []siteapp.DeviceSummary
	Signals            []siteapp.SignalSummary
	Definitions        []semantics.Definition
	Outputs            []store.YokaKitRoute
	Audit              []siteapp.AuditEvent
	Accounts           []siteapp.Account
	Raw                any
	Certificate        certificateStatus
	ProjectionFailures int64
}

type certificateStatus struct {
	Available     bool
	DaysRemaining int
	NotAfter      string
	NeedsAction   bool
}

func (server *Server) consolePage(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserAuth(response, request)
	if !ok {
		return
	}
	page := request.URL.Path[1:]
	if page == "" {
		page = "status"
	}
	titles := map[string]string{
		"status": "現場の状態", "monitor": "モニター", "devices": "デバイス",
		"signals": "信号", "logs": "受信ログ", "output": "外部出力",
		"audit": "変更履歴", "accounts": "アカウント",
	}
	title, exists := titles[page]
	if !exists {
		http.NotFound(response, request)
		return
	}
	data := consoleData{
		Page: page, Title: title, DisplayName: auth.account.DisplayName,
		Role: auth.account.Role,
		IsAdmin: auth.account.Role == siteapp.AccountRoleAdmin ||
			auth.account.Role == siteapp.AccountRoleSystemAdmin,
		IsOwner: auth.account.Role == siteapp.AccountRoleSystemAdmin,
	}
	data.Certificate = server.readCertificateStatus()
	if cookie, err := request.Cookie(csrfCookieName); err == nil {
		data.CSRF = cookie.Value
	}
	var err error
	switch page {
	case "status", "monitor", "signals":
		data.Signals, err = server.site.ListSignals(
			request.Context(), siteapp.PageRequest{Limit: 100},
		)
		if err == nil && page == "signals" {
			data.Definitions, err = server.semantics.List(request.Context())
		}
		if err == nil && page == "status" {
			data.ProjectionFailures, err = server.store.SemanticProjectionFailureCount(
				request.Context(),
			)
		}
	case "devices":
		data.Devices, err = server.site.ListDevices(
			request.Context(), siteapp.PageRequest{Limit: 100},
		)
	case "logs":
		data.Raw, err = server.store.ListRawRecords(request.Context(), 100)
	case "output":
		data.Outputs, err = server.store.ListYokaKitRoutes(request.Context())
		if err == nil {
			data.Definitions, err = server.semantics.List(request.Context())
		}
	case "audit":
		data.Audit, err = server.site.ListAuditEvents(request.Context(), 100)
	case "accounts":
		if data.IsOwner {
			data.Accounts, err = server.accounts.ListAccounts(
				request.Context(), server.actor(auth),
			)
		}
	}
	if err != nil {
		http.Error(response, "画面の情報を取得できません", http.StatusInternalServerError)
		return
	}
	response.Header().Set("Content-Type", "text/html; charset=utf-8")
	if err := server.templates.ExecuteTemplate(response, "console.html", data); err != nil {
		http.Error(response, "画面を表示できません", http.StatusInternalServerError)
	}
}

func (server *Server) readCertificateStatus() certificateStatus {
	if server.certificateFile == "" {
		return certificateStatus{}
	}
	encoded, err := os.ReadFile(server.certificateFile)
	if err != nil {
		return certificateStatus{NeedsAction: true}
	}
	block, _ := pem.Decode(encoded)
	if block == nil {
		return certificateStatus{NeedsAction: true}
	}
	certificate, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		return certificateStatus{NeedsAction: true}
	}
	days := int(time.Until(certificate.NotAfter).Hours() / 24)
	return certificateStatus{
		Available: true, DaysRemaining: days,
		NotAfter:    certificate.NotAfter.Local().Format("2006-01-02 15:04"),
		NeedsAction: days < 30,
	}
}

func (server *Server) consoleDeviceProfile(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	expected := formRevision(request)
	_, err := server.site.Dispatch(request.Context(), server.actor(auth), siteapp.UpdateDeviceProfile{
		DeviceRef: request.PathValue("device_ref"),
		Input: siteapp.DeviceProfileInput{
			DisplayName: request.FormValue("display_name"),
			Location:    request.FormValue("location"),
		},
		Precondition: siteapp.RevisionPrecondition{Expected: expected},
	})
	server.consoleMutationResult(response, request, "/devices", err)
}

func (server *Server) consoleSignalProfile(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	_, err := server.site.Dispatch(request.Context(), server.actor(auth), siteapp.UpdateSignalProfile{
		SignalRef: request.PathValue("signal_ref"),
		Input: siteapp.SignalProfileInput{
			DisplayName: request.FormValue("display_name"),
		},
		Precondition: siteapp.RevisionPrecondition{Expected: formRevision(request)},
	})
	server.consoleMutationResult(response, request, "/signals", err)
}

func (server *Server) consoleSemantic(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	scale, _ := strconv.ParseFloat(request.FormValue("scale"), 64)
	offset, _ := strconv.ParseFloat(request.FormValue("offset"), 64)
	threshold, _ := strconv.ParseFloat(request.FormValue("threshold"), 64)
	hysteresis, _ := strconv.ParseFloat(request.FormValue("hysteresis"), 64)
	spec := semantics.DefinitionSpec{
		Kind: semantics.Kind(request.FormValue("kind")), Scale: scale, Offset: offset,
		Condition: semantics.Condition{
			Mode:      semantics.ConditionMode(request.FormValue("condition")),
			BoolValue: request.FormValue("bool_value") != "false",
			Threshold: threshold, Hysteresis: hysteresis,
		},
		Trigger: semantics.TriggerMode(request.FormValue("trigger")),
	}
	_, err := server.semantics.Put(
		request.Context(), server.actor(auth), request.PathValue("signal_ref"),
		spec, siteapp.RevisionPrecondition{Expected: formRevision(request)},
	)
	server.consoleMutationResult(response, request, "/signals", err)
}

func (server *Server) consoleYokaKitOutput(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	_, err := server.store.ApplyYokaKitRoute(
		request.Context(), server.actor(auth), request.FormValue("definition_id"),
		outputadapter.YokaKit{
			SourceID: request.FormValue("source_id"),
			SignalID: request.FormValue("signal_id"),
			Kind:     outputadapter.YokaKitKind(request.FormValue("kind")),
			Reason:   request.FormValue("reason"),
		},
	)
	server.consoleMutationResult(response, request, "/output", err)
}

func (server *Server) consoleAccount(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserMutation(response, request, false)
	if !ok {
		return
	}
	_, err := server.accounts.DispatchAccount(
		request.Context(), server.actor(auth), siteapp.CreateAccount{
			LoginID:           request.FormValue("login_id"),
			DisplayName:       request.FormValue("display_name"),
			Role:              siteapp.AccountRole(request.FormValue("role")),
			TemporaryPassword: request.FormValue("temporary_password"),
		},
	)
	server.consoleMutationResult(response, request, "/accounts", err)
}

func (server *Server) requireBrowserMutation(
	response http.ResponseWriter,
	request *http.Request,
	adminOnly bool,
) (requestAuth, bool) {
	auth, ok := server.requireBrowserAuth(response, request)
	if !ok {
		return requestAuth{}, false
	}
	if !server.authorizeMutation(response, request, auth.token) {
		return requestAuth{}, false
	}
	if adminOnly && auth.account.Role == siteapp.AccountRoleViewer {
		http.Error(response, "この操作を行う権限がありません。", http.StatusForbidden)
		return requestAuth{}, false
	}
	return auth, true
}

func (server *Server) consoleMutationResult(
	response http.ResponseWriter,
	request *http.Request,
	target string,
	err error,
) {
	if err != nil {
		http.Error(response, "保存できませんでした。入力内容と更新状況を確認してください。",
			http.StatusBadRequest)
		return
	}
	http.Redirect(response, request, target, http.StatusSeeOther)
}

func formRevision(request *http.Request) *int64 {
	raw := request.FormValue("revision")
	if raw == "" {
		return nil
	}
	value, err := strconv.ParseInt(raw, 10, 64)
	if err != nil {
		value = -1
	}
	return &value
}

func (server *Server) passwordPage(response http.ResponseWriter, request *http.Request) {
	_, err := server.authenticate(request)
	if err != nil {
		http.Redirect(response, request, "/login", http.StatusSeeOther)
		return
	}
	csrf := ""
	if cookie, err := request.Cookie(csrfCookieName); err == nil {
		csrf = cookie.Value
	}
	response.Header().Set("Content-Type", "text/html; charset=utf-8")
	_ = server.templates.ExecuteTemplate(response, "password.html", struct {
		CSRF  string
		Error string
	}{CSRF: csrf, Error: ""})
}

func (server *Server) passwordForm(response http.ResponseWriter, request *http.Request) {
	auth, err := server.authenticate(request)
	if err != nil {
		http.Redirect(response, request, "/login", http.StatusSeeOther)
		return
	}
	if !server.authorizeMutation(response, request, auth.token) {
		return
	}
	_, err = server.accounts.DispatchAccount(
		request.Context(), server.actor(auth), siteapp.ChangeOwnPassword{
			CurrentPassword: request.FormValue("current_password"),
			NewPassword:     request.FormValue("new_password"),
		},
	)
	if err != nil {
		http.Error(response, "パスワードを変更できませんでした。", http.StatusBadRequest)
		return
	}
	_ = server.sessions.Logout(request.Context(), auth.token)
	server.clearSessionCookies(response)
	http.Redirect(response, request, "/login", http.StatusSeeOther)
}
