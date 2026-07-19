package sitehttp

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math"
	"strconv"
	"strings"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/store"
)

type consoleSignalView struct {
	siteapp.SignalSummary
	Name                string
	Value               string
	Unit                string
	SensorType          string
	LastReceived        string
	LastReceivedTitle   string
	StatusLabel         string
	StatusClass         string
	SettingLabel        string
	SettingClass        string
	MeaningLabel        string
	MeaningClass        string
	FormProfile         siteapp.SignalProfileInput
	Definition          *semantics.Definition
	SourceSensorType    string
	SourceValueType     string
	SourceUnit          string
	ChannelLabel        string
	DeviceName          string
	DeviceLocation      string
	InputIsBoolean      bool
	RiseDebounceSeconds string
	FallDebounceSeconds string
	Configuration       *semantics.Configuration
	NormalRules         []consoleSemanticRuleView
	AlarmRules          []consoleSemanticRuleView
}

type consoleSemanticRuleView struct {
	semantics.Rule
	KindLabel           string
	RiseDebounceSeconds string
	FallDebounceSeconds string
}

type consoleEdgeView struct {
	siteapp.Edge
	Name                string
	LocationLabel       string
	StateLabel          string
	StateClass          string
	LastCommunication   string
	LastCommunicationAt string
	LastResult          string
	CanActivate         bool
}

func newConsoleEdgeViews(edges []siteapp.Edge, now time.Time) []consoleEdgeView {
	views := make([]consoleEdgeView, 0, len(edges))
	for _, edge := range edges {
		view := consoleEdgeView{
			Edge:          edge,
			Name:          edge.DisplayName,
			LocationLabel: edge.Location,
			CanActivate:   edge.State == siteapp.EdgeDiscovered,
		}
		if view.Name == "" {
			view.Name = edge.EdgeNodeID
		}
		if view.LocationLabel == "" {
			view.LocationLabel = "設置場所 未設定"
		}
		view.LastCommunication, view.LastCommunicationAt = displayAge(
			edge.LastDescriptorAt, now,
		)
		view.LastResult, _ = displayAge(edge.LastResultAt, now)
		switch edge.State {
		case siteapp.EdgeDiscovered:
			view.StateLabel, view.StateClass = "未登録", "needs-setup"
		case siteapp.EdgeActivating:
			view.StateLabel, view.StateClass = "登録処理中", "stale"
		case siteapp.EdgeActive:
			view.StateLabel, view.StateClass = "登録済み", "configured"
		case siteapp.EdgeRecoveryHold:
			view.StateLabel, view.StateClass = "復旧確認待ち", "stale"
		default:
			view.StateLabel, view.StateClass = "状態不明", "stale"
		}
		views = append(views, view)
	}
	return views
}

type consoleEquipmentEdgeView struct {
	consoleEdgeView
	Devices            []consoleSetupDeviceView
	DevicePendingCount int
	SensorPendingCount int
}

