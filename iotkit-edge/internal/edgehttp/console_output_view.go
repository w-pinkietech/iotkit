package edgehttp

import (
	"bytes"
	"encoding/json"
	"fmt"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
	"sort"
	"time"
)

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
	AdapterLabel             string
	StateLabel               string
	StateClass               string
	KindLabel                string
	ModeLabel                string
	Preview                  *edgeapp.OutputPublicationPreview
	PreviewLabel             string
	PrettyPayload            string
	HasDiagnostics           bool
	TransformLabel           string
	TransformClass           string
	DeliveryLabel            string
	DeliveryClass            string
	PendingCount             int64
	NeedsAttention           bool
	TransformError           bool
	DeliveryAttention        bool
	RegistrationAction       bool
	SharesSensorRegistration bool
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
		case "pinikiet.mqtt.v1":
			view.AdapterLabel = "Pinikiet"
		}
		switch profile.State {
		case edgeapp.ExportProfilePreparing:
			view.StateLabel, view.StateClass = "送信前の準備中", "needs-setup"
		case edgeapp.ExportProfileActive:
			view.StateLabel, view.StateClass = "使用中", "configured"
		case edgeapp.ExportProfileDraining:
			view.StateLabel, view.StateClass = "配送終了処理中", "in-progress"
		}
		registrationSensors := map[string]bool{}
		for _, binding := range profile.Bindings {
			if binding.State == edgeapp.OutputBindingActive && binding.SensorID != "" {
				registrationSensors[binding.SensorID] = true
			}
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
				if binding.SensorID == "" || !registrationSensors[binding.SensorID] {
					bindingView.RegistrationAction = true
					registrationSensors[binding.SensorID] = true
				} else {
					bindingView.SharesSensorRegistration = true
				}
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
		case "pinikiet.mqtt.v1":
			view.AdapterLabel = "Pinikiet MQTT v1"
			config, err := outputadapter.DecodePinikietConfig(route.Config)
			if err == nil {
				view.Destination = config.SourceID + " / " + config.SensorID
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
