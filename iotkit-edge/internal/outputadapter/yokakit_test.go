package outputadapter

import (
	"encoding/json"
	"errors"
	"testing"
)

func TestYokaKitTransformsGenericCumulativeValueToProductionContract(t *testing.T) {
	config, err := EncodeYokaKitConfig(YokaKitConfig{
		SourceID: "iotkit-01",
		SignalID: "press-count",
		Kind:     YokaKitProduction,
	})
	if err != nil {
		t.Fatal(err)
	}
	message, err := (YokaKitAdapter{}).Transform(config, Observation{
		ObservationID: "d36cb7b3-7010-43b3-afc6-1931ed705dea",
		SeriesID:      "a921df88-6af2-46ca-a5f1-f346bf4433bb",
		Sequence:      42,
		Kind:          KindCumulativeValue,
		Value:         json.RawMessage(`1524`),
		ObservedAt:    1784190000123,
	})
	if err != nil {
		t.Fatal(err)
	}
	if message.Topic != "yokakit/v1/sources/iotkit-01/signals/press-count/observations" ||
		message.QoS != 1 || message.Retain {
		t.Fatalf("message routing = %#v", message)
	}
	var payload map[string]any
	if err := json.Unmarshal(message.Payload, &payload); err != nil {
		t.Fatal(err)
	}
	if payload["kind"] != "production" || payload["value"] != float64(1524) ||
		len(payload) != 7 {
		t.Fatalf("payload = %#v", payload)
	}
}

func TestYokaKitRejectsIncompatibleMeaningAndUnsafeTopicIdentity(t *testing.T) {
	config, err := EncodeYokaKitConfig(YokaKitConfig{
		SourceID: "iotkit-01",
		SignalID: "press-running",
		Kind:     YokaKitProduction,
	})
	if err != nil {
		t.Fatal(err)
	}
	_, err = (YokaKitAdapter{}).Transform(config, Observation{
		ObservationID: "70f83542-9033-437a-925e-8d61fc147498",
		SeriesID:      "a0ec47fa-3abe-4230-bff5-a794906f8305",
		Sequence:      18,
		ObservedAt:    1784190000123,
		Kind:          KindBoolean,
		Value:         json.RawMessage(`true`),
	})
	if !errors.Is(err, ErrUnsupportedObservation) {
		t.Fatalf("error = %v", err)
	}
	unsafeConfig, err := EncodeYokaKitConfig(YokaKitConfig{
		SourceID: "../bad",
		SignalID: "x",
		Kind:     YokaKitOnOff,
	})
	if err != nil {
		t.Fatal(err)
	}
	if err := (YokaKitAdapter{}).ValidateConfig(
		unsafeConfig,
		KindBoolean,
	); !errors.Is(err, ErrInvalidConfiguration) {
		t.Fatal("unsafe source ID accepted")
	}
}

func TestYokaKitConfigurationIsVersionedAndClosed(t *testing.T) {
	adapter := YokaKitAdapter{}
	for _, config := range []json.RawMessage{
		json.RawMessage(`{"schema_version":2,"source_id":"line-a","signal_id":"count","kind":"production"}`),
		json.RawMessage(`{"schema_version":1,"source_id":"line-a","signal_id":"count","kind":"production","unknown":true}`),
		json.RawMessage(`{"schema_version":1,"source_id":"line-a","signal_id":"count","kind":"production"} trailing`),
	} {
		if err := adapter.ValidateConfig(
			config,
			KindCumulativeValue,
		); !errors.Is(err, ErrInvalidConfiguration) {
			t.Fatalf("config=%s error=%v", config, err)
		}
	}
}

func TestYokaKitConfigurationRejectsUnknownModeAndIrrelevantReason(t *testing.T) {
	adapter := YokaKitAdapter{}
	for _, config := range []json.RawMessage{
		json.RawMessage(`{"schema_version":1,"source_id":"line-a","signal_id":"count","kind":"unknown"}`),
		json.RawMessage(`{"schema_version":1,"source_id":"line-a","signal_id":"count","kind":"production","reason":"unused"}`),
	} {
		if err := adapter.ValidateConfig(
			config,
			KindCumulativeValue,
		); !errors.Is(err, ErrInvalidConfiguration) {
			t.Fatalf("config=%s error=%v", config, err)
		}
	}
}

func TestYokaKitProductionRejectsValueOutsideContractRange(t *testing.T) {
	config, err := EncodeYokaKitConfig(YokaKitConfig{
		SourceID: "line-a",
		SignalID: "count",
		Kind:     YokaKitProduction,
	})
	if err != nil {
		t.Fatal(err)
	}
	_, err = (YokaKitAdapter{}).Transform(config, Observation{
		ObservationID: "d36cb7b3-7010-43b3-afc6-1931ed705dea",
		SeriesID:      "a921df88-6af2-46ca-a5f1-f346bf4433bb",
		Sequence:      42,
		ObservedAt:    1784190000123,
		Kind:          KindCumulativeValue,
		Value:         json.RawMessage(`9007199254740992`),
	})
	if !errors.Is(err, ErrInvalidObservation) {
		t.Fatalf("error = %v", err)
	}
}

func TestYokaKitSourceStatusIsRetained(t *testing.T) {
	message, err := YokaKitStatus("iotkit-01", 1784190000123)
	if err != nil {
		t.Fatal(err)
	}
	if message.Topic != "yokakit/v1/sources/iotkit-01/status" ||
		!message.Retain || message.QoS != 1 {
		t.Fatalf("status = %#v", message)
	}
}
