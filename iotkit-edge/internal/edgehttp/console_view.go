package edgehttp

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
)

type consoleSignalView struct {
	edgeapp.SignalSummary
	Name                 string
	Value                string
	Unit                 string
	SensorType           string
	LastReceived         string
	LastReceivedTitle    string
	StatusLabel          string
	StatusClass          string
	SettingLabel         string
	SettingClass         string
	MeaningLabel         string
	MeaningClass         string
	FormProfile          edgeapp.SignalProfileInput
	Definition           *semantics.Definition
	SourceSensorType     string
	SourceValue          string
	SourceValueType      string
	SourceUnit           string
	ChannelLabel         string
	DeviceName           string
	DeviceLocation       string
	DeviceModelID        string
	InputIsBoolean       bool
	RiseDebounceSeconds  string
	FallDebounceSeconds  string
	Configuration        *semantics.Configuration
	NormalRules          []consoleSemanticRuleView
	AlarmRules           []consoleSemanticRuleView
	FlowRules            []consoleSemanticRuleView
	FlowRuleRemaining    int
	FlowRoutes           []consoleOutputRouteView
	FlowRouteRemaining   int
	FlowRouteCount       int
	FlowActiveCount      int
	FlowStoppedCount     int
	FlowTransformErrors  int
	FlowDeliveryErrors   int
	FlowPendingCount     int64
	FlowPreparedCount    int
	FlowNeedsConfigCount int
	FlowIneligibleCount  int
}

type consoleSemanticRuleView struct {
	semantics.Rule
	KindLabel           string
	RiseDebounceSeconds string
	FallDebounceSeconds string
	OutputRoutes        []consoleOutputRouteView
	OutputBindings      []consoleOutputBindingView
}

type consoleEdgeNodeView struct {
	edgeapp.EdgeNode
	Name                string
	LocationLabel       string
	StateLabel          string
	StateClass          string
	LastCommunication   string
	LastCommunicationAt string
	LastResult          string
	CanActivate         bool
}

func newConsoleEdgeNodeViews(edgeNodes []edgeapp.EdgeNode, now time.Time) []consoleEdgeNodeView {
	views := make([]consoleEdgeNodeView, 0, len(edgeNodes))
	for _, edgeNode := range edgeNodes {
		view := consoleEdgeNodeView{
			EdgeNode:      edgeNode,
			Name:          edgeNode.DisplayName,
			LocationLabel: edgeNode.Location,
			CanActivate:   edgeNode.State == edgeapp.EdgeNodeDiscovered,
		}
		if view.Name == "" {
			view.Name = edgeNode.EdgeNodeID
		}
		if view.LocationLabel == "" {
			view.LocationLabel = "設置場所 未設定"
		}
		view.LastCommunication, view.LastCommunicationAt = displayAge(
			edgeNode.LastDescriptorAt, now,
		)
		view.LastResult, _ = displayAge(edgeNode.LastResultAt, now)
		switch edgeNode.State {
		case edgeapp.EdgeNodeDiscovered:
			view.StateLabel, view.StateClass = "未登録", "needs-setup"
		case edgeapp.EdgeNodeActivating:
			view.StateLabel, view.StateClass = "登録処理中", "stale"
		case edgeapp.EdgeNodeActive:
			view.StateLabel, view.StateClass = "登録済み", "configured"
		case edgeapp.EdgeNodeRecoveryHold:
			view.StateLabel, view.StateClass = "復旧確認待ち", "stale"
		default:
			view.StateLabel, view.StateClass = "状態不明", "stale"
		}
		views = append(views, view)
	}
	return views
}

type consoleEquipmentEdgeNodeView struct {
	consoleEdgeNodeView
	Devices            []consoleSetupDeviceView
	DevicePendingCount int
	SensorPendingCount int
}

func newConsoleEquipmentEdgeNodeViews(
	edgeNodes []edgeapp.EdgeNode,
	devices []edgeapp.SetupDevice,
	now time.Time,
) []consoleEquipmentEdgeNodeView {
	devicesByEdgeNode := make(map[string][]edgeapp.SetupDevice)
	for _, device := range devices {
		devicesByEdgeNode[device.Device.EdgeNodeID] = append(
			devicesByEdgeNode[device.Device.EdgeNodeID],
			device,
		)
	}

	edgeNodeViews := newConsoleEdgeNodeViews(edgeNodes, now)
	views := make([]consoleEquipmentEdgeNodeView, 0, len(edgeNodeViews))
	for _, edgeNode := range edgeNodeViews {
		edgeDevices := devicesByEdgeNode[edgeNode.EdgeNodeID]
		view := consoleEquipmentEdgeNodeView{
			consoleEdgeNodeView: edgeNode,
			Devices:             newConsoleSetupDeviceViews(edgeDevices, now),
		}
		for _, device := range edgeDevices {
			if device.State == edgeapp.SetupWaitingForDevice {
				view.DevicePendingCount++
			}
			for _, signal := range device.Signals {
				if !signal.ProfileComplete {
					view.SensorPendingCount++
				}
			}
		}
		views = append(views, view)
	}
	return views
}

func newConsoleOrphanDeviceViews(
	edgeNodes []edgeapp.EdgeNode,
	devices []edgeapp.SetupDevice,
	now time.Time,
) []consoleSetupDeviceView {
	knownEdgeNodes := make(map[string]struct{}, len(edgeNodes))
	for _, edgeNode := range edgeNodes {
		knownEdgeNodes[edgeNode.EdgeNodeID] = struct{}{}
	}
	orphans := make([]edgeapp.SetupDevice, 0)
	for _, device := range devices {
		if _, exists := knownEdgeNodes[device.Device.EdgeNodeID]; !exists {
			orphans = append(orphans, device)
		}
	}
	return newConsoleSetupDeviceViews(orphans, now)
}

type consoleSetupDeviceView struct {
	edgeapp.SetupDevice
	Name              string
	LocationLabel     string
	StateLabel        string
	StateClass        string
	LastReceived      string
	LastReceivedTitle string
	Signals           []consoleSetupSignalView
}

type consoleSetupSignalView struct {
	edgeapp.SetupSignal
	RawValue          string
	RawUnit           string
	MeasurementKey    string
	ValueTypeLabel    string
	ChannelLabel      string
	LastReceived      string
	LastReceivedTitle string
	FormProfile       edgeapp.SignalProfileInput
	ProfileRevision   *int64
	MissingMessage    string
}

