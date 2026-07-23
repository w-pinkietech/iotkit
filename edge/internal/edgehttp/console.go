package edgehttp

import (
	"crypto/x509"
	"encoding/pem"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
	"net/http"
	"net/url"
	"os"
	"strconv"
	"time"
)

type consoleData struct {
	Page                    string
	NavigationPage          string
	Title                   string
	Description             string
	Notice                  string
	PageError               string
	FeedbackTarget          string
	DisplayName             string
	Role                    edgeapp.AccountRole
	RoleLabel               string
	IsAdmin                 bool
	IsOwner                 bool
	CSRF                    string
	Devices                 []edgeapp.DeviceSummary
	EdgeNodes               []edgeapp.EdgeNode
	EdgeNodeRows            []consoleEdgeNodeView
	EquipmentRows           []consoleEquipmentEdgeNodeView
	OrphanDevices           []consoleSetupDeviceView
	EquipmentView           string
	SelectedEdgeNode        *consoleEquipmentEdgeNodeView
	SelectedDevice          *consoleSetupDeviceView
	SelectedDeviceEdgeNode  *consoleEquipmentEdgeNodeView
	Signals                 []edgeapp.SignalSummary
	DeviceRows              []consoleDeviceView
	SignalRows              []consoleSignalView
	SensorView              string
	SignalSettingsLinks     bool
	SensorSettingsPath      string
	SelectedSignal          *consoleSignalView
	SetupRows               []consoleSetupDeviceView
	LogRows                 []consoleLogView
	HistoryRange            string
	HistoryRangeLabel       string
	HistorySelectedSignal   string
	HistorySelectedEdgeNode string
	HistoryRawExportURL     string
	HistoryExportURL        string
	HistoryHasMore          bool
	HistoryChart            *consoleHistoryChart
	Storage                 consoleStorageView
	Diagnostics             store.DiagnosticReport
	AuditRows               []consoleAuditView
	Definitions             []semantics.Definition
	OutputRules             []consoleRuleOption
	OutputRoutes            []edgeapp.OutputRoute
	OutputRouteRows         []consoleOutputRouteView
	ExportProfiles          []consoleExportProfileView
	AvailableOutputs        []consoleAvailableOutput
	Audit                   []edgeapp.AuditEvent
	Accounts                []edgeapp.Account
	Certificate             certificateStatus
	ProjectionFailures      int64
	ReceivingCount          int
	AttentionCount          int
	UnconfiguredCount       int
	SetupPendingCount       int
	SetupDeviceCount        int
	EdgeNodeCount           int
	EdgeNodePendingCount    int
	SemanticRuleCount       int
	OutputPendingCount      int64
	OutputActiveCount       int
	OutputErrorCount        int
	OutputStalledCount      int
	OutputHealthClass       string
	OutputFlowClass         string
	OutputStatusLabel       string
	OutputStatusDetail      string
	OutputStatusSummary     string
	Onboarding              consoleOnboardingView
}

type certificateStatus struct {
	Available     bool
	DaysRemaining int
	NotAfter      string
	NeedsAction   bool
}

type consoleAvailableOutput struct {
	AdapterID               string
	DisplayName             string
	Description             string
	AutomaticCount          int
	NeedsConfigurationCount int
	IneligibleCount         int
	Rules                   []edgeapp.OutputActivationRulePreview
}

