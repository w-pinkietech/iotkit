package edgehttp

import (
	"bytes"
	"encoding/json"
	"fmt"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
	"math"
	"strconv"
	"strings"
	"time"
)

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