type consoleDeviceView struct {
	edgeapp.DeviceSummary
	Name              string
	LocationLabel     string
	LastReceived      string
	LastReceivedTitle string
	StatusLabel       string
	StatusClass       string
}

type consoleLogView struct {
	ReceivedAt string
	ObservedAt string
	EdgeNode   string
	Sensor     string
	SignalRef  string
	Value      string
	Unit       string
}

type consoleHistoryChart struct {
	Path            string
	DisplayName     string
	Unit            string
	Minimum         string
	Maximum         string
	Latest          string
	SampleCount     int64
	StartLabel      string
	EndLabel        string
	AccessibleLabel string
}

type consoleStorageView struct {
	Available           bool
	StateClass          string
	StateLabel          string
	DatabaseSize        string
	ReclaimableSize     string
	DiskAvailable       string
	DiskUsedPercent     int
	WarningPercent      int
	RawRecordCount      int64
	ObservationCount    int64
	PendingOutputCount  int64
	ProjectionFailures  int64
	LastBackupAvailable bool
	LastBackupAt        string
	LastBackupRecords   int64
	BackupProtectedRaw  int64
	UnprotectedRaw      int64
}

func newConsoleStorageView(status store.StorageStatus) consoleStorageView {
	view := consoleStorageView{
		Available:  status.FilesystemAvailable,
		StateClass: "stale", StateLabel: "容量を確認できません",
		DatabaseSize:       formatByteCount(uint64(max(status.DatabaseBytes, 0))),
		ReclaimableSize:    formatByteCount(uint64(max(status.ReclaimableBytes, 0))),
		DiskUsedPercent:    status.DiskUsedPercent,
		WarningPercent:     status.WarningPercent,
		RawRecordCount:     status.RawRecordCount,
		ObservationCount:   status.SemanticObservationCount,
		PendingOutputCount: status.PendingOutputCount,
		ProjectionFailures: status.ProjectionFailureCount,
		BackupProtectedRaw: status.BackupProtectedRawCount,
		UnprotectedRaw:     status.UnprotectedRawCount,
	}
	if status.FilesystemAvailable {
		view.DiskAvailable = formatByteCount(status.DiskAvailableBytes)
	}
	if status.LastBackupAt != nil {
		view.LastBackupAvailable = true
		view.LastBackupAt = time.UnixMilli(*status.LastBackupAt).In(time.Local).Format("2006年1月2日 15:04")
		view.LastBackupRecords = status.LastBackupRawRecordCount
	}
	switch status.State {
	case store.StorageHealthy:
		view.StateClass, view.StateLabel = "healthy", "保存容量は正常です"
	case store.StorageWarning:
		view.StateClass, view.StateLabel = "in-progress", "保存容量が少なくなっています"
	case store.StorageCritical:
		view.StateClass, view.StateLabel = "stale", "保存容量が残りわずかです"
	}
	return view
}

func formatByteCount(bytes uint64) string {
	const unit = uint64(1024)
	if bytes < unit {
		return strconv.FormatUint(bytes, 10) + " B"
	}
	value, suffix := float64(bytes), "KiB"
	for _, candidate := range []string{"KiB", "MiB", "GiB", "TiB"} {
		suffix = candidate
		value = float64(bytes) / float64(unit)
		if value < 1024 || candidate == "TiB" {
			break
		}
		bytes /= unit
	}
	return strconv.FormatFloat(value, 'f', 1, 64) + " " + suffix
}

type consoleAuditView struct {
	OccurredAt   string
	Actor        string
	Operation    string
	Resource     string
	Outcome      string
	OutcomeClass string
}

type consoleRuleOption struct {
	ID              string
	Name            string
	DisplayName     string
	Kind            string
	ObservationKind outputadapter.ObservationKind
	SignalRef       string
	SensorName      string
}

type consoleExportProfileView struct {
	edgeapp.ExportProfile
	AdapterLabel           string
	StateLabel             string
	StateClass             string
	ActiveCount            int
	NeedsConfigCount       int
	PreparedCount          int
	IneligibleCount        int
	DrainingCount          int
	StoppedCount           int
	AttentionCount         int
	TransformErrorCount    int
	DeliveryAttentionCount int
	Bindings               []consoleOutputBindingView
}

type consoleOutputBindingView struct {
	edgeapp.OutputProfileRuleBinding
	AdapterLabel      string
	StateLabel        string
	StateClass        string
	KindLabel         string
	ModeLabel         string
	Preview           *edgeapp.OutputPublicationPreview
	PreviewLabel      string
	PrettyPayload     string
	HasDiagnostics    bool
	TransformLabel    string
	TransformClass    string
	DeliveryLabel     string
	DeliveryClass     string
	PendingCount      int64
	NeedsAttention    bool
	TransformError    bool
	DeliveryAttention bool
}