func selectConsoleHistoryScope(
	request *http.Request,
	signals []consoleSignalView,
) (rangeValue, rangeLabel, edgeNodeID, signalRef string) {
	rangeValue = request.URL.Query().Get("range")
	switch rangeValue {
	case "1h":
		rangeLabel = "直近1時間"
	case "24h":
		rangeLabel = "直近24時間"
	case "7d":
		rangeLabel = "直近7日"
	case "30d":
		rangeLabel = "直近30日"
	default:
		rangeValue, rangeLabel = "1h", "直近1時間"
	}
	edgeNodeID = request.URL.Query().Get("edge_node_id")
	requestedSignal := request.URL.Query().Get("signal_ref")
	for _, signal := range signals {
		if signal.SignalRef == requestedSignal &&
			(edgeNodeID == "" || signal.EdgeNodeID == edgeNodeID) {
			return rangeValue, rangeLabel, edgeNodeID, signal.SignalRef
		}
	}
	for _, signal := range signals {
		if signal.Latest != nil && (edgeNodeID == "" || signal.EdgeNodeID == edgeNodeID) {
			return rangeValue, rangeLabel, edgeNodeID, signal.SignalRef
		}
	}
	for _, signal := range signals {
		if edgeNodeID == "" || signal.EdgeNodeID == edgeNodeID {
			return rangeValue, rangeLabel, edgeNodeID, signal.SignalRef
		}
	}
	return rangeValue, rangeLabel, edgeNodeID, ""
}

func consoleHistoryWindow(now time.Time, rangeValue string) (int64, int64) {
	duration := time.Hour
	switch rangeValue {
	case "24h":
		duration = 24 * time.Hour
	case "7d":
		duration = 7 * 24 * time.Hour
	case "30d":
		duration = 30 * 24 * time.Hour
	}
	return now.Add(-duration).UnixMilli(), now.Add(time.Millisecond).UnixMilli()
}

func consoleHistoryExportURL(
	from, until int64,
	signalRef, edgeNodeID string,
) string {
	values := url.Values{
		"from": {strconv.FormatInt(from, 10)},
		"to":   {strconv.FormatInt(until, 10)},
	}
	if signalRef != "" {
		values.Set("signal_ref", signalRef)
	}
	if edgeNodeID != "" {
		values.Set("edge_node_id", edgeNodeID)
	}
	return "/api/v1/history.csv?" + values.Encode()
}

func consoleSemanticHistoryExportURL(
	from, until int64,
	signalRef, edgeNodeID string,
) string {
	values := url.Values{
		"from": {strconv.FormatInt(from, 10)},
		"to":   {strconv.FormatInt(until, 10)},
	}
	if signalRef != "" {
		values.Set("signal_ref", signalRef)
	}
	if edgeNodeID != "" {
		values.Set("edge_node_id", edgeNodeID)
	}
	return "/api/v1/semantic-history.csv?" + values.Encode()
}

func selectedConsoleSignal(
	signals []consoleSignalView,
	signalRef string,
) *consoleSignalView {
	for index := range signals {
		if signals[index].SignalRef == signalRef {
			return &signals[index]
		}
	}
	return nil
}

