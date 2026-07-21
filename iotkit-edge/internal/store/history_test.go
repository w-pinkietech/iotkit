package store

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func seedHistorySignal(t *testing.T, store *Store, edgeNode, signalRef, seriesKey string) {
	t.Helper()
	if _, err := store.db.Exec(`
		INSERT INTO edge_signals(
			signal_ref, edge_node_id, series_key, system_id,
			last_received_at, created_at
		) VALUES (?, ?, ?, ?, ?, ?)
	`, signalRef, edgeNode, seriesKey, "device-01", int64(4_000), int64(1)); err != nil {
		t.Fatal(err)
	}
	if _, err := store.db.Exec(`
		INSERT INTO descriptor_signals(
			edge_node_id, series_key, system_id, measurement_key,
			channel_index, variant, unit, value_type, presence,
			descriptor_revision, updated_at
		) VALUES (?, ?, ?, ?, NULL, 'primary', 'degC', 'float', 'current', 1, 1)
	`, edgeNode, seriesKey, "device-01", "temperature_c"); err != nil {
		t.Fatal(err)
	}
	if _, err := store.db.Exec(`
		INSERT INTO signal_profiles(
			edge_node_id, series_key, display_name, revision, updated_at,
			display_sensor_type, display_sensor_type_label,
			display_value_kind, display_unit_mode, display_unit,
			decimal_places
		) VALUES (?, ?, '乾燥炉温度', 1, 1, 'thermocouple', '',
			'numeric', 'custom', '℃', 1)
	`, edgeNode, seriesKey); err != nil {
		t.Fatal(err)
	}
}