func newConsoleExportProfileViews(
	profiles []edgeapp.ExportProfile,
	previews map[string]edgeapp.OutputPublicationPreview,
	routes []consoleOutputRouteView,
) []consoleExportProfileView {
	routesByBinding := make(map[string]consoleOutputRouteView, len(routes))
	for _, route := range routes {
		if route.BindingID != "" {
			routesByBinding[route.BindingID] = route
		}
	}
	views := make([]consoleExportProfileView, 0, len(profiles))
	for _, profile := range profiles {
		view := consoleExportProfileView{
			ExportProfile: profile,
			AdapterLabel:  profile.AdapterID,
			StateLabel:    "停止中",
			StateClass:    "never",
		}
		switch profile.AdapterID {
		case "iotkit.mqtt-json.v1":
			view.AdapterLabel = "汎用MQTT JSON"
		case "yokakit.mqtt.v1":
			view.AdapterLabel = "YokaKit"
		}
		switch profile.State {
		case edgeapp.ExportProfilePreparing:
			view.StateLabel, view.StateClass = "送信前の準備中", "needs-setup"
		case edgeapp.ExportProfileActive:
			view.StateLabel, view.StateClass = "使用中", "configured"
		case edgeapp.ExportProfileDraining:
			view.StateLabel, view.StateClass = "配送終了処理中", "in-progress"
		}
		for _, binding := range profile.Bindings {
			bindingView := consoleOutputBindingView{
				OutputProfileRuleBinding: binding,
				AdapterLabel:             view.AdapterLabel,
				KindLabel:                displaySemanticKind(semantics.Kind(binding.RuleKind)),
				ModeLabel:                displayOutputKind(binding.Mode),
			}
			switch binding.State {
			case edgeapp.OutputBindingPrepared:
				bindingView.StateLabel, bindingView.StateClass = "外部登録待ち", "needs-setup"
				view.PreparedCount++
			case edgeapp.OutputBindingActive:
				bindingView.StateLabel, bindingView.StateClass = "送信対象", "configured"
				view.ActiveCount++
			case edgeapp.OutputBindingNeedsConfiguration:
				bindingView.StateLabel, bindingView.StateClass = "用途を選んでください", "needs-setup"
				view.NeedsConfigCount++
			case edgeapp.OutputBindingIneligible:
				bindingView.StateLabel, bindingView.StateClass = "対象外", "never"
				view.IneligibleCount++
			case edgeapp.OutputBindingDraining:
				bindingView.StateLabel, bindingView.StateClass = "配送終了処理中", "in-progress"
				view.DrainingCount++
			default:
				bindingView.StateLabel, bindingView.StateClass = "停止", "never"
				view.StoppedCount++
			}
			if preview, exists := previews[binding.BindingID]; exists {
				copied := preview
				bindingView.Preview = &copied
				switch preview.Provenance {
				case "actual":
					bindingView.PreviewLabel = "実際の送信内容"
				case "latest_observation":
					bindingView.PreviewLabel = "最新値を使った変換結果"
				default:
					bindingView.PreviewLabel = "送信内容のサンプル"
				}
				var pretty bytes.Buffer
				if json.Indent(&pretty, preview.Payload, "", "  ") == nil {
					bindingView.PrettyPayload = pretty.String()
				} else {
					bindingView.PrettyPayload = string(preview.Payload)
				}
			}
			if route, exists := routesByBinding[binding.BindingID]; exists &&
				(binding.State == edgeapp.OutputBindingActive ||
					binding.State == edgeapp.OutputBindingDraining) {
				bindingView.HasDiagnostics = true
				bindingView.TransformLabel = route.TransformLabel
				bindingView.TransformClass = route.TransformClass
				bindingView.DeliveryLabel = route.DeliveryLabel
				bindingView.DeliveryClass = route.DeliveryClass
				bindingView.PendingCount = route.PendingCount
				bindingView.TransformError =
					route.LastTransformErrorCode != ""
				bindingView.DeliveryAttention =
					route.DeliveryClass == "stale"
				bindingView.NeedsAttention =
					bindingView.TransformError ||
						bindingView.DeliveryAttention
				if bindingView.TransformError {
					view.TransformErrorCount++
				}
				if bindingView.DeliveryAttention {
					view.DeliveryAttentionCount++
				}
				if bindingView.NeedsAttention {
					view.AttentionCount++
				}
			}
			view.Bindings = append(view.Bindings, bindingView)
		}
		sort.SliceStable(view.Bindings, func(left, right int) bool {
			if view.Bindings[left].NeedsAttention !=
				view.Bindings[right].NeedsAttention {
				return view.Bindings[left].NeedsAttention
			}
			priority := func(state edgeapp.OutputBindingState) int {
				switch state {
				case edgeapp.OutputBindingNeedsConfiguration:
					return 0
				case edgeapp.OutputBindingPrepared:
					return 1
				case edgeapp.OutputBindingDraining:
					return 2
				case edgeapp.OutputBindingActive:
					return 3
				case edgeapp.OutputBindingIneligible:
					return 4
				default:
					return 5
				}
			}
			return priority(view.Bindings[left].State) <
				priority(view.Bindings[right].State)
		})
		views = append(views, view)
	}
	return views
}

func currentExportProfiles(
	profiles []edgeapp.ExportProfile,
) []edgeapp.ExportProfile {
	current := make([]edgeapp.ExportProfile, 0, len(profiles))
	for _, profile := range profiles {
		if profile.State != edgeapp.ExportProfileStopped {
			current = append(current, profile)
		}
	}
	return current
}

func attachConsoleOutputBindings(
	signals []consoleSignalView,
	profiles []edgeapp.ExportProfile,
) {
	bindingsByRule := make(map[string][]consoleOutputBindingView)
	for _, profileView := range newConsoleExportProfileViews(profiles, nil, nil) {
		for _, binding := range profileView.Bindings {
			switch binding.State {
			case edgeapp.OutputBindingNeedsConfiguration,
				edgeapp.OutputBindingPrepared,
				edgeapp.OutputBindingIneligible:
				bindingsByRule[binding.RuleID] = append(
					bindingsByRule[binding.RuleID],
					binding,
				)
			}
		}
	}
	for signalIndex := range signals {
		signals[signalIndex].FlowPreparedCount = 0
		signals[signalIndex].FlowNeedsConfigCount = 0
		signals[signalIndex].FlowIneligibleCount = 0
		for ruleIndex := range signals[signalIndex].NormalRules {
			rule := &signals[signalIndex].NormalRules[ruleIndex]
			rule.OutputBindings = bindingsByRule[rule.ID]
			addConsoleOutputBindingFlowCounts(
				&signals[signalIndex],
				rule.OutputBindings,
			)
		}
		for ruleIndex := range signals[signalIndex].AlarmRules {
			rule := &signals[signalIndex].AlarmRules[ruleIndex]
			rule.OutputBindings = bindingsByRule[rule.ID]
			addConsoleOutputBindingFlowCounts(
				&signals[signalIndex],
				rule.OutputBindings,
			)
		}
	}
}

func addConsoleOutputBindingFlowCounts(
	signal *consoleSignalView,
	bindings []consoleOutputBindingView,
) {
	for _, binding := range bindings {
		switch binding.State {
		case edgeapp.OutputBindingPrepared:
			signal.FlowPreparedCount++
		case edgeapp.OutputBindingNeedsConfiguration:
			signal.FlowNeedsConfigCount++
		case edgeapp.OutputBindingIneligible:
			signal.FlowIneligibleCount++
		}
	}
}

type consoleOutputRouteView struct {
	edgeapp.OutputRoute
	RuleName       string
	KindLabel      string
	AdapterLabel   string
	Destination    string
	StateLabel     string
	StateClass     string
	TransformLabel string
	TransformClass string
	DeliveryLabel  string
	DeliveryClass  string
	DeliveryDetail string
	SignalRef      string
	SensorName     string
}

