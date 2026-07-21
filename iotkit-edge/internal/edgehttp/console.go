package edgehttp

import (
	"crypto/x509"
	"encoding/pem"
	"errors"
	"net/http"
	"net/url"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
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
				if !busyAdapters["yokakit.mqtt.v1"] {
					preview, previewErr := server.store.PreviewExportProfileActivation(
						request.Context(), "yokakit.mqtt.v1",
					)
					if previewErr != nil {
						err = previewErr
						break
					}
					data.AvailableOutputs = append(
						data.AvailableOutputs,
						consoleAvailableOutput{
							AdapterID:               "yokakit.mqtt.v1",
							DisplayName:             "YokaKitへ送る",
							Description:             "累積値・状態・アラームをYokaKit契約へ変換します。",
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

func (server *Server) consoleDeviceProfile(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	expected := formRevision(request)
	_, err := server.edge.Dispatch(request.Context(), server.actor(auth), edgeapp.UpdateDeviceProfile{
		DeviceRef: request.PathValue("device_ref"),
		Input: edgeapp.DeviceProfileInput{
			DisplayName: request.FormValue("display_name"),
			Location:    request.FormValue("location"),
		},
		Precondition: edgeapp.RevisionPrecondition{Expected: expected},
	})
	server.consoleMutationResult(response, request, consoleReturnTarget(request, "/devices"), err)
}

func (server *Server) consoleEdgeNodeActivation(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	_, err := server.edge.Dispatch(
		request.Context(),
		server.actor(auth),
		edgeapp.ActivateEdgeNode{
			EdgeNodeRef: request.PathValue("edge_node_ref"),
			Precondition: edgeapp.RevisionPrecondition{
				Expected: formRevision(request),
			},
		},
	)
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(request, "/edge-nodes"),
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
	_, err := server.edge.Dispatch(request.Context(), server.actor(auth), edgeapp.UpdateSignalProfile{
		SignalRef: request.PathValue("signal_ref"),
		Input: edgeapp.SignalProfileInput{
			DisplayName:            request.FormValue("display_name"),
			DisplaySensorType:      request.FormValue("display_sensor_type"),
			DisplaySensorTypeLabel: request.FormValue("display_sensor_type_label"),
			DisplayValueKind:       request.FormValue("display_value_kind"),
			DisplayUnitMode:        displayUnitMode,
			DisplayUnit:            displayUnit,
			DecimalPlaces:          formInt(request, "decimal_places"),
		},
		Precondition: edgeapp.RevisionPrecondition{Expected: formRevision(request)},
	})
	server.consoleMutationResult(response, request, consoleReturnTarget(request, "/signals"), err)
}

func consoleReturnTarget(request *http.Request, fallback string) string {
	target := request.FormValue("return_to")
	if safeSensorSettingsReturnTarget(target) {
		return target
	}
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

func safeSensorSettingsReturnTarget(target string) bool {
	if !strings.HasPrefix(target, "/") ||
		strings.ContainsAny(target, "?#\\") ||
		strings.HasPrefix(target, "//") {
		return false
	}
	parts := strings.Split(strings.TrimPrefix(target, "/"), "/")
	return len(parts) == 5 &&
		parts[0] == "equipment" &&
		parts[1] == "devices" &&
		validConsoleResourceRef(parts[2], "dev_") &&
		parts[3] == "sensors" &&
		validConsoleResourceRef(parts[4], "sig_")
}

func safeSensorReturnTarget(target string) bool {
	if !strings.HasPrefix(target, "/") ||
		strings.ContainsAny(target, "?#\\") ||
		strings.HasPrefix(target, "//") {
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
	if !strings.HasPrefix(target, "/") ||
		strings.ContainsAny(target, "?#\\") ||
		strings.HasPrefix(target, "//") {
		return false
	}
	parts := strings.Split(strings.TrimPrefix(target, "/"), "/")
	if len(parts) != 3 || parts[0] != "equipment" || parts[2] == "" {
		return false
	}
	switch parts[1] {
	case "edge-nodes":
		return validConsoleResourceRef(parts[2], "edge_node_")
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
		spec, edgeapp.RevisionPrecondition{Expected: formRevision(request)},
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
		edgeapp.RevisionPrecondition{Expected: formRevision(request)},
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
			edgeapp.RevisionPrecondition{Expected: revision},
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
			edgeapp.RevisionPrecondition{Expected: revision},
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
			edgeapp.RevisionPrecondition{Expected: revision},
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
		edgeapp.RevisionPrecondition{Expected: revision},
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

func consoleObservationKind(kind semantics.Kind) outputadapter.ObservationKind {
	switch kind {
	case semantics.KindNumeric:
		return outputadapter.KindNumeric
	case semantics.KindBoolean:
		return outputadapter.KindBoolean
	case semantics.KindCumulativeCounter:
		return outputadapter.KindCumulativeValue
	case semantics.KindAlarm:
		return outputadapter.KindAlarm
	default:
		return ""
	}
}

func (server *Server) consoleActivateExportProfile(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	var err error
	if request.FormValue("auto_bind_future_rules") != "true" {
		err = errors.New("future rule authorization is required")
	} else {
		_, err = server.store.ActivateExportProfile(
			request.Context(),
			server.actor(auth),
			request.FormValue("display_name"),
			request.FormValue("adapter_id"),
		)
	}
	server.consoleMutationResult(response, request, "/output", err)
}

func (server *Server) consoleConfigureOutputBinding(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	revision, err := strconv.ParseInt(request.FormValue("revision"), 10, 64)
	if err == nil {
		_, err = server.store.ConfigureYokaKitBooleanBinding(
			request.Context(),
			server.actor(auth),
			request.PathValue("binding_id"),
			request.FormValue("mode"),
			revision,
		)
	}
	server.consoleMutationResult(response, request, "/output", err)
}

func (server *Server) consoleStopExportProfile(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	revision, err := strconv.ParseInt(request.FormValue("revision"), 10, 64)
	if err == nil {
		_, err = server.store.RequestExportProfileStop(
			request.Context(),
			server.actor(auth),
			request.PathValue("profile_id"),
			revision,
		)
	}
	server.consoleMutationResult(response, request, "/output", err)
}

func (server *Server) consoleStartOutputBinding(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	var err error
	if request.FormValue("external_registration_complete") != "true" {
		err = errors.New("external topic registration confirmation is required")
	} else {
		revision, parseErr := strconv.ParseInt(
			request.FormValue("revision"), 10, 64,
		)
		if parseErr != nil {
			err = parseErr
		} else {
			_, err = server.store.StartPreparedOutputBinding(
				request.Context(),
				server.actor(auth),
				request.PathValue("binding_id"),
				revision,
			)
		}
	}
	server.consoleMutationResult(response, request, "/output", err)
}

func (server *Server) consoleAccount(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserOwnerMutation(response, request)
	if !ok {
		return
	}
	_, err := server.accounts.DispatchAccount(
		request.Context(), server.actor(auth), edgeapp.CreateAccount{
			LoginID:           request.FormValue("login_id"),
			DisplayName:       request.FormValue("display_name"),
			Role:              edgeapp.AccountRole(request.FormValue("role")),
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
		request.Context(), server.actor(auth), edgeapp.UpdateAccount{
			AccountRef:       request.PathValue("account_ref"),
			DisplayName:      request.FormValue("display_name"),
			Role:             edgeapp.AccountRole(request.FormValue("role")),
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
		request.Context(), server.actor(auth), edgeapp.DisableAccount{
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
		request.Context(), server.actor(auth), edgeapp.ResetAccountPassword{
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
	if auth.account.Role != edgeapp.AccountRoleSystemAdmin {
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
	if adminOnly && auth.account.Role == edgeapp.AccountRoleViewer {
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
	parsed, parseErr := url.Parse(target)
	if parseErr != nil {
		parsed = &url.URL{Path: "/status"}
	}
	query := parsed.Query()
	anchor := request.FormValue("return_anchor")
	if safeConsoleAnchor(anchor) {
		query.Set("focus", anchor)
		parsed.Fragment = anchor
	}
	if err != nil {
		query.Set("error", consoleErrorCode(err))
		parsed.RawQuery = query.Encode()
		http.Redirect(response, request, parsed.String(), http.StatusSeeOther)
		return
	}
	query.Set("saved", "1")
	parsed.RawQuery = query.Encode()
	http.Redirect(response, request, parsed.String(), http.StatusSeeOther)
}

func safeConsoleAnchor(anchor string) bool {
	if anchor == "" || len(anchor) > 128 {
		return false
	}
	for _, character := range anchor {
		if (character < 'a' || character > 'z') &&
			(character < 'A' || character > 'Z') &&
			(character < '0' || character > '9') &&
			character != '-' && character != '_' {
			return false
		}
	}
	return true
}

func consoleErrorCode(err error) string {
	if errors.Is(err, edgeapp.ErrRevisionMismatch) {
		return "revision_mismatch"
	}
	switch err.Error() {
	case "semantic falling threshold cannot exceed rising threshold":
		return "threshold_order"
	case "invalid semantic rule number",
		"semantic detector thresholds must be finite":
		return "rule_number"
	case "semantic detector debounce must be between 0 and 300000 milliseconds":
		return "debounce_range"
	case "semantic calibration scale must be a finite non-zero number":
		return "calibration_scale"
	case "semantic calibration offset must be finite":
		return "calibration_offset"
	case "semantic rule display name must be 1 to 128 characters without surrounding whitespace":
		return "rule_name"
	default:
		return "save"
	}
}

func consoleErrorMessage(code string) string {
	switch code {
	case "revision_mismatch":
		return "別の担当者が先に設定を変更しました。最新の設定を確認して、もう一度変更してください。"
	case "threshold_order":
		return "保存できませんでした。立ち下がりしきい値は、立ち上がりしきい値以下にしてください。"
	case "rule_number":
		return "保存できませんでした。しきい値と確定待ち時間には数値を入力してください。"
	case "debounce_range":
		return "保存できませんでした。確定待ち時間は0秒から300秒の範囲で入力してください。"
	case "calibration_scale":
		return "保存できませんでした。補正倍率には0以外の数値を入力してください。"
	case "calibration_offset":
		return "保存できませんでした。補正加算には数値を入力してください。"
	case "rule_name":
		return "保存できませんでした。ルール名は前後の空白を除き、1文字から128文字で入力してください。"
	default:
		return "保存できませんでした。入力内容を確認し、もう一度お試しください。"
	}
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
		request.Context(), server.actor(auth), edgeapp.ChangeOwnPassword{
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
