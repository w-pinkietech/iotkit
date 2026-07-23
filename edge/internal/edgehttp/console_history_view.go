package edgehttp

import (
	"encoding/json"
	"fmt"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
	"math"
	"strconv"
	"strings"
	"time"
)

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