func profileOutputRoutes(
	routes []edgeapp.OutputRoute,
) []edgeapp.OutputRoute {
	profileRoutes := make([]edgeapp.OutputRoute, 0, len(routes))
	for _, route := range routes {
		if route.BindingID != "" {
			profileRoutes = append(profileRoutes, route)
		}
	}
	return profileRoutes
}

func newConsoleSignalView(
	summary edgeapp.SignalSummary,
	now time.Time,
) consoleSignalView {
	view := consoleSignalView{
		SignalSummary: summary,
		Name:          summary.DisplayName,
		Value:         "—",
		Unit:          displayUnit(summary.Unit),
		SensorType:    displaySensorType(summary.SensorType),
		SettingLabel:  "未設定",
		SettingClass:  "needs-setup",
		MeaningLabel:  "未設定",
		MeaningClass:  "needs-setup",
		SourceSensorType: pointerText(
			summary.SensorType,
			"未通知",
		),
		SourceValue:     "—",
		SourceValueType: displayValueType(summary.ValueType),
		SourceUnit:      displayUnit(summary.Unit),
		ChannelLabel:    "なし",
	}
	if view.SourceUnit == "" {
		view.SourceUnit = "通知なし"
	}
	if summary.ChannelIndex != nil {
		view.ChannelLabel = strconv.FormatInt(int64(*summary.ChannelIndex), 10)
	}
	if summary.Latest != nil {
		view.SourceValue = displayValues(summary.Latest.Values, summary.ValueType)
	}
	view.FormProfile, _ = edgeapp.SignalProfileCandidate(summary)
	if summary.Profile != nil {
		if summary.Profile.DisplayName != "" {
			view.FormProfile.DisplayName = summary.Profile.DisplayName
		}
		if summary.Profile.DisplaySensorType != "" {
			view.FormProfile.DisplaySensorType = normalizeDisplaySensorType(
				summary.Profile.DisplaySensorType,
			)
			view.FormProfile.DisplaySensorTypeLabel = summary.Profile.DisplaySensorTypeLabel
			view.FormProfile.DisplayValueKind = summary.Profile.DisplayValueKind
			view.FormProfile.DisplayUnitMode = summary.Profile.DisplayUnitMode
			view.FormProfile.DisplayUnit = summary.Profile.DisplayUnit
			view.FormProfile.DecimalPlaces = summary.Profile.DecimalPlaces
		}
	}
	profileComplete := summary.Profile != nil && summary.Profile.Complete()
	if profileComplete {
		profile := summary.Profile
		view.Name = profile.DisplayName
		view.Unit = ""
		if profile.DisplayUnitMode == "unit" {
			view.Unit = displayUnit(&profile.DisplayUnit)
		}
		view.SensorType = displayProfileSensorType(profile)
		valueType := "float"
		if profile.DisplayValueKind == "boolean" {
			valueType = "bool"
		}
		if summary.Latest != nil {
			view.Value = displayValuesWithPrecision(
				summary.Latest.Values,
				&valueType,
				profile.DecimalPlaces,
			)
		}
		view.SettingLabel = "設定済み"
		view.SettingClass = "configured"
	}
	if view.Name == "" {
		view.Name = "名前未設定のセンサー"
	}
	if summary.Latest != nil && !profileComplete {
		view.Value = displayValues(summary.Latest.Values, summary.ValueType)
	}
	if summary.Latest == nil {
		view.Unit = ""
	}
	view.LastReceived, view.LastReceivedTitle = displayAge(summary.LastReceivedAt, now)
	switch summary.ReceiptStatus {
	case "receiving":
		view.StatusLabel = "受信中"
		view.StatusClass = "receiving"
	case "stale":
		view.StatusLabel = "停止・古い"
		view.StatusClass = "stale"
	default:
		view.StatusLabel = "未受信"
		view.StatusClass = "never"
	}
	return view
}

func attachConsoleSignalDevices(
	signals []consoleSignalView,
	devices []edgeapp.DeviceSummary,
) {
	deviceByRef := make(map[string]edgeapp.DeviceSummary, len(devices))
	for _, device := range devices {
		deviceByRef[device.DeviceRef] = device
	}
	for index := range signals {
		if signals[index].DeviceRef == nil {
			continue
		}
		device, exists := deviceByRef[*signals[index].DeviceRef]
		if !exists {
			continue
		}
		signals[index].DeviceName = device.DisplayName
		if signals[index].DeviceName == "" {
			signals[index].DeviceName = "名前未設定のデバイス"
		}
		signals[index].DeviceLocation = device.Location
		if signals[index].DeviceLocation == "" {
			signals[index].DeviceLocation = "設置場所 未設定"
		}
		if device.ModelID != nil {
			signals[index].DeviceModelID = *device.ModelID
		}
	}
}

func newConsoleSetupDeviceViews(
	devices []edgeapp.SetupDevice,
	now time.Time,
) []consoleSetupDeviceView {
	views := make([]consoleSetupDeviceView, 0, len(devices))
	for _, device := range devices {
		view := consoleSetupDeviceView{
			SetupDevice:   device,
			Name:          device.Device.DisplayName,
			LocationLabel: device.Device.Location,
		}
		if view.Name == "" {
			view.Name = "名前未設定のデバイス"
		}
		if view.LocationLabel == "" {
			view.LocationLabel = "設置場所 未設定"
		}
		view.LastReceived, view.LastReceivedTitle = displayAge(
			device.Device.LastReceivedAt,
			now,
		)
		switch device.State {
		case edgeapp.SetupReady:
			view.StateLabel, view.StateClass = "登録済み", "configured"
		case edgeapp.SetupWaitingForDevice:
			view.StateLabel, view.StateClass = "デバイス情報を入力", "needs-setup"
		case edgeapp.SetupMetadataMissing:
			view.StateLabel, view.StateClass = "種類・単位を確認", "needs-setup"
		default:
			view.StateLabel, view.StateClass = "センサーを設定", "needs-setup"
		}
		for _, signal := range device.Signals {
			view.Signals = append(
				view.Signals,
				newConsoleSetupSignalView(signal, now),
			)
		}
		views = append(views, view)
	}
	return views
}

