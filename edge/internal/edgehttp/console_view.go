package edgehttp

import (
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
	"strconv"
	"time"
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

type consoleOnboardingFacts struct {
	ActiveEdgeNodes     int
	DeviceCount         int
	PendingDevices      int
	SignalCount         int
	UnconfiguredSignals int
	SemanticRules       int
}

type consoleOnboardingStep struct {
	Number      int
	Title       string
	Description string
	Href        string
	Complete    bool
	Current     bool
}

type consoleOnboardingView struct {
	Show          bool
	CompleteCount int
	TotalCount    int
	NextTitle     string
	NextHref      string
	Steps         []consoleOnboardingStep
}

func newConsoleOnboardingView(facts consoleOnboardingFacts) consoleOnboardingView {
	steps := []consoleOnboardingStep{
		{
			Number: 1, Title: "収集ノードを登録",
			Description: "接続元を確認し、IoTKit Edgeへのデータ送信を許可します。",
			Href:        "/equipment",
			Complete:    facts.ActiveEdgeNodes > 0,
		},
		{
			Number: 2, Title: "デバイス名と設置場所を設定",
			Description: "現場で見分けられる名前と、設置されている場所を登録します。",
			Href:        "/equipment",
			Complete:    facts.DeviceCount > 0 && facts.PendingDevices == 0,
		},
		{
			Number: 3, Title: "センサーの種類と単位を確認",
			Description: "受信値を、温度・接点入力など人が分かる表示にします。",
			Href:        "/equipment",
			Complete:    facts.SignalCount > 0 && facts.UnconfiguredSignals == 0,
		},
		{
			Number: 4, Title: "センサーの値の使い方を設定",
			Description: "通常値、累積値、状態、アラームから必要な使い方を作ります。",
			Href:        "/sensors",
			Complete:    facts.SemanticRules > 0,
		},
	}
	view := consoleOnboardingView{Show: true, TotalCount: len(steps), Steps: steps}
	for index := range view.Steps {
		if view.Steps[index].Complete {
			view.CompleteCount++
			continue
		}
		if view.NextTitle == "" {
			view.Steps[index].Current = true
			view.NextTitle = view.Steps[index].Title
			view.NextHref = view.Steps[index].Href
		}
	}
	view.Show = view.CompleteCount < view.TotalCount
	return view
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
	ProfileLabel        string
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
	GrowthPerDay        string
	DaysRemainingKnown  bool
	DaysRemaining       int64
	ReserveLabel        string
}

func newConsoleStorageView(status store.StorageStatus) consoleStorageView {
	view := consoleStorageView{
		ProfileLabel: map[store.Profile]string{
			store.ProfileEmbedded: "内蔵データベース",
			store.ProfilePostgres: "PostgreSQL",
		}[status.Profile],
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
		GrowthPerDay:       formatByteCount(uint64(max(status.GrowthBytesPerDay, 0))),
	}
	if status.EstimatedDaysRemaining != nil {
		view.DaysRemainingKnown = true
		view.DaysRemaining = *status.EstimatedDaysRemaining
	}
	view.ReserveLabel = map[string]string{
		"adequate": "十分", "warning": "注意", "critical": "危険", "unknown": "host監視で確認",
	}[status.AbsoluteReserveState]
	if view.ProfileLabel == "" {
		view.ProfileLabel = "不明"
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