func findConsoleEquipmentDevice(
	rows []consoleEquipmentEdgeNodeView,
	orphans []consoleSetupDeviceView,
	deviceRef string,
) (*consoleSetupDeviceView, *consoleEquipmentEdgeNodeView) {
	for edgeNodeIndex := range rows {
		for deviceIndex := range rows[edgeNodeIndex].Devices {
			device := &rows[edgeNodeIndex].Devices[deviceIndex]
			if device.Device.DeviceRef == deviceRef {
				return device, &rows[edgeNodeIndex]
			}
		}
	}
	for index := range orphans {
		if orphans[index].Device.DeviceRef == deviceRef {
			return &orphans[index], nil
		}
	}
	return nil, nil
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
	if request.PathValue("edge_node_ref") != "" || request.PathValue("device_ref") != "" {
		page = "equipment"
	}
	if request.PathValue("signal_ref") != "" {
		page = "sensors"
	}
	if page == "" {
		page = "status"
	}
	titles := map[string]string{
		"status": "システム概要", "monitor": "センサーの現在値", "devices": "デバイス管理",
		"equipment": "機器管理", "setup": "デバイス管理", "edge-nodes": "収集ノード管理",
		"sensors": "センサー一覧",
		"signals": "値の変換",
		"logs":    "受信履歴", "output": "外部出力",
		"audit": "変更履歴", "accounts": "アカウント", "system": "システム",
	}
	descriptions := map[string]string{
		"status":  "センサーとデータの流れを、ひと目で確認できます。",
		"monitor": "各センサーから最後に届いた値と受信状態を確認します。",
		"equipment": "収集ノード、デバイス、センサーのつながりを確認し、" +
			"表示名や設置場所を設定します。",
		"sensors":    "各センサーの現在値を確認し、詳細から設定できます。",
		"setup":      "デバイスごとに、接続されたセンサーの名前・種類・単位を登録します。",
		"edge-nodes": "IoTKit Edgeへデータを送る収集ノードの登録状態と最終通信を確認します。",
		"devices":    "接続したデバイスの名前と設置場所を管理します。",
		"signals":    "センサーから届く値の補正・判定・累積方法を設定します。",
		"logs":       "センサーと期間を選び、値の推移・受信履歴・CSVを一画面で確認します。",
		"output":     "使い方を設定した値が外部アプリへ渡っているか確認します。",
		"audit":      "誰が、いつ、どの設定を変更したか確認します。",
		"accounts":   "Consoleへログインできる担当者と権限を管理します。",
		"system":     "IoTKit Edgeと通信証明書の状態を確認します。",
	}
	title, exists := titles[page]
	if !exists {
		http.NotFound(response, request)
		return
	}
	data := consoleData{
		Page: page, NavigationPage: page, Title: title, Description: descriptions[page],
		DisplayName: auth.account.DisplayName,
		Role:        auth.account.Role,
		RoleLabel:   roleLabel(auth.account.Role),
		IsAdmin: auth.account.Role == edgeapp.AccountRoleAdmin ||
			auth.account.Role == edgeapp.AccountRoleSystemAdmin,
		IsOwner: auth.account.Role == edgeapp.AccountRoleSystemAdmin,
	}
	data.SignalSettingsLinks = page == "sensors" && data.IsAdmin
	if request.PathValue("device_ref") != "" &&
		request.PathValue("signal_ref") != "" {
		data.NavigationPage = "equipment"
	}
	feedbackTarget := request.URL.Query().Get("focus")
	if safeConsoleAnchor(feedbackTarget) {
		data.FeedbackTarget = feedbackTarget
	}
	if request.URL.Query().Get("saved") == "1" {
		data.Notice = "変更を保存しました。"
	}
	if errorCode := request.URL.Query().Get("error"); errorCode != "" {
		data.PageError = consoleErrorMessage(errorCode)
	}
	data.Certificate = server.readCertificateStatus()
	if cookie, err := request.Cookie(csrfCookieName); err == nil {
		data.CSRF = cookie.Value
	}
	var err error
	switch page {
	case "equipment":
		data.EquipmentView = "list"
		data.EdgeNodes, err = server.edge.ListEdgeNodes(request.Context())
		var setupDevices []edgeapp.SetupDevice
		if err == nil {
			setupDevices, err = server.edge.ListSetupDevices(
				request.Context(),
				server.actor(auth),
				100,
			)
		}
		if err == nil {
			data.EquipmentRows = newConsoleEquipmentEdgeNodeViews(
				data.EdgeNodes,
				setupDevices,
				server.now(),
			)
			data.OrphanDevices = newConsoleOrphanDeviceViews(
				data.EdgeNodes,
				setupDevices,
				server.now(),
			)
			if edgeNodeRef := request.PathValue("edge_node_ref"); edgeNodeRef != "" {
				data.EquipmentView = "edge_node"
				for index := range data.EquipmentRows {
					if data.EquipmentRows[index].EdgeNodeRef == edgeNodeRef {
						data.SelectedEdgeNode = &data.EquipmentRows[index]
						break
					}
				}
				if data.SelectedEdgeNode == nil {
					http.NotFound(response, request)
					return
				}
			}
			if deviceRef := request.PathValue("device_ref"); deviceRef != "" {
				data.EquipmentView = "device"
				data.SelectedDevice, data.SelectedDeviceEdgeNode = findConsoleEquipmentDevice(
					data.EquipmentRows,
					data.OrphanDevices,
					deviceRef,
				)
				if data.SelectedDevice == nil {
					http.NotFound(response, request)
					return
				}
			}
			for _, edgeNode := range data.EdgeNodes {
				if edgeNode.State == edgeapp.EdgeNodeActive {
					data.EdgeNodeCount++
				} else {
					data.EdgeNodePendingCount++
				}
			}
			for _, device := range setupDevices {
				if device.State == edgeapp.SetupWaitingForDevice {
					data.SetupPendingCount++
				}
				for _, signal := range device.Signals {
					if !signal.ProfileComplete {
						data.UnconfiguredCount++
					}
				}
			}
		}
	case "edge-nodes":
		data.EdgeNodes, err = server.edge.ListEdgeNodes(request.Context())
		if err == nil {
			data.EdgeNodeRows = newConsoleEdgeNodeViews(data.EdgeNodes, server.now())
		}
	case "status", "monitor", "signals", "sensors":
		data.SensorView = "list"
		data.Signals, err = server.edge.ListSignals(
			request.Context(), edgeapp.PageRequest{Limit: 100},
		)
		if err == nil && (page == "signals" || page == "status" || page == "sensors") {
			data.Definitions, err = server.semantics.List(request.Context())
		}
		if err == nil && page == "status" {
			data.ProjectionFailures, err = server.store.SemanticRuleProjectionFailureCount(
				request.Context(),
			)
			if err == nil {
				data.OutputRoutes, err = server.store.ListOutputRoutes(request.Context())
				data.OutputRoutes = profileOutputRoutes(data.OutputRoutes)
				data.summarizeOutputRoutes(server.now())
			}
			if err == nil {
				profiles, profileErr := server.store.ListExportProfiles(
					request.Context(),
				)
				if profileErr != nil {
					err = profileErr
				} else {
					data.ExportProfiles = newConsoleExportProfileViews(
						currentExportProfiles(profiles),
						nil,
						nil,
					)
					data.summarizeExportProfiles()
				}
			}
			if err == nil {
				data.EdgeNodes, err = server.edge.ListEdgeNodes(request.Context())
				for _, edgeNode := range data.EdgeNodes {
					if edgeNode.State == edgeapp.EdgeNodeActive {
						data.EdgeNodeCount++
					} else {
						data.EdgeNodePendingCount++
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
		if err == nil && (page == "status" || page == "sensors") {
			var devices []edgeapp.DeviceSummary
			devices, err = server.edge.ListDevices(
				request.Context(),
				edgeapp.PageRequest{Limit: 100},
			)
			if err == nil {
				attachConsoleSignalDevices(data.SignalRows, devices)
			}
		}
		if err == nil && page == "status" {
			for _, signal := range data.SignalRows {
				data.SemanticRuleCount += len(signal.NormalRules) + len(signal.AlarmRules)
			}
		}
		if err == nil && page == "sensors" {
			data.OutputRoutes, err = server.store.ListOutputRoutes(request.Context())
			data.OutputRoutes = profileOutputRoutes(data.OutputRoutes)
			for _, signal := range data.SignalRows {
				for _, rule := range append(
					append([]consoleSemanticRuleView(nil), signal.NormalRules...),
					signal.AlarmRules...,
				) {
					data.OutputRules = append(data.OutputRules, consoleRuleOption{
						ID:              rule.ID,
						Name:            signal.Name + " — " + rule.DisplayName,
						DisplayName:     rule.DisplayName,
						Kind:            displaySemanticKind(rule.Kind),
						ObservationKind: consoleObservationKind(rule.Kind),
						SignalRef:       signal.SignalRef,
						SensorName:      signal.Name,
					})
				}
			}
			if err == nil {
				data.OutputRouteRows = newConsoleOutputRouteViews(
					data.OutputRoutes,
					data.OutputRules,
					server.now(),
				)
				attachConsoleOutputSensorNames(
					data.OutputRouteRows,
					data.SignalRows,
				)
				attachConsoleOutputRoutes(data.SignalRows, data.OutputRouteRows)
				profiles, profileErr := server.store.ListExportProfiles(
					request.Context(),
				)
				if profileErr != nil {
					err = profileErr
				} else {
					attachConsoleOutputBindings(
						data.SignalRows,
						currentExportProfiles(profiles),
					)
				}
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
			if data.SelectedSignal.DeviceRef != nil {
				data.SensorSettingsPath = "/equipment/devices/" +
					*data.SelectedSignal.DeviceRef + "/sensors/" + signalRef
			}
			data.Title = data.SelectedSignal.Name
			data.Description = "現在値、受信状態、設定されている内容を確認します。"
			if deviceRef := request.PathValue("device_ref"); deviceRef != "" {
				if data.SelectedSignal.DeviceRef == nil ||
					*data.SelectedSignal.DeviceRef != deviceRef {
					http.NotFound(response, request)
					return
				}
				data.SensorView = "settings"
				data.SensorSettingsPath = "/equipment/devices/" + deviceRef +
					"/sensors/" + signalRef
				data.Title = data.SelectedSignal.Name + "の設定"
				data.Description = "表示、値の変換、異常検知を設定します。"
				var setupDevices []edgeapp.SetupDevice
				data.EdgeNodes, err = server.edge.ListEdgeNodes(request.Context())
				if err == nil {
					setupDevices, err = server.edge.ListSetupDevices(
						request.Context(),
						server.actor(auth),
						100,
					)
				}
				if err == nil {
					data.EquipmentRows = newConsoleEquipmentEdgeNodeViews(
						data.EdgeNodes,
						setupDevices,
						server.now(),
					)
					data.OrphanDevices = newConsoleOrphanDeviceViews(
						data.EdgeNodes,
						setupDevices,
						server.now(),
					)
					data.SelectedDevice, data.SelectedDeviceEdgeNode =
						findConsoleEquipmentDevice(
							data.EquipmentRows,
							data.OrphanDevices,
							deviceRef,
						)
					if data.SelectedDevice == nil {
						http.NotFound(response, request)
						return
					}
				}
			}
		}
		data.summarizeSignals()
	case "devices":
		data.Devices, err = server.edge.ListDevices(
			request.Context(), edgeapp.PageRequest{Limit: 100},
		)
		for _, device := range data.Devices {
			data.DeviceRows = append(data.DeviceRows, newConsoleDeviceView(device, server.now()))
		}
	case "setup":
		var setupDevices []edgeapp.SetupDevice
		setupDevices, err = server.edge.ListSetupDevices(
			request.Context(),
			server.actor(auth),
			100,
		)
		if err == nil {
			data.SetupRows = newConsoleSetupDeviceViews(setupDevices, server.now())
			for _, device := range setupDevices {
				if device.State == edgeapp.SetupWaitingForDevice {
					data.SetupPendingCount++
				}
			}
		}
	case "logs":
		data.Signals, err = server.edge.ListSignals(
			request.Context(), edgeapp.PageRequest{Limit: 100},
		)
		if err == nil {
			data.EdgeNodes, err = server.edge.ListEdgeNodes(request.Context())
		}
		if err == nil {
			now := server.now()
			data.SignalRows = newConsoleSignalViews(data.Signals, nil, now)
			data.EdgeNodeRows = newConsoleEdgeNodeViews(data.EdgeNodes, now)
			data.HistoryRange, data.HistoryRangeLabel, data.HistorySelectedEdgeNode,
				data.HistorySelectedSignal = selectConsoleHistoryScope(
				request, data.SignalRows,
			)
			from, until := consoleHistoryWindow(now, data.HistoryRange)
			var page store.HistoryPage
			page, err = server.store.QueryHistory(request.Context(), store.HistoryQuery{
				SignalRef:  data.HistorySelectedSignal,
				EdgeNodeID: data.HistorySelectedEdgeNode,
				From:       from, Until: until, Limit: 200,
			})
			if err == nil {
				data.LogRows = newConsoleHistoryLogViews(page.Records)
				data.HistoryHasMore = page.HasMore
				data.HistoryRawExportURL = consoleHistoryExportURL(
					from, until, data.HistorySelectedSignal, data.HistorySelectedEdgeNode,
				)
				data.HistoryExportURL = consoleSemanticHistoryExportURL(
					from, until, data.HistorySelectedSignal, data.HistorySelectedEdgeNode,
				)
			}
			if err == nil && data.HistorySelectedSignal != "" {
				bucketMilliseconds := (until - from + 179) / 180
				if bucketMilliseconds < 1_000 {
					bucketMilliseconds = 1_000
				}
				var series store.HistorySeries
				series, err = server.store.QueryHistorySeries(
					request.Context(), store.HistorySeriesQuery{
						SignalRef: data.HistorySelectedSignal,
						From:      from, Until: until,
						BucketMilliseconds: bucketMilliseconds,
					},
				)
				if err == nil {
					data.HistoryChart = newConsoleHistoryChart(
						series, selectedConsoleSignal(data.SignalRows, data.HistorySelectedSignal),
					)
				}
			}
		}
	case "output":
		data.OutputRoutes, err = server.store.ListOutputRoutes(request.Context())
		data.OutputRoutes = profileOutputRoutes(data.OutputRoutes)
		data.summarizeOutputRoutes(server.now())
		if err == nil {
			var profiles []edgeapp.ExportProfile
			profiles, err = server.store.ListExportProfiles(request.Context())
			if err == nil {
				currentProfiles := currentExportProfiles(profiles)
				previews := make(map[string]edgeapp.OutputPublicationPreview)
				for _, profile := range currentProfiles {
					for _, binding := range profile.Bindings {
						if binding.State != edgeapp.OutputBindingActive &&
							binding.State != edgeapp.OutputBindingPrepared &&
							binding.State != edgeapp.OutputBindingDraining {
							continue
						}
						preview, previewErr := server.store.GetOutputBindingPublication(
							request.Context(), binding.BindingID,
						)
						if previewErr == nil {
							previews[binding.BindingID] = preview
						}
					}
				}
				data.ExportProfiles = newConsoleExportProfileViews(
					currentProfiles,
					previews,
					newConsoleOutputRouteViews(
						data.OutputRoutes,
						nil,
						server.now(),
					),
				)
				data.summarizeExportProfiles()
				busyAdapters := make(map[string]bool)
				for _, profile := range profiles {
					if profile.State != edgeapp.ExportProfileStopped {
						busyAdapters[profile.AdapterID] = true
					}
				}
				if !busyAdapters["pinikiet.mqtt.v1"] {
					preview, previewErr := server.store.PreviewExportProfileActivation(
						request.Context(), "pinikiet.mqtt.v1",
					)
					if previewErr != nil {
						err = previewErr
						break
					}
					data.AvailableOutputs = append(
						data.AvailableOutputs,
						consoleAvailableOutput{
							AdapterID:               "pinikiet.mqtt.v1",
							DisplayName:             "Pinikietへ送る",
							Description:             "累積値・状態・アラームをPinikiet契約へ変換します。",
							AutomaticCount:          preview.AutomaticCount,
							NeedsConfigurationCount: preview.NeedsConfigurationCount,
							IneligibleCount:         preview.IneligibleCount,
							Rules:                   preview.Rules,
						},
					)
				}
				if !busyAdapters["iotkit.mqtt-json.v1"] {
					preview, previewErr := server.store.PreviewExportProfileActivation(
						request.Context(), "iotkit.mqtt-json.v1",
					)
					if previewErr != nil {
						err = previewErr
						break
					}
					data.AvailableOutputs = append(
						data.AvailableOutputs,
						consoleAvailableOutput{
							AdapterID:      "iotkit.mqtt-json.v1",
							DisplayName:    "汎用MQTT JSONで送る",
							Description:    "すべての意味づけ済みの値をIoTKit共通形式で送ります。",
							AutomaticCount: preview.AutomaticCount,
							Rules:          preview.Rules,
						},
					)
				}
			}
		}
	case "audit":
		data.Audit, err = server.edge.ListAuditEvents(request.Context(), 100)
		data.AuditRows = newConsoleAuditViews(data.Audit)
	case "accounts":
		if data.IsOwner {
			data.Accounts, err = server.accounts.ListAccounts(
				request.Context(), server.actor(auth),
			)
		}
	case "system":
		data.OutputRoutes, err = server.store.ListOutputRoutes(request.Context())
		data.OutputRoutes = profileOutputRoutes(data.OutputRoutes)
		if err == nil {
			var storageStatus store.StorageStatus
			storageStatus, err = server.store.GetStorageStatus(
				request.Context(), server.storageWarningPercent,
			)
			if err == nil {
				data.Storage = newConsoleStorageView(storageStatus)
			}
		}
		if err == nil {
			data.Diagnostics, err = server.store.GetDiagnostics(
				request.Context(), server.storageWarningPercent, server.now(),
			)
		}
	}
	if err == nil && page == "status" {
		var setupDevices []edgeapp.SetupDevice
		setupDevices, err = server.edge.ListSetupDevices(
			request.Context(),
			server.actor(auth),
			100,
		)
		if err == nil {
			for _, device := range setupDevices {
				data.SetupDeviceCount++
				if device.State == edgeapp.SetupWaitingForDevice {
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
	if err == nil && page == "status" && data.IsAdmin {
		data.Onboarding = newConsoleOnboardingView(consoleOnboardingFacts{
			ActiveEdgeNodes:     data.EdgeNodeCount,
			DeviceCount:         data.SetupDeviceCount,
			PendingDevices:      data.SetupPendingCount,
			SignalCount:         len(data.SignalRows),
			UnconfiguredSignals: data.UnconfiguredCount,
			SemanticRules:       data.SemanticRuleCount,
		})
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

func (data *consoleData) summarizeOutputRoutes(now time.Time) {
	data.OutputPendingCount = 0
	data.OutputActiveCount = 0
	data.OutputErrorCount = 0
	data.OutputStalledCount = 0
	for _, route := range data.OutputRoutes {
		data.OutputPendingCount += route.PendingCount
		if route.Active {
			data.OutputActiveCount++
		}
		if route.Active && route.LastTransformErrorCode != "" {
			data.OutputErrorCount++
		}
		if outputRouteDeliveryStalled(route, now) {
			data.OutputStalledCount++
		}
	}
	data.OutputHealthClass = "attention"
	data.OutputFlowClass = "pending"
	switch {
	case len(data.OutputRoutes) == 0:
		data.OutputStatusLabel = "外部出力は未設定です"
		data.OutputStatusDetail = "IoTKit Edgeで作った値を外部へ送る設定はまだありません。"
		data.OutputStatusSummary = "出力未設定"
	case data.OutputErrorCount > 0:
		data.OutputStatusLabel = "変換エラーがあります"
		data.OutputStatusDetail = strconv.Itoa(data.OutputErrorCount) +
			"件の外部向け変換を確認してください。"
		data.OutputStatusSummary = strconv.Itoa(data.OutputErrorCount) + "件の変換エラー"
	case data.OutputStalledCount > 0:
		data.OutputStatusLabel = "MQTT配送が停止している可能性があります"
		data.OutputStatusDetail = strconv.Itoa(data.OutputStalledCount) +
			"件の外部出力で、5分以上データを配送できていません。"
		data.OutputStatusSummary = strconv.Itoa(data.OutputStalledCount) +
			"件の配送を確認"
	case data.OutputPendingCount > 0:
		data.OutputHealthClass = "in-progress"
		data.OutputFlowClass = "complete"
		data.OutputStatusLabel = "配送中のデータがあります"
		data.OutputStatusDetail = strconv.FormatInt(data.OutputPendingCount, 10) +
			"件のデータをMQTTで配送しています。"
		data.OutputStatusSummary = strconv.FormatInt(data.OutputPendingCount, 10) +
			"件配送中"
	case data.OutputActiveCount == 0:
		data.OutputStatusLabel = "外部出力は停止中です"
		data.OutputStatusDetail = "登録済みの外部出力はすべて停止しています。"
		data.OutputStatusSummary = "すべて停止中"
	default:
		data.OutputHealthClass = "healthy"
		data.OutputFlowClass = "complete"
		data.OutputStatusLabel = "外部出力が設定されています"
		data.OutputStatusDetail = strconv.Itoa(data.OutputActiveCount) +
			"件の外部出力を使用中です。変換エラーと配送待ちはありません。"
		data.OutputStatusSummary = strconv.Itoa(data.OutputActiveCount) +
			"件の出力を使用中"
	}
}

func (data *consoleData) summarizeExportProfiles() {
	if len(data.ExportProfiles) == 0 ||
		data.OutputErrorCount > 0 ||
		data.OutputStalledCount > 0 {
		return
	}
	var activeDestinations, preparingDestinations, sendingValues int
	var needsConfiguration, preparedValues int
	for _, profile := range data.ExportProfiles {
		if profile.State == edgeapp.ExportProfileActive {
			activeDestinations++
		}
		if profile.State == edgeapp.ExportProfilePreparing {
			preparingDestinations++
		}
		sendingValues += profile.ActiveCount
		needsConfiguration += profile.NeedsConfigCount
		preparedValues += profile.PreparedCount
	}
	switch {
	case preparedValues > 0:
		data.OutputHealthClass = "in-progress"
		data.OutputStatusLabel = "外部アプリへの登録待ちがあります"
		data.OutputStatusDetail = strconv.Itoa(preparedValues) +
			"個の値はtopicを外部アプリへ登録してから送信を開始します。" +
			strconv.Itoa(needsConfiguration) + "個は用途の選択待ちです。"
		data.OutputStatusSummary = strconv.Itoa(preparedValues) + "件の外部登録待ち"
	case needsConfiguration > 0:
		data.OutputHealthClass = "in-progress"
		data.OutputStatusLabel = "用途の選択が必要な値があります"
		data.OutputStatusDetail = strconv.Itoa(activeDestinations) +
			"件の外部出力先で" + strconv.Itoa(sendingValues) +
			"個の値を送信中、" + strconv.Itoa(needsConfiguration) +
			"個は用途の選択待ちです。"
		data.OutputStatusSummary = strconv.Itoa(needsConfiguration) + "件の設定待ち"
	case data.OutputPendingCount > 0:
		data.OutputHealthClass = "in-progress"
		data.OutputFlowClass = "complete"
		data.OutputStatusLabel = "配送中のデータがあります"
		data.OutputStatusDetail = strconv.Itoa(activeDestinations) +
			"件の外部出力先で" + strconv.Itoa(sendingValues) +
			"個の値を送信中、" +
			strconv.FormatInt(data.OutputPendingCount, 10) +
			"件をMQTTで配送しています。"
		data.OutputStatusSummary = strconv.FormatInt(
			data.OutputPendingCount,
			10,
		) + "件配送中"
	case activeDestinations > 0:
		data.OutputHealthClass = "healthy"
		data.OutputFlowClass = "complete"
		data.OutputStatusLabel = "外部出力先が設定されています"
		data.OutputStatusDetail = strconv.Itoa(activeDestinations) +
			"件の外部出力先で" + strconv.Itoa(sendingValues) +
			"個の値を送信しています。"
		data.OutputStatusSummary = strconv.Itoa(activeDestinations) +
			"件の出力先を使用中"
	case preparingDestinations > 0:
		data.OutputHealthClass = "in-progress"
		data.OutputStatusLabel = "外部出力先を準備しています"
		data.OutputStatusDetail = "対応する値が追加されるとtopicを準備します。"
		data.OutputStatusSummary = "出力先を準備中"
	default:
		data.OutputStatusLabel = "外部出力先は停止中です"
		data.OutputStatusDetail = "登録済みの外部出力先はすべて停止しています。"
		data.OutputStatusSummary = "すべて停止中"
	}
}

func roleLabel(role edgeapp.AccountRole) string {
	switch role {
	case edgeapp.AccountRoleSystemAdmin:
		return "システム管理者"
	case edgeapp.AccountRoleAdmin:
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
