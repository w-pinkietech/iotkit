package outputadapter

import (
	"encoding/json"
	"errors"
	"testing"
)

func TestAdapterDescriptorDefinesStableModes(t *testing.T) {
	descriptor := (YokaKitAdapter{}).Descriptor()
	if err := descriptor.Validate(); err != nil {
		t.Fatal(err)
	}
	if descriptor.ID != "yokakit.mqtt.v1" ||
		descriptor.ConfigSchemaVersion != 1 ||
		descriptor.DisplayName != "YokaKit MQTT v1" {
		t.Fatalf("descriptor = %#v", descriptor)
	}
	if len(descriptor.Modes) != 4 {
		t.Fatalf("modes = %#v", descriptor.Modes)
	}
	assertModeAccepts(t, descriptor, "production", KindCumulativeValue)
	assertModeAccepts(t, descriptor, "onoff", KindBoolean)
	assertModeAccepts(t, descriptor, "gantt_chart", KindBoolean)
	assertModeAccepts(t, descriptor, "alarm", KindAlarm)
}

func TestAdapterDescriptorRejectsAmbiguousModes(t *testing.T) {
	descriptor := Descriptor{
		ID:                  "example.mqtt.v1",
		DisplayName:         "Example",
		ConfigSchemaVersion: 1,
		Modes: []Mode{
			{Key: "state", DisplayName: "State", Accepts: []ObservationKind{KindBoolean}},
			{Key: "state", DisplayName: "Duplicate", Accepts: []ObservationKind{KindAlarm}},
		},
	}
	if err := descriptor.Validate(); !errors.Is(err, ErrInvalidDescriptor) {
		t.Fatalf("error = %v", err)
	}
}

func TestObservationContractUsesGenericCumulativeValueVocabulary(t *testing.T) {
	observation := Observation{
		ObservationID: "d36cb7b3-7010-43b3-afc6-1931ed705dea",
		SeriesID:      "a921df88-6af2-46ca-a5f1-f346bf4433bb",
		Sequence:      42,
		ObservedAt:    1784190000123,
		Kind:          KindCumulativeValue,
		Value:         json.RawMessage(`1524`),
	}
	if err := observation.Validate(); err != nil {
		t.Fatal(err)
	}
	if string(observation.Kind) != "cumulative_value" {
		t.Fatalf("kind = %q", observation.Kind)
	}
}

func TestMQTTPublicationRejectsWildcardTopicAndInvalidJSON(t *testing.T) {
	for _, publication := range []MQTTPublication{
		{Topic: "factory/+/state", QoS: 1, Payload: json.RawMessage(`{}`)},
		{Topic: "factory/state", QoS: 1, Payload: json.RawMessage(`not-json`)},
		{Topic: "factory/state", QoS: 0, Payload: json.RawMessage(`{}`)},
	} {
		if err := publication.Validate(); !errors.Is(err, ErrInvalidPublication) {
			t.Fatalf("publication=%#v error=%v", publication, err)
		}
	}
}

func TestBuiltInRegistryResolvesStableAdapterIDs(t *testing.T) {
	registry, err := BuiltInRegistry()
	if err != nil {
		t.Fatal(err)
	}
	for _, id := range []string{
		"iotkit.mqtt-json.v1",
		"yokakit.mqtt.v1",
	} {
		adapter, ok := registry.Resolve(id)
		if !ok || adapter.Descriptor().ID != id {
			t.Fatalf("id=%q adapter=%#v ok=%t", id, adapter, ok)
		}
	}
	descriptors := registry.Descriptors()
	if len(descriptors) != 2 ||
		descriptors[0].ID != "iotkit.mqtt-json.v1" ||
		descriptors[1].ID != "yokakit.mqtt.v1" {
		t.Fatalf("descriptors = %#v", descriptors)
	}
}

func assertModeAccepts(
	t *testing.T,
	descriptor Descriptor,
	key string,
	kind ObservationKind,
) {
	t.Helper()
	for _, mode := range descriptor.Modes {
		if mode.Key != key {
			continue
		}
		for _, accepted := range mode.Accepts {
			if accepted == kind {
				return
			}
		}
		t.Fatalf("mode %q does not accept %q: %#v", key, kind, mode)
	}
	t.Fatalf("mode %q not found in %#v", key, descriptor.Modes)
}
