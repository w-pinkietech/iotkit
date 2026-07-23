package edgehttp

import (
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
	"strconv"
	"time"
)

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
