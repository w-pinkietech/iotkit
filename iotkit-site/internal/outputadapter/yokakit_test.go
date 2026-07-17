package outputadapter

import (
	"encoding/json"
	"errors"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
)

func TestYokaKitTransformsGenericCumulativeValueToProductionContract(t *testing.T) {
	adapter := YokaKit{
		SourceID: "iotkit-01",
		SignalID: "press-count",
		Kind:     YokaKitProduction,
	}
	message, err := adapter.Transform(semantics.Observation{
		ObservationID: "d36cb7b3-7010-43b3-afc6-1931ed705dea",
		SeriesID:      "a921df88-6af2-46ca-a5f1-f346bf4433bb",
		Sequence:      42,
		Kind:          semantics.KindCumulativeCounter,
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
	_, err := (YokaKit{
		SourceID: "iotkit-01",
		SignalID: "press-running",
		Kind:     YokaKitProduction,
	}).Transform(semantics.Observation{
		Kind:  semantics.KindBoolean,
		Value: json.RawMessage(`true`),
	})
	if !errors.Is(err, ErrUnsupportedObservation) {
		t.Fatalf("error = %v", err)
	}
	if err := (YokaKit{SourceID: "../bad", SignalID: "x", Kind: YokaKitOnOff}).Validate(); err == nil {
		t.Fatal("unsafe source ID accepted")
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