func newConsoleEquipmentViews(
	edges []siteapp.Edge,
	devices []siteapp.SetupDevice,
	now time.Time,
) []consoleEquipmentEdgeView {
	devicesByEdge := make(map[string][]siteapp.SetupDevice)
	for _, device := range devices {
		devicesByEdge[device.Device.Edge] = append(
			devicesByEdge[device.Device.Edge],
			device,
		)
	}

	edgeViews := newConsoleEdgeViews(edges, now)
	views := make([]consoleEquipmentEdgeView, 0, len(edgeViews))
	for _, edge := range edgeViews {
		edgeDevices := devicesByEdge[edge.EdgeNodeID]
		view := consoleEquipmentEdgeView{
			consoleEdgeView: edge,
			Devices:         newConsoleSetupDeviceViews(edgeDevices, now),
		}
		for _, device := range edgeDevices {
			if device.State == siteapp.SetupWaitingForDevice {
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
	edges []siteapp.Edge,
	devices []siteapp.SetupDevice,
	now time.Time,
) []consoleSetupDeviceView {
	knownEdges := make(map[string]struct{}, len(edges))
	for _, edge := range edges {
		knownEdges[edge.EdgeNodeID] = struct{}{}
	}
	orphans := make([]siteapp.SetupDevice, 0)
	for _, device := range devices {
		if _, exists := knownEdges[device.Device.Edge]; !exists {
			orphans = append(orphans, device)
		}
	}
	return newConsoleSetupDeviceViews(orphans, now)
}

type consoleSetupDeviceView struct {
	siteapp.SetupDevice
	Name              string
	LocationLabel     string
	StateLabel        string
	StateClass        string
	LastReceived      string
	LastReceivedTitle string
	Signals           []consoleSetupSignalView
}

type consoleSetupSignalView struct {
	siteapp.SetupSignal
	RawValue          string
	RawUnit           string
	MeasurementKey    string
	ValueTypeLabel    string
	ChannelLabel      string
	LastReceived      string
	LastReceivedTitle string
	FormProfile       siteapp.SignalProfileInput
	ProfileRevision   *int64
	MissingMessage    string
}

type consoleDeviceView struct {
	siteapp.DeviceSummary
	Name              string
	LocationLabel     string
	LastReceived      string
	LastReceivedTitle string
	StatusLabel       string
	StatusClass       string
}

type consoleLogView struct {
	ReceivedAt string
	Edge       string
	Sensor     string
	Value      string
	Unit       string
}

type consoleAuditView struct {
	OccurredAt   string
	Actor        string
	Operation    string
	Resource     string
	Outcome      string
	OutcomeClass string
}

type consoleDefinitionOption struct {
	ID   string
	Name string
	Kind string
}

type consoleOutputView struct {
	store.YokaKitRoute
	KindLabel  string
	StateLabel string
	StateClass string
}

type consoleRuleOption struct {
	ID   string
	Name string
	Kind string
}

type consoleRuleOutputView struct {
	store.YokaKitRuleRoute
	KindLabel  string
	StateLabel string
	StateClass string
}

func newConsoleSignalView(
	summary siteapp.SignalSummary,
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
	view.FormProfile, _ = siteapp.SignalProfileCandidate(summary)
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
	devices []siteapp.DeviceSummary,
) {
	deviceByRef := make(map[string]siteapp.DeviceSummary, len(devices))
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
	}
}

func newConsoleSetupDeviceViews(
	devices []siteapp.SetupDevice,
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
		case siteapp.SetupReady:
			view.StateLabel, view.StateClass = "登録済み", "configured"
		case siteapp.SetupWaitingForDevice:
			view.StateLabel, view.StateClass = "デバイス情報を入力", "needs-setup"
		case siteapp.SetupMetadataMissing:
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
	signal siteapp.SetupSignal,
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
		view.MissingMessage = "Adapterから種類・単位が届いていません。現場で確認して入力してください。"
	}
	return view
}

func newConsoleSignalViews(
	summaries []siteapp.SignalSummary,
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

func formatMillisecondsAsSeconds(milliseconds int64) string {
	return strconv.FormatFloat(float64(milliseconds)/1000, 'f', -1, 64)
}

func newConsoleDeviceView(
	summary siteapp.DeviceSummary,
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
			Edge:       record.EdgeNodeID,
			Sensor:     sensorName,
			Value:      value,
			Unit:       unit,
		})
	}
	return views
}

func newConsoleAuditViews(events []siteapp.AuditEvent) []consoleAuditView {
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

func newConsoleDefinitionOptions(
	definitions []semantics.Definition,
	signals []consoleSignalView,
) []consoleDefinitionOption {
	nameBySignal := make(map[string]string, len(signals))
	for _, signal := range signals {
		nameBySignal[signal.SignalRef] = signal.Name
	}
	options := make([]consoleDefinitionOption, 0, len(definitions))
	for _, definition := range definitions {
		if !definition.Active {
			continue
		}
		name := nameBySignal[definition.SignalRef]
		if name == "" {
			name = "名前未設定のセンサー"
		}
		options = append(options, consoleDefinitionOption{
			ID: definition.ID, Name: name, Kind: displaySemanticKind(definition.Kind),
		})
	}
	return options
}

func newConsoleOutputViews(routes []store.YokaKitRoute) []consoleOutputView {
	views := make([]consoleOutputView, 0, len(routes))
	for _, route := range routes {
		view := consoleOutputView{
			YokaKitRoute: route,
			KindLabel:    displayOutputKind(string(route.Kind)),
			StateLabel:   "停止",
			StateClass:   "never",
		}
		if route.Active {
			view.StateLabel = "使用中"
			view.StateClass = "receiving"
		}
		views = append(views, view)
	}
	return views
}

func newConsoleRuleOutputViews(
	routes []store.YokaKitRuleRoute,
) []consoleRuleOutputView {
	views := make([]consoleRuleOutputView, 0, len(routes))
	for _, route := range routes {
		view := consoleRuleOutputView{
			YokaKitRuleRoute: route,
			KindLabel:        displayOutputKind(string(route.Kind)),
			StateLabel:       "停止",
			StateClass:       "never",
		}
		if route.Active {
			view.StateLabel = "使用中"
			view.StateClass = "receiving"
		}
		views = append(views, view)
	}
	return views
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

func displayActor(actor siteapp.ActorClass) string {
	switch actor {
	case siteapp.ActorLocalCLI:
		return "現地作業"
	case siteapp.ActorSystem:
		return "システム"
	case siteapp.ActorSettingsSession:
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

func displayProfileSensorType(profile *siteapp.SignalProfile) string {
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
		"yokakit_route.create":           "外部出力を追加",
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
	default:
		return "Site"
	}
}

func displayOutcome(outcome string) string {
	if outcome == "success" {
		return "完了"
	}
	return "失敗"
}