func newConsoleSetupSignalView(
	signal edgeapp.SetupSignal,
	now time.Time,
) consoleSetupSignalView {
	view := consoleSetupSignalView{
		SetupSignal:    signal,
		RawValue:       "—",
		RawUnit:        displayUnit(signal.Signal.Unit),
		MeasurementKey: pointerText(signal.Signal.SensorType, "未通知"),
		ValueTypeLabel: displayValueType(signal.Signal.ValueType),
		ChannelLabel:   "なし",
		FormProfile:    signal.Candidate,
	}
	if signal.Signal.Latest != nil {
		view.RawValue = displayValues(
			signal.Signal.Latest.Values,
			signal.Signal.ValueType,
		)
	}
	if signal.ChannelIndex != nil {
		view.ChannelLabel = strconv.FormatInt(int64(*signal.ChannelIndex), 10)
	}
	view.LastReceived, view.LastReceivedTitle = displayAge(
		signal.Signal.LastReceivedAt,
		now,
	)
	if signal.Profile != nil {
		view.ProfileRevision = &signal.Profile.Revision
		if signal.Profile.DisplayName != "" {
			view.FormProfile.DisplayName = signal.Profile.DisplayName
		}
		if signal.Profile.DisplaySensorType != "" {
			view.FormProfile.DisplaySensorType = normalizeDisplaySensorType(
				signal.Profile.DisplaySensorType,
			)
			view.FormProfile.DisplaySensorTypeLabel = signal.Profile.DisplaySensorTypeLabel
			view.FormProfile.DisplayValueKind = signal.Profile.DisplayValueKind
			view.FormProfile.DisplayUnitMode = signal.Profile.DisplayUnitMode
			view.FormProfile.DisplayUnit = signal.Profile.DisplayUnit
			view.FormProfile.DecimalPlaces = signal.Profile.DecimalPlaces
		}
	}
	if len(signal.CandidateMissing) > 0 {
		view.MissingMessage = "収集ノードから種類・単位を確認できません。接続したセンサーを確認して入力してください。"
	}
	return view
}

func newConsoleSignalViews(
	summaries []edgeapp.SignalSummary,
	definitions []semantics.Definition,
	now time.Time,
) []consoleSignalView {
	definitionBySignal := make(map[string]*semantics.Definition, len(definitions))
	for index := range definitions {
		if definitions[index].Active {
			definitionBySignal[definitions[index].SignalRef] = &definitions[index]
		}
	}
	views := make([]consoleSignalView, 0, len(summaries))
	for _, summary := range summaries {
		view := newConsoleSignalView(summary, now)
		view.Definition = definitionBySignal[summary.SignalRef]
		view.InputIsBoolean = view.FormProfile.DisplayValueKind == "boolean"
		view.RiseDebounceSeconds = "0"
		view.FallDebounceSeconds = "0"
		if view.Definition != nil {
			view.MeaningLabel = "設定済み"
			view.MeaningClass = "configured"
			view.RiseDebounceSeconds = formatMillisecondsAsSeconds(
				view.Definition.Detector.RiseDebounceMS,
			)
			view.FallDebounceSeconds = formatMillisecondsAsSeconds(
				view.Definition.Detector.FallDebounceMS,
			)
		}
		views = append(views, view)
	}
	return views
}

func attachConsoleSemanticConfigurations(
	signals []consoleSignalView,
	configurations map[string]semantics.Configuration,
) {
	for index := range signals {
		configuration, exists := configurations[signals[index].SignalRef]
		if !exists {
			continue
		}
		signals[index].Configuration = &configuration
		signals[index].MeaningLabel = "未設定"
		signals[index].MeaningClass = "needs-setup"
		for _, rule := range configuration.Rules {
			view := consoleSemanticRuleView{
				Rule:                rule,
				KindLabel:           displaySemanticKind(rule.Kind),
				RiseDebounceSeconds: formatMillisecondsAsSeconds(rule.Detector.RiseDebounceMS),
				FallDebounceSeconds: formatMillisecondsAsSeconds(rule.Detector.FallDebounceMS),
			}
			if rule.Kind == semantics.KindAlarm {
				signals[index].AlarmRules = append(signals[index].AlarmRules, view)
			} else {
				signals[index].NormalRules = append(signals[index].NormalRules, view)
			}
		}
		if len(configuration.Rules) > 0 {
			signals[index].MeaningLabel = strconv.Itoa(len(configuration.Rules)) + "件のルール"
			signals[index].MeaningClass = "configured"
		}
	}
}

func attachConsoleOutputRoutes(
	signals []consoleSignalView,
	routes []consoleOutputRouteView,
) {
	routesByRule := make(map[string][]consoleOutputRouteView)
	for _, route := range routes {
		if route.BindingID != "" && !route.Active {
			continue
		}
		routesByRule[route.RuleID] = append(routesByRule[route.RuleID], route)
	}
	for signalIndex := range signals {
		signal := &signals[signalIndex]
		for ruleIndex := range signal.NormalRules {
			rule := &signal.NormalRules[ruleIndex]
			rule.OutputRoutes = routesByRule[rule.ID]
		}
		for ruleIndex := range signal.AlarmRules {
			rule := &signal.AlarmRules[ruleIndex]
			rule.OutputRoutes = routesByRule[rule.ID]
		}
		allRules := append(
			append([]consoleSemanticRuleView(nil), signal.NormalRules...),
			signal.AlarmRules...,
		)
		signal.FlowRules = allRules
		if len(signal.FlowRules) > 2 {
			signal.FlowRuleRemaining = len(signal.FlowRules) - 2
			signal.FlowRules = signal.FlowRules[:2]
		}
		for _, rule := range allRules {
			signal.FlowRoutes = append(signal.FlowRoutes, rule.OutputRoutes...)
		}
		signal.FlowRouteCount = len(signal.FlowRoutes)
		for _, route := range signal.FlowRoutes {
			if route.Active {
				signal.FlowActiveCount++
			} else {
				signal.FlowStoppedCount++
			}
			if route.Active && route.LastTransformErrorCode != "" {
				signal.FlowTransformErrors++
			}
			if route.DeliveryClass == "stale" {
				signal.FlowDeliveryErrors++
			}
			signal.FlowPendingCount += route.PendingCount
		}
		if len(signal.FlowRoutes) > 2 {
			signal.FlowRouteRemaining = len(signal.FlowRoutes) - 2
			signal.FlowRoutes = signal.FlowRoutes[:2]
		}
	}
}

