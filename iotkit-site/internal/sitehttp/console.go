package sitehttp

import (
	"crypto/x509"
	"encoding/json"
	"encoding/pem"
	"errors"
	"net/http"
	"os"
	"strconv"
	"strings"
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
	Edges              []siteapp.Edge
	EdgeRows           []consoleEdgeView
	EquipmentRows      []consoleEquipmentEdgeView
	OrphanDevices      []consoleSetupDeviceView
	EquipmentView      string
	SelectedEdge       *consoleEquipmentEdgeView
	SelectedDevice     *consoleSetupDeviceView
	SelectedDeviceEdge *consoleEquipmentEdgeView
	Signals            []siteapp.SignalSummary
	DeviceRows         []consoleDeviceView
	SignalRows         []consoleSignalView
	SensorView         string
	SelectedSignal     *consoleSignalView
	SetupRows          []consoleSetupDeviceView
	LogRows            []consoleLogView
	AuditRows          []consoleAuditView
	Definitions        []semantics.Definition
	OutputDefinitions  []consoleDefinitionOption
	Outputs            []store.YokaKitRoute
	OutputRows         []consoleOutputView
	OutputRules        []consoleRuleOption
	RuleOutputs        []store.YokaKitRuleRoute
	RuleOutputRows     []consoleRuleOutputView
	OutputRoutes       []siteapp.OutputRoute
	OutputRouteRows    []consoleOutputRouteView
	Audit              []siteapp.AuditEvent
	Accounts           []siteapp.Account
	Certificate        certificateStatus
	ProjectionFailures int64
	ReceivingCount     int
	AttentionCount     int
	UnconfiguredCount  int
	SetupPendingCount  int
	EdgeCount          int
	EdgePendingCount   int
}

type certificateStatus struct {
	Available     bool
	DaysRemaining int
	NotAfter      string
	NeedsAction   bool
}

func (server *Server) consoleSensorsRedirect(
	response http.ResponseWriter,
	request *http.Request,
) {
	if _, ok := server.requireBrowserAuth(response, request); !ok {
		return
	}
	http.Redirect(response, request, "/sensors", http.StatusSeeOther)
}

