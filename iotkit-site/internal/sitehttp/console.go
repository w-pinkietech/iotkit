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
	Description        string
	Notice             string
	PageError          string
	DisplayName        string
	Role               siteapp.AccountRole
	RoleLabel          string
	IsAdmin            bool
	IsOwner            bool
	CSRF               string
	Devices            []siteapp.DeviceSummary
	Signals            []siteapp.SignalSummary
	DeviceRows         []consoleDeviceView
	SignalRows         []consoleSignalView
	SetupRows          []consoleSetupDeviceView
	LogRows            []consoleLogView
	AuditRows          []consoleAuditView
	Definitions        []semantics.Definition
	OutputDefinitions  []consoleDefinitionOption
	Outputs            []store.YokaKitRoute
	OutputRows         []consoleOutputView
	Audit              []siteapp.AuditEvent
	Accounts           []siteapp.Account
	Certificate        certificateStatus
	ProjectionFailures int64
	ReceivingCount     int
	AttentionCount     int
	UnconfiguredCount  int
	SetupPendingCount  int
	EdgeCount          int
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
		"status": "現場の概要", "monitor": "センサーの現在値", "devices": "デバイス管理",
		"setup": "デバイス管理", "signals": "センサー設定",
		"logs": "受信履歴", "output": "外部出力",
		"audit": "変更履歴", "accounts": "アカウント", "system": "システム",
	}
	descriptions := map[string]string{
		"status":   "現場のセンサーとデータの流れを、ひと目で確認できます。",
		"monitor":  "各センサーから最後に届いた値と受信状態を確認します。",
		"setup":    "デバイスごとに、接続されたセンサーの名前・種類・単位を登録します。",
		"devices":  "現場に設置したデバイスの名前と場所を管理します。",
		"signals":  "センサーから届く値の補正・判定・累積方法を設定します。",
		"logs":     "Siteが受け取った直近のデータを時系列で確認します。",
		"output":   "使い方を設定した値が外部アプリへ渡っているか確認します。",
		"audit":    "誰が、いつ、どの設定を変更したか確認します。",
		"accounts": "Consoleへログインできる担当者と権限を管理します。",
		"system":   "Siteと通信証明書の状態を確認します。",
	}
	title, exists := titles[page]
	if !exists {
		http.NotFound(response, request)
		return
	}
	data := consoleData{
		Page: page, Title: title, Description: descriptions[page],
		DisplayName: auth.account.DisplayName,
		Role:        auth.account.Role,
		RoleLabel:   roleLabel(auth.account.Role),
		IsAdmin: auth.account.Role == siteapp.AccountRoleAdmin ||
			auth.account.Role == siteapp.AccountRoleSystemAdmin,
		IsOwner: auth.account.Role == siteapp.AccountRoleSystemAdmin,
	}
	if request.URL.Query().Get("saved") == "1" {
		data.Notice = "変更を保存しました。"
	}
	if request.URL.Query().Get("error") == "save" {
		data.PageError = "保存できませんでした。入力内容を確認し、画面を再読み込みしてもう一度お試しください。"
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
		if err == nil && (page == "signals" || page == "status") {
			data.Definitions, err = server.semantics.List(request.Context())
		}
		if err == nil && page == "status" {
			data.ProjectionFailures, err = server.store.SemanticProjectionFailureCount(
				request.Context(),
			)
			if err == nil {
				data.Outputs, err = server.store.ListYokaKitRoutes(request.Context())
			}
		}
		data.SignalRows = newConsoleSignalViews(data.Signals, data.Definitions, server.now())
		data.summarizeSignals()
	case "devices":
		data.Devices, err = server.site.ListDevices(
			request.Context(), siteapp.PageRequest{Limit: 100},
		)
		for _, device := range data.Devices {
			data.DeviceRows = append(data.DeviceRows, newConsoleDeviceView(device, server.now()))
		}
	case "setup":
		var setupDevices []siteapp.SetupDevice
		setupDevices, err = server.site.ListSetupDevices(
			request.Context(),
			server.actor(auth),
			100,
		)
		if err == nil {
			data.SetupRows = newConsoleSetupDeviceViews(setupDevices, server.now())
			for _, device := range setupDevices {
				if device.State == siteapp.SetupWaitingForDevice {
					data.SetupPendingCount++
				}
			}
		}
	case "logs":
		var records []store.RawRecord
		records, err = server.store.ListRawRecords(request.Context(), 100)
		if err == nil {
			data.Signals, err = server.site.ListSignals(
				request.Context(), siteapp.PageRequest{Limit: 100},
			)
		}
		if err == nil {
			data.SignalRows = newConsoleSignalViews(data.Signals, nil, server.now())
			data.LogRows = newConsoleLogViews(records, data.SignalRows)
		}
	case "output":
		data.Outputs, err = server.store.ListYokaKitRoutes(request.Context())
		data.OutputRows = newConsoleOutputViews(data.Outputs)
		if err == nil {
			data.Definitions, err = server.semantics.List(request.Context())
		}
		if err == nil {
			data.Signals, err = server.site.ListSignals(
				request.Context(), siteapp.PageRequest{Limit: 100},
			)
		}
		if err == nil {
			data.SignalRows = newConsoleSignalViews(data.Signals, data.Definitions, server.now())
			data.OutputDefinitions = newConsoleDefinitionOptions(
				data.Definitions, data.SignalRows,
			)
		}
	case "audit":
		data.Audit, err = server.site.ListAuditEvents(request.Context(), 100)
		data.AuditRows = newConsoleAuditViews(data.Audit)
	case "accounts":
		if data.IsOwner {
			data.Accounts, err = server.accounts.ListAccounts(
				request.Context(), server.actor(auth),
			)
		}
	case "system":
		data.Outputs, err = server.store.ListYokaKitRoutes(request.Context())
	}
	if err == nil && page == "status" {
		var setupDevices []siteapp.SetupDevice
		setupDevices, err = server.site.ListSetupDevices(
			request.Context(),
			server.actor(auth),
			100,
		)
		if err == nil {
			for _, device := range setupDevices {
				if device.State == siteapp.SetupWaitingForDevice {
					data.SetupPendingCount++
				}
				for _, signal := range device.Signals {
					if !signal.ProfileComplete {
						data.UnconfiguredCount++
					}
				}
			}
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

func (data *consoleData) summarizeSignals() {
	edges := make(map[string]struct{})
	for _, signal := range data.SignalRows {
		edges[signal.Edge] = struct{}{}
		switch signal.StatusClass {
		case "receiving":
			data.ReceivingCount++
		case "stale", "never":
			data.AttentionCount++
		}
	}
	data.AttentionCount += int(data.ProjectionFailures)
	data.EdgeCount = len(edges)
}

func roleLabel(role siteapp.AccountRole) string {
	switch role {
	case siteapp.AccountRoleSystemAdmin:
		return "システム管理者"
	case siteapp.AccountRoleAdmin:
		return "設定管理者"
	default:
		return "閲覧担当者"
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
	server.consoleMutationResult(response, request, consoleReturnTarget(request, "/devices"), err)
}

func (server *Server) consoleSignalProfile(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	_, err := server.site.Dispatch(request.Context(), server.actor(auth), siteapp.UpdateSignalProfile{
		SignalRef: request.PathValue("signal_ref"),
		Input: siteapp.SignalProfileInput{
			DisplayName:            request.FormValue("display_name"),
			DisplaySensorType:      request.FormValue("display_sensor_type"),
			DisplaySensorTypeLabel: request.FormValue("display_sensor_type_label"),
			DisplayValueKind:       request.FormValue("display_value_kind"),
			DisplayUnitMode:        request.FormValue("display_unit_mode"),
			DisplayUnit:            request.FormValue("display_unit"),
			DecimalPlaces:          formInt(request, "decimal_places"),
		},
		Precondition: siteapp.RevisionPrecondition{Expected: formRevision(request)},
	})
	server.consoleMutationResult(response, request, consoleReturnTarget(request, "/signals"), err)
}

func consoleReturnTarget(request *http.Request, fallback string) string {
	switch request.FormValue("return_to") {
	case "/setup":
		return "/setup"
	case "/signals":
		return "/signals"
	case "/devices":
		return "/devices"
	default:
		return fallback
	}
}

func formInt(request *http.Request, name string) int {
	value, _ := strconv.Atoi(request.FormValue(name))
	return value
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
	auth, ok := server.requireBrowserOwnerMutation(response, request)
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

func (server *Server) consoleAccountUpdate(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserOwnerMutation(response, request)
	if !ok {
		return
	}
	revision := formRevision(request)
	if revision == nil {
		http.Error(response, "画面を再読み込みして、もう一度操作してください。",
			http.StatusPreconditionFailed)
		return
	}
	_, err := server.accounts.DispatchAccount(
		request.Context(), server.actor(auth), siteapp.UpdateAccount{
			AccountRef:       request.PathValue("account_ref"),
			DisplayName:      request.FormValue("display_name"),
			Role:             siteapp.AccountRole(request.FormValue("role")),
			ExpectedRevision: *revision,
		},
	)
	server.consoleMutationResult(response, request, "/accounts", err)
}

func (server *Server) consoleAccountDisable(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserOwnerMutation(response, request)
	if !ok {
		return
	}
	revision := formRevision(request)
	if revision == nil {
		http.Error(response, "画面を再読み込みして、もう一度操作してください。",
			http.StatusPreconditionFailed)
		return
	}
	_, err := server.accounts.DispatchAccount(
		request.Context(), server.actor(auth), siteapp.DisableAccount{
			AccountRef:       request.PathValue("account_ref"),
			ExpectedRevision: *revision,
		},
	)
	server.consoleMutationResult(response, request, "/accounts", err)
}

func (server *Server) consoleAccountPassword(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserOwnerMutation(response, request)
	if !ok {
		return
	}
	revision := formRevision(request)
	if revision == nil {
		http.Error(response, "画面を再読み込みして、もう一度操作してください。",
			http.StatusPreconditionFailed)
		return
	}
	_, err := server.accounts.DispatchAccount(
		request.Context(), server.actor(auth), siteapp.ResetAccountPassword{
			AccountRef:        request.PathValue("account_ref"),
			TemporaryPassword: request.FormValue("temporary_password"),
			ExpectedRevision:  *revision,
		},
	)
	server.consoleMutationResult(response, request, "/accounts", err)
}

func (server *Server) requireBrowserOwnerMutation(
	response http.ResponseWriter,
	request *http.Request,
) (requestAuth, bool) {
	auth, ok := server.requireBrowserAuth(response, request)
	if !ok {
		return requestAuth{}, false
	}
	if !server.authorizeMutation(response, request, auth.token) {
		return requestAuth{}, false
	}
	if auth.account.Role != siteapp.AccountRoleSystemAdmin {
		http.Error(response, "この操作を行う権限がありません。", http.StatusForbidden)
		return requestAuth{}, false
	}
	return auth, true
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
		http.Redirect(response, request, target+"?error=save", http.StatusSeeOther)
		return
	}
	http.Redirect(response, request, target+"?saved=1", http.StatusSeeOther)
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