func attachConsoleOutputSensorNames(
	routes []consoleOutputRouteView,
	signals []consoleSignalView,
) {
	names := make(map[string]string, len(signals))
	for _, signal := range signals {
		names[signal.SignalRef] = signal.Name
	}
	for index := range routes {
		if routes[index].SensorName == "" {
			routes[index].SensorName = names[routes[index].SignalRef]
		}
		if routes[index].SensorName == "" {
			routes[index].SensorName = "名前未設定のセンサー"
		}
	}
}

func formatMillisecondsAsSeconds(milliseconds int64) string {
	return strconv.FormatFloat(float64(milliseconds)/1000, 'f', -1, 64)
}

func newConsoleDeviceView(
	summary edgeapp.DeviceSummary,
	now time.Time,
) consoleDeviceView {
	view := consoleDeviceView{
		DeviceSummary: summary,
		Name:          summary.DisplayName,
		LocationLabel: summary.Location,
	}
	if view.Name == "" {
		view.Name = "名前未設定のデバイス"
	}
	if view.LocationLabel == "" {
		view.LocationLabel = "設置場所 未設定"
	}
	view.LastReceived, view.LastReceivedTitle = displayAge(summary.LastReceivedAt, now)
	switch {
	case summary.DescriptorPresence == "stale":
		view.StatusLabel = "接続情報が古い"
		view.StatusClass = "stale"
	case summary.DeviceState == "active":
		view.StatusLabel = "使用中"
		view.StatusClass = "receiving"
	case summary.DeviceState == "retired":
		view.StatusLabel = "使用終了"
		view.StatusClass = "never"
	default:
		view.StatusLabel = "確認が必要"
		view.StatusClass = "stale"
	}
	return view
}

func newConsoleLogViews(
	records []store.RawRecord,
	signals []consoleSignalView,
) []consoleLogView {
	type measurement struct {
		SeriesKey string          `json:"series_key"`
		Values    json.RawMessage `json:"values"`
	}
	signalBySeries := make(map[string]consoleSignalView, len(signals))
	for _, signal := range signals {
		if signal.SeriesKey != "" {
			signalBySeries[signal.SeriesKey] = signal
		}
	}
	views := make([]consoleLogView, 0, len(records))
	for _, record := range records {
		var payload measurement
		_ = json.Unmarshal(record.Record, &payload)
		signal, found := signalBySeries[payload.SeriesKey]
		sensorName := "名前未設定のセンサー"
		value := displayValues(payload.Values, nil)
		unit := ""
		if found {
			sensorName = signal.Name
			valueType := signal.ValueType
			precision := -1
			if signal.Profile != nil && signal.Profile.Complete() {
				precision = signal.Profile.DecimalPlaces
				if signal.Profile.DisplayValueKind == "boolean" {
					booleanType := "bool"
					valueType = &booleanType
				} else {
					numericType := "float"
					valueType = &numericType
				}
			}
			value = displayValuesWithPrecision(payload.Values, valueType, precision)
			unit = signal.Unit
		}
		views = append(views, consoleLogView{
			ReceivedAt: displayDateTime(record.ReceivedAt),
			EdgeNode:   record.EdgeNodeID,
			Sensor:     sensorName,
			Value:      value,
			Unit:       unit,
		})
	}
	return views
}

func newConsoleHistoryLogViews(records []store.HistoryRecord) []consoleLogView {
	views := make([]consoleLogView, 0, len(records))
	for _, record := range records {
		valueType := record.ValueType
		if record.DisplayValueKind == "boolean" {
			valueType = "bool"
		}
		views = append(views, consoleLogView{
			ReceivedAt: displayDateTime(record.ReceivedAt),
			ObservedAt: displayDateTime(record.ObservedAt),
			EdgeNode:   record.EdgeNodeID,
			Sensor:     record.DisplayName,
			SignalRef:  record.SignalRef,
			Value: displayValuesWithPrecision(
				record.Values, &valueType, record.DecimalPlaces,
			),
			Unit: record.Unit,
		})
	}
	return views
}

func newConsoleHistoryChart(
	series store.HistorySeries,
	signal *consoleSignalView,
) *consoleHistoryChart {
	if len(series.Points) == 0 {
		return nil
	}
	displayName := series.DisplayName
	unit := series.Unit
	decimalPlaces := 2
	if signal != nil {
		displayName = signal.Name
		unit = signal.Unit
		if signal.Profile != nil && signal.Profile.Complete() {
			decimalPlaces = signal.Profile.DecimalPlaces
		}
	}
	minimum, maximum := series.Points[0].Minimum, series.Points[0].Maximum
	for _, point := range series.Points[1:] {
		minimum = math.Min(minimum, point.Minimum)
		maximum = math.Max(maximum, point.Maximum)
	}
	dataMinimum, dataMaximum := minimum, maximum
	rangeValue := maximum - minimum
	if rangeValue == 0 {
		rangeValue = math.Max(math.Abs(maximum)*0.1, 1)
		minimum -= rangeValue / 2
		maximum += rangeValue / 2
	}
	const width, height, left, top, bottom = 920.0, 220.0, 24.0, 18.0, 24.0
	plotWidth, plotHeight := width-left-12, height-top-bottom
	firstTime := series.Points[0].BucketStart
	lastTime := series.Points[len(series.Points)-1].BucketStart
	timeRange := float64(lastTime - firstTime)
	if timeRange == 0 {
		timeRange = 1
	}
	var path strings.Builder
	for index, point := range series.Points {
		x := left + float64(point.BucketStart-firstTime)/timeRange*plotWidth
		if len(series.Points) == 1 {
			x = left + plotWidth/2
		}
		y := top + (maximum-point.Average)/(maximum-minimum)*plotHeight
		command := "L"
		if index == 0 {
			command = "M"
		}
		_, _ = fmt.Fprintf(&path, "%s %.2f %.2f ", command, x, y)
	}
	latest := series.Points[len(series.Points)-1].Average
	format := func(value float64) string {
		return strconv.FormatFloat(value, 'f', decimalPlaces, 64)
	}
	start := time.UnixMilli(firstTime).In(time.Local)
	end := time.UnixMilli(lastTime).In(time.Local)
	return &consoleHistoryChart{
		Path: path.String(), DisplayName: displayName, Unit: unit,
		Minimum: format(dataMinimum), Maximum: format(dataMaximum), Latest: format(latest),
		SampleCount: series.SampleCount,
		StartLabel:  start.Format("1/2 15:04"), EndLabel: end.Format("1/2 15:04"),
		AccessibleLabel: fmt.Sprintf(
			"受信値の推移。%s、%d件、最小%s%s、最大%s%s、最新%s%s",
			displayName, series.SampleCount, format(dataMinimum), unit,
			format(dataMaximum), unit, format(latest), unit,
		),
	}
}