func (server *Server) consolePage(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserAuth(response, request)
	if !ok {
		return
	}
	page := request.URL.Path[1:]
	if request.PathValue("edge_ref") != "" || request.PathValue("device_ref") != "" {
		page = "equipment"
	}
	if request.PathValue("signal_ref") != "" {
		page = "sensors"
	}
	if page == "" {
		page = "status"
	}
	titles := map[string]string{
		"status": "現場の概要", "monitor": "センサーの現在値", "devices": "デバイス管理",
		"equipment": "機器管理", "setup": "デバイス管理", "edges": "Edge管理",
		"sensors": "センサー一覧",
		"signals": "値の変換",
		"logs":    "受信履歴", "output": "外部出力",
		"audit": "変更履歴", "accounts": "アカウント", "system": "システム",
	}
	descriptions := map[string]string{
		"status":  "現場のセンサーとデータの流れを、ひと目で確認できます。",
		"monitor": "各センサーから最後に届いた値と受信状態を確認します。",
		"equipment": "Edge、デバイス、センサーのつながりを確認し、" +
			"現場で使うための基本情報を設定します。",
		"sensors":  "各センサーの現在値を確認し、詳細から設定できます。",
		"setup":    "デバイスごとに、接続されたセンサーの名前・種類・単位を登録します。",
		"edges":    "Siteへデータを送るEdgeの登録状態と最終通信を確認します。",
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
	case "equipment":
		data.EquipmentView = "list"
		data.Edges, err = server.site.ListEdges(request.Context())
		var setupDevices []siteapp.SetupDevice
		if err == nil {
			setupDevices, err = server.site.ListSetupDevices(
				request.Context(),
				server.actor(auth),
				100,
			)
		}
		if err == nil {
			data.EquipmentRows = newConsoleEquipmentViews(
				data.Edges,
				setupDevices,
				server.now(),
			)
			data.OrphanDevices = newConsoleOrphanDeviceViews(
				data.Edges,
				setupDevices,
				server.now(),
			)
			if edgeRef := request.PathValue("edge_ref"); edgeRef != "" {
				data.EquipmentView = "edge"
				for index := range data.EquipmentRows {
					if data.EquipmentRows[index].EdgeRef == edgeRef {
						data.SelectedEdge = &data.EquipmentRows[index]
						break
					}
				}
				if data.SelectedEdge == nil {
					http.NotFound(response, request)
					return
				}
			}
			if deviceRef := request.PathValue("device_ref"); deviceRef != "" {
				data.EquipmentView = "device"
				for edgeIndex := range data.EquipmentRows {
					for deviceIndex := range data.EquipmentRows[edgeIndex].Devices {
						device := &data.EquipmentRows[edgeIndex].Devices[deviceIndex]
						if device.Device.DeviceRef == deviceRef {
							data.SelectedDevice = device
							data.SelectedDeviceEdge = &data.EquipmentRows[edgeIndex]
							break
						}
					}
					if data.SelectedDevice != nil {
						break
					}
				}
				if data.SelectedDevice == nil {
					for index := range data.OrphanDevices {
						if data.OrphanDevices[index].Device.DeviceRef == deviceRef {
							data.SelectedDevice = &data.OrphanDevices[index]
							break
						}
					}
				}
				if data.SelectedDevice == nil {
					http.NotFound(response, request)
					return
				}
			}
			for _, edge := range data.Edges {
				if edge.State == siteapp.EdgeActive {
					data.EdgeCount++
				} else {
					data.EdgePendingCount++
				}
			}
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
	case "edges":
		data.Edges, err = server.site.ListEdges(request.Context())
		if err == nil {
			data.EdgeRows = newConsoleEdgeViews(data.Edges, server.now())
		}
	case "status", "monitor", "signals", "sensors":
		data.SensorView = "list"
		data.Signals, err = server.site.ListSignals(
			request.Context(), siteapp.PageRequest{Limit: 100},
		)
		if err == nil && (page == "signals" || page == "status" || page == "sensors") {
			data.Definitions, err = server.semantics.List(request.Context())
		}
		if err == nil && page == "status" {
			data.ProjectionFailures, err = server.store.SemanticRuleProjectionFailureCount(
				request.Context(),
			)
			if err == nil {
				data.RuleOutputs, err = server.store.ListYokaKitRuleRoutes(request.Context())
			}
			if err == nil {
				data.Edges, err = server.site.ListEdges(request.Context())
				for _, edge := range data.Edges {
					if edge.State == siteapp.EdgeActive {
						data.EdgeCount++
					} else {
						data.EdgePendingCount++
					}
				}
			}
		}
		data.SignalRows = newConsoleSignalViews(data.Signals, data.Definitions, server.now())
		if err == nil && (page == "status" || page == "signals" || page == "sensors") {
			configurations := make(map[string]semantics.Configuration, len(data.SignalRows))
			for _, signal := range data.SignalRows {
				configuration, configErr := server.store.GetSemanticConfiguration(
					request.Context(),
					signal.SignalRef,
				)
				if configErr != nil {
					err = configErr
					break
				}
				configurations[signal.SignalRef] = configuration
			}
			if err == nil {
				attachConsoleSemanticConfigurations(data.SignalRows, configurations)
			}
		}
		if err == nil && page == "sensors" {
			var devices []siteapp.DeviceSummary
			devices, err = server.site.ListDevices(
				request.Context(),
				siteapp.PageRequest{Limit: 100},
			)
			if err == nil {
				attachConsoleSignalDevices(data.SignalRows, devices)
			}
		}
		if signalRef := request.PathValue("signal_ref"); signalRef != "" {
			data.SensorView = "detail"
			for index := range data.SignalRows {
				if data.SignalRows[index].SignalRef == signalRef {
					data.SelectedSignal = &data.SignalRows[index]
					break
				}
			}
			if data.SelectedSignal == nil {
				http.NotFound(response, request)
				return
			}
			data.Title = data.SelectedSignal.Name
			data.Description = "現在値と受信元を確認し、このセンサーの設定を管理します。"
		}
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
		data.OutputRoutes, err = server.store.ListOutputRoutes(request.Context())
		if err == nil {
			data.Signals, err = server.site.ListSignals(
				request.Context(), siteapp.PageRequest{Limit: 100},
			)
		}
		if err == nil {
			data.SignalRows = newConsoleSignalViews(data.Signals, nil, server.now())
			signalNames := make(map[string]string, len(data.SignalRows))
			for _, signal := range data.SignalRows {
				signalNames[signal.SignalRef] = signal.Name
				configuration, configErr := server.store.GetSemanticConfiguration(
					request.Context(),
					signal.SignalRef,
				)
				if configErr != nil {
					err = configErr
					break
				}
				for _, rule := range configuration.Rules {
					data.OutputRules = append(data.OutputRules, consoleRuleOption{
						ID:   rule.ID,
						Name: signalNames[rule.SignalRef] + " — " + rule.DisplayName,
						Kind: displaySemanticKind(rule.Kind),
					})
				}
			}
		}
		if err == nil {
			data.OutputRouteRows = newConsoleOutputRouteViews(
				data.OutputRoutes,
				data.OutputRules,
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
		data.OutputRoutes, err = server.store.ListOutputRoutes(request.Context())
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
	for _, signal := range data.SignalRows {
		switch signal.StatusClass {
		case "receiving":
			data.ReceivingCount++
		case "stale", "never":
			data.AttentionCount++
		}
	}
	data.AttentionCount += int(data.ProjectionFailures)
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

func (server *Server) consoleEdgeActivation(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	_, err := server.site.Dispatch(
		request.Context(),
		server.actor(auth),
		siteapp.ActivateEdge{
			EdgeRef: request.PathValue("edge_ref"),
			Precondition: siteapp.RevisionPrecondition{
				Expected: formRevision(request),
			},
		},
	)
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(request, "/edges"),
		err,
	)
}

func (server *Server) consoleSignalProfile(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	displayUnitMode := request.FormValue("display_unit_mode")
	displayUnit := request.FormValue("display_unit")
	if displayUnitMode == "dimensionless" {
		displayUnit = ""
	}
	_, err := server.site.Dispatch(request.Context(), server.actor(auth), siteapp.UpdateSignalProfile{
		SignalRef: request.PathValue("signal_ref"),
		Input: siteapp.SignalProfileInput{
			DisplayName:            request.FormValue("display_name"),
			DisplaySensorType:      request.FormValue("display_sensor_type"),
			DisplaySensorTypeLabel: request.FormValue("display_sensor_type_label"),
			DisplayValueKind:       request.FormValue("display_value_kind"),
			DisplayUnitMode:        displayUnitMode,
			DisplayUnit:            displayUnit,
			DecimalPlaces:          formInt(request, "decimal_places"),
		},
		Precondition: siteapp.RevisionPrecondition{Expected: formRevision(request)},
	})
	server.consoleMutationResult(response, request, consoleReturnTarget(request, "/signals"), err)
}

func consoleReturnTarget(request *http.Request, fallback string) string {
	target := request.FormValue("return_to")
	if safeEquipmentReturnTarget(target) {
		return target
	}
	if safeSensorReturnTarget(target) {
		return target
	}
	switch target {
	case "/equipment":
		return "/equipment"
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

func safeSensorReturnTarget(target string) bool {
	if strings.ContainsAny(target, "?#\\") || strings.HasPrefix(target, "//") {
		return false
	}
	parts := strings.Split(strings.TrimPrefix(target, "/"), "/")
	return len(parts) == 2 &&
		parts[0] == "sensors" &&
		validConsoleResourceRef(parts[1], "sig_")
}

func safeEquipmentReturnTarget(target string) bool {
	if target == "/equipment" {
		return true
	}
	if strings.ContainsAny(target, "?#\\") || strings.HasPrefix(target, "//") {
		return false
	}
	parts := strings.Split(strings.TrimPrefix(target, "/"), "/")
	if len(parts) != 3 || parts[0] != "equipment" || parts[2] == "" {
		return false
	}
	switch parts[1] {
	case "edges":
		return validConsoleResourceRef(parts[2], "edge_")
	case "devices":
		return validConsoleResourceRef(parts[2], "dev_")
	default:
		return false
	}
}

func validConsoleResourceRef(value, prefix string) bool {
	if len(value) != len(prefix)+32 || !strings.HasPrefix(value, prefix) {
		return false
	}
	for _, character := range value[len(prefix):] {
		if (character < '0' || character > '9') &&
			(character < 'a' || character > 'f') {
			return false
		}
	}
	return true
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
	riseThreshold, _ := strconv.ParseFloat(request.FormValue("rise_threshold"), 64)
	fallThreshold, _ := strconv.ParseFloat(request.FormValue("fall_threshold"), 64)
	riseDebounceSeconds, _ := strconv.ParseFloat(
		request.FormValue("rise_debounce_seconds"), 64,
	)
	fallDebounceSeconds, _ := strconv.ParseFloat(
		request.FormValue("fall_debounce_seconds"), 64,
	)
	spec := semantics.DefinitionSpec{
		Kind: semantics.Kind(request.FormValue("kind")), Scale: scale, Offset: offset,
		Detector: semantics.Detector{
			Mode:          semantics.DetectorMode(request.FormValue("detector_mode")),
			RiseThreshold: riseThreshold, FallThreshold: fallThreshold,
			RiseDebounceMS: int64(riseDebounceSeconds * 1000),
			FallDebounceMS: int64(fallDebounceSeconds * 1000),
		},
		Trigger: semantics.TriggerMode(request.FormValue("trigger")),
	}
	_, err := server.semantics.Put(
		request.Context(), server.actor(auth), request.PathValue("signal_ref"),
		spec, siteapp.RevisionPrecondition{Expected: formRevision(request)},
	)
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(
			request,
			"/sensors/"+request.PathValue("signal_ref"),
		),
		err,
	)
}

func (server *Server) consoleSemanticCounterReset(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	_, err := server.semantics.ResetCounter(
		request.Context(),
		server.actor(auth),
		request.PathValue("signal_ref"),
		siteapp.RevisionPrecondition{Expected: formRevision(request)},
	)
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(
			request,
			"/sensors/"+request.PathValue("signal_ref"),
		),
		err,
	)
}

func (server *Server) deprecatedConsoleSemanticMutation(
	response http.ResponseWriter,
	request *http.Request,
) {
	if _, ok := server.requireBrowserMutation(response, request, true); !ok {
		return
	}
	http.Error(
		response,
		"画面を再読み込みして、ルールごとの設定を使用してください。",
		http.StatusGone,
	)
}

func (server *Server) consoleSignalCalibration(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	revision, ok := requireConsoleRevision(response, request)
	if !ok {
		return
	}
	scale, scaleErr := strconv.ParseFloat(request.FormValue("scale"), 64)
	offset, offsetErr := strconv.ParseFloat(request.FormValue("offset"), 64)
	var err error
	if scaleErr != nil || offsetErr != nil {
		err = semantics.Calibration{}.Validate()
	} else {
		_, err = server.semanticConfig.UpdateCalibration(
			request.Context(),
			server.actor(auth),
			request.PathValue("signal_ref"),
			scale,
			offset,
			siteapp.RevisionPrecondition{Expected: revision},
		)
	}
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(
			request,
			"/sensors/"+request.PathValue("signal_ref"),
		),
		err,
	)
}

func (server *Server) consoleSemanticRuleCreate(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	revision, ok := requireConsoleRevision(response, request)
	if !ok {
		return
	}
	spec, err := semanticRuleSpecFromForm(request)
	if err == nil {
		_, err = server.semanticConfig.CreateRule(
			request.Context(),
			server.actor(auth),
			request.PathValue("signal_ref"),
			request.FormValue("display_name"),
			spec,
			siteapp.RevisionPrecondition{Expected: revision},
		)
	}
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(
			request,
			"/sensors/"+request.PathValue("signal_ref"),
		),
		err,
	)
}

func (server *Server) consoleSemanticRuleUpdate(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	revision, ok := requireConsoleRevision(response, request)
	if !ok {
		return
	}
	spec, err := semanticRuleSpecFromForm(request)
	if err == nil {
		_, err = server.semanticConfig.UpdateRule(
			request.Context(),
			server.actor(auth),
			request.PathValue("rule_id"),
			request.FormValue("display_name"),
			spec,
			siteapp.RevisionPrecondition{Expected: revision},
		)
	}
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(request, "/sensors"),
		err,
	)
}

func (server *Server) consoleSemanticRuleRetire(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	revision, ok := requireConsoleRevision(response, request)
	if !ok {
		return
	}
	_, err := server.semanticConfig.RetireRule(
		request.Context(),
		server.actor(auth),
		request.PathValue("rule_id"),
		siteapp.RevisionPrecondition{Expected: revision},
	)
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(request, "/sensors"),
		err,
	)
}