func seedHistoryRecord(
	t *testing.T,
	store *Store,
	edgeNode, epoch, seriesKey string,
	seq, receivedAt int64,
	value float64,
) {
	t.Helper()
	payload, err := json.Marshal(map[string]any{
		"family":         "measurement",
		"schema_version": 1,
		"epoch":          epoch,
		"pub_seq":        seq,
		"series_key":     seriesKey,
		"event_time":     receivedAt - 25,
		"values":         []float64{value},
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.db.Exec(`
		INSERT INTO raw_records(
			edge_node_id, ledger_epoch, pub_seq, publication_id,
			record_json, record_sha256, received_at
		) VALUES (?, ?, ?, ?, ?, zeroblob(32), ?)
	`, edgeNode, epoch, seq, edgeNode+":"+epoch, payload, receivedAt); err != nil {
		t.Fatal(err)
	}
}

func seedSemanticHistoryObservation(
	t *testing.T,
	store *Store,
	ruleID, signalRef, edgeNodeID, kind, seriesID, value string,
	rowID, sequence, observedAt int64,
) {
	t.Helper()
	if _, err := store.db.Exec(`
		INSERT INTO semantic_observations_v3(
			observation_row_id, observation_id, rule_id, rule_revision,
			calibration_revision, series_id, sequence, kind, value_json,
			signal_ref, edge_node_id, ledger_epoch, source_pub_seq,
			observed_at, created_at
		) VALUES (?, ?, ?, 3, 2, ?, ?, ?, ?, ?, ?, 'epoch-a', ?, ?, ?)
	`, rowID, "observation-"+ruleID+value, ruleID, seriesID, sequence,
		kind, value, signalRef, edgeNodeID, rowID, observedAt, observedAt+25); err != nil {
		t.Fatal(err)
	}
}

func TestQueryHistoryFiltersAndUsesStableCursor(t *testing.T) {
	archive := openTestStore(t)
	seedHistorySignal(t, archive, "edge-a", "signal-a", "device-01:temperature_c:na:primary")
	seedHistorySignal(t, archive, "edge-b", "signal-b", "device-01:temperature_c:na:primary")
	seedHistoryRecord(t, archive, "edge-a", "epoch-a", "device-01:temperature_c:na:primary", 1, 1_000, 20)
	seedHistoryRecord(t, archive, "edge-a", "epoch-a", "device-01:temperature_c:na:primary", 2, 2_000, 21)
	seedHistoryRecord(t, archive, "edge-a", "epoch-a", "device-01:temperature_c:na:primary", 3, 3_000, 22)
	seedHistoryRecord(t, archive, "edge-b", "epoch-b", "device-01:temperature_c:na:primary", 1, 3_000, 99)

	page, err := archive.QueryHistory(context.Background(), HistoryQuery{
		SignalRef: "signal-a", From: 1_500, Until: 4_000, Limit: 1,
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Records) != 1 || !page.HasMore || page.Next == nil {
		t.Fatalf("first page = %#v", page)
	}
	if page.Records[0].ReceivedAt != 3_000 || page.Records[0].DisplayName != "乾燥炉温度" {
		t.Fatalf("first record = %#v", page.Records[0])
	}
	if string(page.Records[0].Values) != "[22]" || page.Records[0].Unit != "℃" {
		t.Fatalf("first values = %s unit=%q", page.Records[0].Values, page.Records[0].Unit)
	}

	next, err := archive.QueryHistory(context.Background(), HistoryQuery{
		SignalRef: "signal-a", From: 1_500, Until: 4_000, Limit: 10,
		Before: page.Next,
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(next.Records) != 1 || next.Records[0].ReceivedAt != 2_000 || next.HasMore {
		t.Fatalf("second page = %#v", next)
	}
}

func TestQueryHistorySeriesAggregatesBoundedBuckets(t *testing.T) {
	archive := openTestStore(t)
	series := "device-01:temperature_c:na:primary"
	seedHistorySignal(t, archive, "edge-a", "signal-a", series)
	seedHistoryRecord(t, archive, "edge-a", "epoch-a", series, 1, 1_000, 10)
	seedHistoryRecord(t, archive, "edge-a", "epoch-a", series, 2, 1_200, 20)
	seedHistoryRecord(t, archive, "edge-a", "epoch-a", series, 3, 2_200, 30)

	result, err := archive.QueryHistorySeries(context.Background(), HistorySeriesQuery{
		SignalRef: "signal-a", From: 1_000, Until: 3_000, BucketMilliseconds: 1_000,
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(result.Points) != 2 || result.SampleCount != 3 {
		t.Fatalf("series = %#v", result)
	}
	if result.Points[0].Minimum != 10 || result.Points[0].Average != 15 ||
		result.Points[0].Maximum != 20 || result.Points[0].SampleCount != 2 {
		t.Fatalf("first bucket = %#v", result.Points[0])
	}
	if result.Points[1].Average != 30 || result.Points[1].SampleCount != 1 {
		t.Fatalf("second bucket = %#v", result.Points[1])
	}
}

func TestHistoryDoesNotDisplayAUnitForDimensionlessProfiles(t *testing.T) {
	archive := openTestStore(t)
	series := "device-01:contact:na:primary"
	seedHistorySignal(t, archive, "edge-a", "signal-a", series)
	if _, err := archive.db.Exec(`
		UPDATE signal_profiles
		SET display_unit_mode = 'dimensionless', display_unit = ''
		WHERE edge_node_id = 'edge-a' AND series_key = ?
	`, series); err != nil {
		t.Fatal(err)
	}
	seedHistoryRecord(t, archive, "edge-a", "epoch-a", series, 1, 1_000, 1)
	page, err := archive.QueryHistory(context.Background(), HistoryQuery{
		SignalRef: "signal-a", From: 0, Until: 2_000, Limit: 10,
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Records) != 1 || page.Records[0].Unit != "" {
		t.Fatalf("dimensionless history = %#v", page.Records)
	}
}

func TestHistoryQueriesRejectUnboundedRanges(t *testing.T) {
	archive := openTestStore(t)
	if _, err := archive.QueryHistory(context.Background(), HistoryQuery{
		From: 0, Until: maxHistoryRangeMilliseconds + 1, Limit: 100,
	}); err == nil {
		t.Fatal("QueryHistory accepted a range longer than the product boundary")
	}
	if _, err := archive.QueryHistorySeries(context.Background(), HistorySeriesQuery{
		SignalRef: "signal-a", From: 10, Until: 10, BucketMilliseconds: 1_000,
	}); err == nil {
		t.Fatal("QueryHistorySeries accepted an empty range")
	}
}

func TestOpenCreatesHistoryTimelineIndex(t *testing.T) {
	archive := openTestStore(t)
	var definition string
	if err := archive.db.QueryRow(`
		SELECT sql FROM sqlite_master
		WHERE type = 'index' AND name = 'idx_raw_records_history_received'
	`).Scan(&definition); err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{"received_at", "edge_node_id", "ledger_epoch", "pub_seq"} {
		if !strings.Contains(definition, want) {
			t.Fatalf("history index missing %q: %s", want, definition)
		}
	}
}

func TestQuerySemanticHistoryReturnsPersistedRuleResultsAndMetadata(t *testing.T) {
	archive := openTestStore(t)
	series := "device-01:temperature_c:na:primary"
	seedHistorySignal(t, archive, "edge-a", "signal-a", series)
	if _, err := archive.db.Exec(`
		INSERT INTO semantic_rules_v3(
			rule_id, signal_ref, display_name, kind, series_id,
			display_order, created_at
		) VALUES
			('rule-numeric', 'signal-a', '補正温度', 'numeric', 'series-numeric', 1, 1),
			('rule-alarm', 'signal-a', '高温警報', 'alarm', 'series-alarm', 2, 1)
	`); err != nil {
		t.Fatal(err)
	}
	seedSemanticHistoryObservation(
		t, archive, "rule-numeric", "signal-a", "edge-a", "numeric",
		"series-numeric", "21.5", 1, 1, 1_000,
	)
	seedSemanticHistoryObservation(
		t, archive, "rule-alarm", "signal-a", "edge-a", "alarm",
		"series-alarm", "true", 2, 1, 2_000,
	)

	page, err := archive.QuerySemanticHistory(context.Background(), SemanticHistoryQuery{
		SignalRef: "signal-a", EdgeNodeID: "edge-a",
		From: 500, Until: 2_500, Limit: 10,
	})
	if err != nil {
		t.Fatal(err)
	}
	if page.HasMore || len(page.Records) != 2 {
		t.Fatalf("semantic history page = %#v", page)
	}
	alarm := page.Records[0]
	if alarm.RuleName != "高温警報" || alarm.Kind != "alarm" ||
		string(alarm.Value) != "true" || alarm.Unit != "" ||
		alarm.RuleRevision != 3 || alarm.CalibrationRevision != 2 {
		t.Fatalf("alarm row = %#v", alarm)
	}
	numeric := page.Records[1]
	if numeric.SensorName != "乾燥炉温度" || numeric.RuleName != "補正温度" ||
		string(numeric.Value) != "21.5" || numeric.Unit != "℃" ||
		numeric.ProcessedAt != 1_025 {
		t.Fatalf("numeric row = %#v", numeric)
	}
}

func TestQuerySemanticHistoryDetectsRowsBeyondLimit(t *testing.T) {
	archive := openTestStore(t)
	series := "device-01:temperature_c:na:primary"
	seedHistorySignal(t, archive, "edge-a", "signal-a", series)
	if _, err := archive.db.Exec(`
		INSERT INTO semantic_rules_v3(
			rule_id, signal_ref, display_name, kind, series_id,
			display_order, created_at
		) VALUES ('rule-numeric', 'signal-a', '補正温度', 'numeric',
			'series-numeric', 1, 1)
	`); err != nil {
		t.Fatal(err)
	}
	seedSemanticHistoryObservation(
		t, archive, "rule-numeric", "signal-a", "edge-a", "numeric",
		"series-numeric", "20", 1, 1, 1_000,
	)
	seedSemanticHistoryObservation(
		t, archive, "rule-numeric", "signal-a", "edge-a", "numeric",
		"series-numeric", "21", 2, 2, 2_000,
	)

	page, err := archive.QuerySemanticHistory(context.Background(), SemanticHistoryQuery{
		From: 0, Until: 3_000, Limit: 1,
	})
	if err != nil {
		t.Fatal(err)
	}
	if !page.HasMore || len(page.Records) != 1 || page.Records[0].ObservedAt != 2_000 {
		t.Fatalf("limited semantic history = %#v", page)
	}
}

func TestOpenCreatesSemanticHistoryTimelineIndex(t *testing.T) {
	archive := openTestStore(t)
	var definition string
	if err := archive.db.QueryRow(`
		SELECT sql FROM sqlite_master
		WHERE type = 'index' AND name = 'idx_semantic_observations_history'
	`).Scan(&definition); err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{"observed_at", "observation_row_id"} {
		if !strings.Contains(definition, want) {
			t.Fatalf("semantic history index missing %q: %s", want, definition)
		}
	}
}