func newConsoleAuditViews(events []edgeapp.AuditEvent) []consoleAuditView {
	views := make([]consoleAuditView, 0, len(events))
	for _, event := range events {
		if strings.HasPrefix(event.Operation, "session.") {
			continue
		}
		actor := displayActor(event.ActorClass)
		if event.ActorDisplayName != nil && *event.ActorDisplayName != "" {
			actor = *event.ActorDisplayName
		}
		outcomeClass := "failed"
		if event.Outcome == "success" {
			outcomeClass = "configured"
		}
		views = append(views, consoleAuditView{
			OccurredAt:   displayDateTime(event.OccurredAt),
			Actor:        actor,
			Operation:    displayOperation(event.Operation),
			Resource:     displayResource(event.ResourceRef),
			Outcome:      displayOutcome(event.Outcome),
			OutcomeClass: outcomeClass,
		})
	}
	return views
}

func newConsoleOutputRouteViews(
	routes []edgeapp.OutputRoute,
	rules []consoleRuleOption,
	now time.Time,
) []consoleOutputRouteView {
	rulesByID := make(map[string]consoleRuleOption, len(rules))
	for _, rule := range rules {
		rulesByID[rule.ID] = rule
	}
	views := make([]consoleOutputRouteView, 0, len(routes))
	for _, route := range routes {
		rule := rulesByID[route.RuleID]
		view := consoleOutputRouteView{
			OutputRoute:    route,
			RuleName:       rule.DisplayName,
			KindLabel:      rule.Kind,
			StateLabel:     "停止",
			StateClass:     "never",
			TransformLabel: "設定済み",
			TransformClass: "configured",
			DeliveryLabel:  "配送実績なし",
			DeliveryClass:  "never",
			SignalRef:      rule.SignalRef,
			SensorName:     rule.SensorName,
		}
		if view.RuleName == "" {
			view.RuleName = route.RuleDisplayName
		}
		if view.RuleName == "" {
			view.RuleName = "設定済みの値"
		}
		if view.KindLabel == "" && route.RuleKind != "" {
			view.KindLabel = displaySemanticKind(semantics.Kind(route.RuleKind))
		}
		if view.SignalRef == "" {
			view.SignalRef = route.SignalRef
		}
		switch route.AdapterID {
		case "iotkit.mqtt-json.v1":
			view.AdapterLabel = "IoTKit MQTT JSON v1"
			config, err := outputadapter.DecodeGenericMQTTJSONConfig(route.Config)
			if err == nil {
				view.Destination = config.Topic
			}
		case "yokakit.mqtt.v1":
			view.AdapterLabel = "YokaKit MQTT v1"
			config, err := outputadapter.DecodeYokaKitConfig(route.Config)
			if err == nil {
				view.Destination = config.SourceID + " / " + config.SignalID
			}
		default:
			view.AdapterLabel = route.AdapterID
		}
		if view.Destination == "" {
			view.Destination = "設定を確認してください"
		}
		if route.LastTransformSuccessAt != nil {
			view.TransformLabel = "変換確認済み"
		}
		if route.Active {
			view.StateLabel = "データ待ち"
			view.StateClass = "receiving"
		} else {
			view.TransformLabel = "停止中"
			view.TransformClass = "never"
		}
		switch {
		case outputRouteDeliveryStalled(route, now):
			view.DeliveryLabel = "要確認"
			view.DeliveryClass = "stale"
			if route.Active {
				view.StateLabel = "配送停止の可能性"
				view.StateClass = "stale"
			}
		case route.PendingCount > 0:
			view.DeliveryLabel = "配送中"
			view.DeliveryClass = "in-progress"
			if route.Active {
				view.StateLabel = "配送中"
				view.StateClass = "in-progress"
			}
		case route.PublishedCount > 0:
			view.DeliveryLabel = "待ちなし"
			view.DeliveryClass = "configured"
			if route.Active {
				view.StateLabel = "待ちなし"
				view.StateClass = "configured"
			}
		}
		switch {
		case route.LastPublishedAt != nil:
			age, _ := displayAge(route.LastPublishedAt, now)
			view.DeliveryDetail = fmt.Sprintf(
				"%d件送信 / %d件待ち · 最終配送 %s",
				route.PublishedCount,
				route.PendingCount,
				age,
			)
		case route.PendingCount > 0:
			view.DeliveryDetail = fmt.Sprintf(
				"%d件送信 / %d件待ち · まだ配送できていません",
				route.PublishedCount,
				route.PendingCount,
			)
		default:
			view.DeliveryDetail = fmt.Sprintf(
				"%d件送信 / %d件待ち",
				route.PublishedCount,
				route.PendingCount,
			)
		}
		if route.Active && route.LastTransformErrorCode != "" {
			view.TransformLabel = "変換エラー"
			view.TransformClass = "stale"
			view.StateLabel = "要確認"
			view.StateClass = "stale"
		}
		views = append(views, view)
	}
	return views
}

const outputDeliveryStallThreshold = 5 * time.Minute

func outputRouteDeliveryStalled(route edgeapp.OutputRoute, now time.Time) bool {
	if route.PendingCount == 0 || route.OldestPendingAt == nil {
		return false
	}
	return now.Sub(time.UnixMilli(*route.OldestPendingAt)) >=
		outputDeliveryStallThreshold
}

func displayOutputKind(kind string) string {
	switch kind {
	case "production":
		return "生産の累積値"
	case "onoff":
		return "ON/OFF状態"
	case "gantt_chart":
		return "稼働状態"
	case "alarm":
		return "アラーム"
	default:
		return "外部アプリ用データ"
	}
}

func displaySemanticKind(kind semantics.Kind) string {
	switch kind {
	case semantics.KindBoolean:
		return "ON/OFF状態"
	case semantics.KindCumulativeCounter:
		return "累積値"
	case semantics.KindAlarm:
		return "アラーム"
	default:
		return "数値"
	}
}