func (server *Server) consoleSemanticRuleCounterReset(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	resetID := request.FormValue("reset_id")
	if resetID == "" {
		resetID = "console_" + newRequestID()
	}
	_, err := server.semanticConfig.RequestCounterReset(
		request.Context(),
		server.actor(auth),
		request.PathValue("rule_id"),
		resetID,
	)
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(request, "/sensors"),
		err,
	)
}

func semanticRuleSpecFromForm(request *http.Request) (semantics.RuleSpec, error) {
	riseThreshold, riseThresholdErr := strconv.ParseFloat(
		request.FormValue("rise_threshold"),
		64,
	)
	fallThreshold, fallThresholdErr := strconv.ParseFloat(
		request.FormValue("fall_threshold"),
		64,
	)
	riseDebounceSeconds, riseDebounceErr := strconv.ParseFloat(
		request.FormValue("rise_debounce_seconds"),
		64,
	)
	fallDebounceSeconds, fallDebounceErr := strconv.ParseFloat(
		request.FormValue("fall_debounce_seconds"),
		64,
	)
	if request.FormValue("rise_threshold") == "" {
		riseThreshold, riseThresholdErr = 0, nil
	}
	if request.FormValue("fall_threshold") == "" {
		fallThreshold, fallThresholdErr = 0, nil
	}
	if request.FormValue("rise_debounce_seconds") == "" {
		riseDebounceSeconds, riseDebounceErr = 0, nil
	}
	if request.FormValue("fall_debounce_seconds") == "" {
		fallDebounceSeconds, fallDebounceErr = 0, nil
	}
	if riseThresholdErr != nil || fallThresholdErr != nil ||
		riseDebounceErr != nil || fallDebounceErr != nil {
		return semantics.RuleSpec{}, errors.New("invalid semantic rule number")
	}
	spec := semantics.RuleSpec{
		Kind: semantics.Kind(request.FormValue("kind")),
		Detector: semantics.Detector{
			Mode:           semantics.DetectorMode(request.FormValue("detector_mode")),
			RiseThreshold:  riseThreshold,
			FallThreshold:  fallThreshold,
			RiseDebounceMS: int64(riseDebounceSeconds * 1000),
			FallDebounceMS: int64(fallDebounceSeconds * 1000),
		},
		Trigger: semantics.TriggerMode(request.FormValue("trigger")),
	}
	if err := spec.Validate(); err != nil {
		return semantics.RuleSpec{}, err
	}
	return spec, nil
}

