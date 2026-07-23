package outputadapter

import (
	"encoding/json"
	"errors"
	"testing"
)

func TestPinikietTransformsGenericCumulativeValueToProductionContract(t *testing.T) {
	config, err := EncodePinikietConfig(PinikietConfig{
		SourceID: "iotkit-01",
		SensorID: "press-sensor",
		Kind:     PinikietProduction,
	})
	if err != nil {
		t.Fatal(err)
	}
	message, err := (PinikietAdapter{}).Transform(config, Observation{
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
	if message.Topic != "pinikiet/v1/sources/iotkit-01/sensors/press-sensor/observations" ||
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

func TestPinikietRejectsIncompatibleMeaningAndUnsafeTopicIdentity(t *testing.T) {
	config, err := EncodePinikietConfig(PinikietConfig{
		SourceID: "iotkit-01",
		SensorID: "press-sensor",
		Kind:     PinikietProduction,
	})
	if err != nil {
		t.Fatal(err)
	}
	_, err = (PinikietAdapter{}).Transform(config, Observation{
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
	unsafeConfig, err := EncodePinikietConfig(PinikietConfig{
		SourceID: "../bad",
		SensorID: "x",
		Kind:     PinikietOnOff,
	})
	if err != nil {
		t.Fatal(err)
	}
	if err := (PinikietAdapter{}).ValidateConfig(
		unsafeConfig,
		KindBoolean,
	); !errors.Is(err, ErrInvalidConfiguration) {
		t.Fatal("unsafe source ID accepted")
	}
}

func TestPinikietConfigurationIsVersionedAndClosed(t *testing.T) {
	adapter := PinikietAdapter{}
	for _, config := range []json.RawMessage{
		json.RawMessage(`{"schema_version":2,"source_id":"line-a","sensor_id":"press","kind":"production"}`),
		json.RawMessage(`{"schema_version":1,"source_id":"line-a","sensor_id":"press","kind":"production","unknown":true}`),
		json.RawMessage(`{"schema_version":1,"source_id":"line-a","sensor_id":"press","kind":"production"} trailing`),
		json.RawMessage(`{"schema_version":1,"source_id":"line-a","signal_id":"press","kind":"production"}`),
	} {
		if err := adapter.ValidateConfig(
			config,
			KindCumulativeValue,
		); !errors.Is(err, ErrInvalidConfiguration) {
			t.Fatalf("config=%s error=%v", config, err)
		}
	}
}

func TestPinikietConfigurationRejectsUnknownModeAndIrrelevantReason(t *testing.T) {
	adapter := PinikietAdapter{}
	for _, config := range []json.RawMessage{
		json.RawMessage(`{"schema_version":1,"source_id":"line-a","sensor_id":"press","kind":"unknown"}`),
		json.RawMessage(`{"schema_version":1,"source_id":"line-a","sensor_id":"press","kind":"production","reason":"unused"}`),
	} {
		if err := adapter.ValidateConfig(
			config,
			KindCumulativeValue,
		); !errors.Is(err, ErrInvalidConfiguration) {
			t.Fatalf("config=%s error=%v", config, err)
		}
	}
}

func TestPinikietProductionRejectsValueOutsideContractRange(t *testing.T) {
	config, err := EncodePinikietConfig(PinikietConfig{
		SourceID: "line-a",
		SensorID: "press",
		Kind:     PinikietProduction,
	})
	if err != nil {
		t.Fatal(err)
	}
	_, err = (PinikietAdapter{}).Transform(config, Observation{
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

func TestPinikietSourceStatusIsRetained(t *testing.T) {
	message, err := PinikietStatus("iotkit-01", 1784190000123)
	if err != nil {
		t.Fatal(err)
	}
	if message.Topic != "pinikiet/v1/sources/iotkit-01/status" ||
		!message.Retain || message.QoS != 1 {
		t.Fatalf("status = %#v", message)
	}
}
