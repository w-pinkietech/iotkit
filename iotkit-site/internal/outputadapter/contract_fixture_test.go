package outputadapter

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

func TestBuiltInAdaptersMatchSharedContractFixtures(t *testing.T) {
	registry, err := BuiltInRegistry()
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{
		"iotkit-cumulative-value.json",
		"yokakit-production.json",
	} {
		t.Run(name, func(t *testing.T) {
			encoded, err := os.ReadFile(filepath.Join(
				"..", "..", "..", "testdata", "output", "v1", name,
			))
			if err != nil {
				t.Fatal(err)
			}
			var fixture struct {
				AdapterID   string          `json:"adapter_id"`
				Config      json.RawMessage `json:"config"`
				Observation struct {
					ObservationID string          `json:"observation_id"`
					SeriesID      string          `json:"series_id"`
					Sequence      int64           `json:"sequence"`
					ObservedAt    int64           `json:"observed_at"`
					Kind          ObservationKind `json:"kind"`
					Value         json.RawMessage `json:"value"`
					Reading       *float64        `json:"reading,omitempty"`
				} `json:"observation"`
				Publication struct {
					Topic   string          `json:"topic"`
					QoS     byte            `json:"qos"`
					Retain  bool            `json:"retain"`
					Payload json.RawMessage `json:"payload"`
				} `json:"publication"`
			}
			if err := json.Unmarshal(encoded, &fixture); err != nil {
				t.Fatal(err)
			}
			adapter, found := registry.Resolve(fixture.AdapterID)
			if !found {
				t.Fatalf("fixture names unknown adapter %q", fixture.AdapterID)
			}
			publication, err := adapter.Transform(fixture.Config, Observation{
				ObservationID: fixture.Observation.ObservationID,
				SeriesID:      fixture.Observation.SeriesID,
				Sequence:      fixture.Observation.Sequence,
				ObservedAt:    fixture.Observation.ObservedAt,
				Kind:          fixture.Observation.Kind,
				Value:         fixture.Observation.Value,
				Reading:       fixture.Observation.Reading,
			})
			if err != nil {
				t.Fatal(err)
			}
			if publication.Topic != fixture.Publication.Topic ||
				publication.QoS != fixture.Publication.QoS ||
				publication.Retain != fixture.Publication.Retain ||
				!equalJSON(publication.Payload, fixture.Publication.Payload) {
				t.Fatalf("publication = %#v payload=%s", publication, publication.Payload)
			}
		})
	}
}

func equalJSON(left, right []byte) bool {
	var decodedLeft, decodedRight any
	if json.Unmarshal(left, &decodedLeft) != nil ||
		json.Unmarshal(right, &decodedRight) != nil {
		return false
	}
	return reflect.DeepEqual(decodedLeft, decodedRight)
}