func (server *Server) consoleYokaKitOutput(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	adapter := outputadapter.YokaKitConfig{
		SourceID: request.FormValue("source_id"),
		SignalID: request.FormValue("signal_id"),
		Kind:     outputadapter.YokaKitKind(request.FormValue("kind")),
		Reason:   request.FormValue("reason"),
	}
	_, err := server.ruleOutputs.CreateYokaKitRoute(
		request.Context(),
		server.actor(auth),
		request.FormValue("rule_id"),
		adapter,
	)
	server.consoleMutationResult(response, request, "/output", err)
}

func (server *Server) consoleOutputRoute(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	adapterID := request.FormValue("adapter_id")
	var config json.RawMessage
	var err error
	switch adapterID {
	case "iotkit.mqtt-json.v1":
		config, err = outputadapter.EncodeGenericMQTTJSONConfig(
			outputadapter.GenericMQTTJSONConfig{
				Topic: request.FormValue("topic"),
			},
		)
	case "yokakit.mqtt.v1":
		config, err = outputadapter.EncodeYokaKitConfig(
			outputadapter.YokaKitConfig{
				SourceID: request.FormValue("source_id"),
				SignalID: request.FormValue("signal_id"),
				Kind: outputadapter.YokaKitKind(
					request.FormValue("kind"),
				),
				Reason: request.FormValue("reason"),
			},
		)
	default:
		err = outputadapter.ErrInvalidConfiguration
	}
	if err == nil {
		_, err = server.ruleOutputs.CreateOutputRoute(
			request.Context(),
			server.actor(auth),
			request.FormValue("rule_id"),
			adapterID,
			config,
		)
	}
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

func requireConsoleRevision(
	response http.ResponseWriter,
	request *http.Request,
) (*int64, bool) {
	revision := formRevision(request)
	if revision == nil {
		http.Error(
			response,
			"画面を再読み込みして、もう一度操作してください。",
			http.StatusPreconditionFailed,
		)
		return nil, false
	}
	return revision, true
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
