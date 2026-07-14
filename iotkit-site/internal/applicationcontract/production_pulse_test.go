package applicationcontract

import (
	"bytes"
	"encoding/json"
	"reflect"
	"testing"
)

func TestProductionPulseV1RoundTrip(t *testing.T) {
	event := ProductionPulseV1{
		SchemaVersion:   1,
		EventID:         "event-01",
		MappingID:       "sm-01",
		MappingRevision: 1,
		EventSequence:   2,
		Meaning:         "production_pulse",
		EdgeNodeID:      "edge-node-01",
		SourceSeriesKey: "subject:contact_state:na:primary",
		SourcePubSeq:    8,
		OccurredAt:      1_720_000_000_000,
		Count:           2,
	}
	if err := event.Validate(); err != nil {
		t.Fatal(err)
	}
	encoded, err := json.Marshal(event)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(encoded, []byte("ipAddress")) || bytes.Contains(encoded, []byte("pinNumber")) {
		t.Fatalf("legacy coordinate leaked: %s", encoded)
	}

	var fields map[string]json.RawMessage
	if err := json.Unmarshal(encoded, &fields); err != nil {
		t.Fatal(err)
	}
	wantFields := []string{
		"schema_version", "event_id", "mapping_id", "mapping_revision",
		"event_sequence", "meaning", "edge_node_id", "source_series_key",
		"source_pub_seq", "occurred_at", "count",
	}
	gotFields := make([]string, 0, len(fields))
	for _, field := range wantFields {
		if _, ok := fields[field]; ok {
			gotFields = append(gotFields, field)
		}
	}
	if len(fields) != len(wantFields) || !reflect.DeepEqual(gotFields, wantFields) {
		t.Fatalf("JSON fields = %v, want exactly %v; payload=%s", fields, wantFields, encoded)
	}

	var decoded ProductionPulseV1
	if err := json.Unmarshal(encoded, &decoded); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(decoded, event) {
		t.Fatalf("decoded = %#v, want %#v", decoded, event)
	}
}

func TestProductionPulseV1AcceptsExplicitUnixEpoch(t *testing.T) {
	event := ProductionPulseV1{
		SchemaVersion:   1,
		EventID:         "event-epoch",
		MappingID:       "sm-01",
		MappingRevision: 1,
		EventSequence:   1,
		Meaning:         "production_pulse",
		EdgeNodeID:      "edge-node-01",
		SourceSeriesKey: "subject:contact_state:na:primary",
		SourcePubSeq:    1,
		OccurredAt:      0,
		Count:           1,
	}
	if err := event.Validate(); err != nil {
		t.Fatalf("Validate rejected explicit Unix epoch: %v", err)
	}
}

func TestProductionPulseV1ValidateRejectsInvalidContract(t *testing.T) {
	valid := ProductionPulseV1{
		SchemaVersion:   1,
		EventID:         "event-01",
		MappingID:       "sm-01",
		MappingRevision: 1,
		EventSequence:   2,
		Meaning:         "production_pulse",
		EdgeNodeID:      "edge-node-01",
		SourceSeriesKey: "subject:contact_state:na:primary",
		SourcePubSeq:    8,
		OccurredAt:      1_720_000_000_000,
		Count:           2,
	}
	tests := []struct {
		name   string
		mutate func(*ProductionPulseV1)
	}{
		{name: "schema version", mutate: func(event *ProductionPulseV1) { event.SchemaVersion = 2 }},
		{name: "event ID", mutate: func(event *ProductionPulseV1) { event.EventID = "" }},
		{name: "mapping ID", mutate: func(event *ProductionPulseV1) { event.MappingID = "" }},
		{name: "mapping revision", mutate: func(event *ProductionPulseV1) { event.MappingRevision = 0 }},
		{name: "event sequence", mutate: func(event *ProductionPulseV1) { event.EventSequence = 0 }},
		{name: "meaning", mutate: func(event *ProductionPulseV1) { event.Meaning = "production" }},
		{name: "edge node ID", mutate: func(event *ProductionPulseV1) { event.EdgeNodeID = "" }},
		{name: "source series key", mutate: func(event *ProductionPulseV1) { event.SourceSeriesKey = "" }},
		{name: "source pub seq", mutate: func(event *ProductionPulseV1) { event.SourcePubSeq = 0 }},
		{name: "occurred at", mutate: func(event *ProductionPulseV1) { event.OccurredAt = -1 }},
		{name: "count", mutate: func(event *ProductionPulseV1) { event.Count = event.EventSequence - 1 }},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			event := valid
			test.mutate(&event)
			if err := event.Validate(); err == nil {
				t.Fatalf("Validate accepted invalid event: %#v", event)
			}
		})
	}
}
