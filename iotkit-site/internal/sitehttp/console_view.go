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
	Name              string
	Value             string
	Unit              string
	SensorType        string
	LastReceived      string
	LastReceivedTitle string
	StatusLabel       string
	StatusClass       string
	SettingLabel      string
	SettingClass      string
	Definition        *semantics.Definition
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
	Signal     string
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
	}
	if view.Name == "" {
		view.Name = "未設定の信号"
	}
	if summary.Latest != nil {
		view.Value = displayValues(summary.Latest.Values, summary.ValueType)
	}
	if summary.HasSemanticMapping {
		view.SettingLabel = "設定済み"
		view.SettingClass = "configured"
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
		views = append(views, view)
	}
	return views
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
		view.Name = "未設定のデバイス"
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
		name := "未設定の信号"
		value := displayValues(payload.Values, nil)
		unit := ""
		if found {
			name = signal.Name
			value = displayValues(payload.Values, signal.ValueType)
			unit = signal.Unit
		}
		views = append(views, consoleLogView{
			ReceivedAt: displayDateTime(record.ReceivedAt),
			Edge:       record.EdgeNodeID,
			Signal:     name,
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
			name = "未設定の信号"
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
		formatted = append(formatted, displayValue(value, valueType))
	}
	return strings.Join(formatted, " / ")
}

func displayValue(value any, valueType *string) string {
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
	case "temperature", "temperature_c", "air_temperature":
		return "温度"
	case "contact", "contact_state", "digital_input":
		return "接点入力"
	case "light", "illuminance", "illuminance_lx":
		return "光"
	case "humidity", "relative_humidity":
		return "湿度"
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
		"signal_profile.update":          "信号名を変更",
		"semantic_definition.put":        "意味付けを保存",
		"semantic_definition.deactivate": "意味付けを停止",
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
		return "信号"
	case strings.HasPrefix(resource, "sem_"):
		return "信号の意味付け"
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