func displayActor(actor edgeapp.ActorClass) string {
	switch actor {
	case edgeapp.ActorLocalCLI:
		return "現地作業"
	case edgeapp.ActorSystem:
		return "システム"
	case edgeapp.ActorSettingsSession:
		return "設定担当者"
	default:
		return "担当者"
	}
}

func displayValues(raw json.RawMessage, valueType *string) string {
	return displayValuesWithPrecision(raw, valueType, -1)
}

func displayValuesWithPrecision(
	raw json.RawMessage,
	valueType *string,
	decimalPlaces int,
) string {
	if len(raw) == 0 {
		return "—"
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.UseNumber()
	var values []any
	if err := decoder.Decode(&values); err != nil || len(values) == 0 {
		return "—"
	}
	formatted := make([]string, 0, len(values))
	for _, value := range values {
		formatted = append(formatted, displayValue(value, valueType, decimalPlaces))
	}
	return strings.Join(formatted, " / ")
}

func displayValue(value any, valueType *string, decimalPlaces int) string {
	if valueType != nil && *valueType == "bool" {
		switch typed := value.(type) {
		case bool:
			if typed {
				return "ON"
			}
			return "OFF"
		case json.Number:
			number, _ := typed.Float64()
			if number != 0 {
				return "ON"
			}
			return "OFF"
		}
	}
	switch typed := value.(type) {
	case json.Number:
		number, err := typed.Float64()
		if err == nil {
			if decimalPlaces >= 0 {
				return strconv.FormatFloat(number, 'f', decimalPlaces, 64)
			}
			rounded := math.Round(number*1_000_000) / 1_000_000
			return strconv.FormatFloat(rounded, 'f', -1, 64)
		}
		return typed.String()
	case bool:
		if typed {
			return "ON"
		}
		return "OFF"
	case string:
		return typed
	default:
		return fmt.Sprint(typed)
	}
}

func displayProfileSensorType(profile *edgeapp.SignalProfile) string {
	if profile.DisplaySensorType == "custom" {
		return profile.DisplaySensorTypeLabel
	}
	value := profile.DisplaySensorType
	return displaySensorType(&value)
}

func normalizeDisplaySensorType(value string) string {
	if value == "temperature" {
		return "thermocouple"
	}
	return value
}

func displayValueType(valueType *string) string {
	switch pointerText(valueType, "") {
	case "bool":
		return "ON / OFF"
	case "float":
		return "数値（小数）"
	case "int":
		return "数値（整数）"
	case "record":
		return "複合値"
	default:
		return "未通知"
	}
}

func pointerText(value *string, fallback string) string {
	if value == nil || *value == "" {
		return fallback
	}
	return *value
}

func displayUnit(unit *string) string {
	if unit == nil || *unit == "" {
		return ""
	}
	switch *unit {
	case "Cel", "degC", "°C":
		return "°C"
	case "Percent", "%":
		return "%"
	default:
		return *unit
	}
}

func displaySensorType(sensorType *string) string {
	if sensorType == nil || *sensorType == "" {
		return "種類を確認中"
	}
	switch strings.ToLower(*sensorType) {
	case "thermocouple", "temperature", "temperature_c", "air_temperature":
		return "熱電対"
	case "contact", "contact_state", "digital_input":
		return "接点入力"
	case "light", "illuminance", "illuminance_lx", "illuminance_lux":
		return "照度"
	case "humidity", "relative_humidity":
		return "湿度"
	case "voltage", "voltage_mv":
		return "電圧"
	case "current", "current_ma":
		return "電流"
	case "distance", "distance_mm":
		return "距離"
	case "pressure", "differential_pressure_pa":
		return "圧力"
	case "acceleration", "acceleration_mg":
		return "加速度"
	default:
		return strings.ReplaceAll(*sensorType, "_", " ")
	}
}

func displayAge(timestamp *int64, now time.Time) (string, string) {
	if timestamp == nil {
		return "まだ受信していません", ""
	}
	received := time.UnixMilli(*timestamp).In(time.Local)
	title := received.Format("2006年1月2日 15:04:05")
	elapsed := now.Sub(received)
	switch {
	case elapsed < time.Minute:
		return "たった今", title
	case elapsed < time.Hour:
		return fmt.Sprintf("%d分前", int(elapsed/time.Minute)), title
	case elapsed < 24*time.Hour:
		return fmt.Sprintf("%d時間前", int(elapsed/time.Hour)), title
	case elapsed < 7*24*time.Hour:
		return fmt.Sprintf("%d日前", int(elapsed/(24*time.Hour))), title
	default:
		return received.Format("1月2日 15:04"), title
	}
}

func displayDateTime(timestamp int64) string {
	return time.UnixMilli(timestamp).In(time.Local).Format("1月2日 15:04:05")
}

func displayOperation(operation string) string {
	labels := map[string]string{
		"device_profile.update":          "デバイス情報を変更",
		"signal_profile.update":          "センサー表示を変更",
		"semantic_definition.put":        "センサー設定を保存",
		"semantic_definition.deactivate": "センサー設定を停止",
		"semantic_counter.reset":         "累積値を0に戻す",
		"export_profile.activate":        "外部出力先を追加",
		"export_profile.stop_requested":  "外部出力先の終了を開始",
		"output_binding.configure":       "外部出力の用途を設定",
		"output_binding.start":           "外部出力の送信を開始",
		"edge_node.activation.request":   "収集ノードの登録を開始",
		"account.create":                 "アカウントを発行",
		"account.update":                 "アカウント情報を変更",
		"account.disable":                "アカウントを無効化",
		"account.password_replace":       "パスワードを変更",
		"session.login":                  "ログイン",
		"session.login_failed":           "ログインに失敗",
		"session.logout":                 "ログアウト",
	}
	if label, ok := labels[operation]; ok {
		return label
	}
	return "システム設定を変更"
}

func displayResource(resource string) string {
	switch {
	case strings.HasPrefix(resource, "dev_"):
		return "デバイス"
	case strings.HasPrefix(resource, "sig_"):
		return "センサー"
	case strings.HasPrefix(resource, "sem_"):
		return "センサー設定"
	case strings.HasPrefix(resource, "acct_"):
		return "アカウント"
	case strings.HasPrefix(resource, "out_"):
		return "外部出力"
	case strings.HasPrefix(resource, "edge_node_"):
		return "収集ノード"
	default:
		return "IoTKit Edge"
	}
}

func displayOutcome(outcome string) string {
	if outcome == "success" {
		return "完了"
	}
	return "失敗"
}
