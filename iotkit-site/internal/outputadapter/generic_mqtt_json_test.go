package outputadapter

import (
	"encoding/json"
	"errors"
	"testing"
)

func TestGenericMQTTJSONTransformsObservationWithoutChangingMeaning(t *testing.T) {
	config, err := EncodeGenericMQTTJSONConfig(GenericMQTTJSONConfig{
		Topic: "factory/line-a/production",
	})
	if err != nil {
		t.Fatal(err)
	}
	publication, err := (GenericMQTTJSONAdapter{}).Transform(config, Observation{
		ObservationID: "d36cb7b3-7010-43b3-afc6-1931ed705dea",
		SeriesID:      "a921df88-6af2-46ca-a5f1-f346bf4433bb",
		Sequence:      42,
		ObservedAt:    1784190000123,
		Kind:          KindCumulativeValue,
		Value:         json.RawMessage(`1524`),
	})
	if err != nil {
		t.Fatal(err)
	}
	if publication.Topic != "factory/line-a/production" ||
		publication.QoS != 1 || publication.Retain {
		t.Fatalf("publication routing = %#v", publication)
	}
	const want = `{"schema_version":1,"observation_id":"d36cb7b3-7010-43b3-afc6-1931ed705dea","series_id":"a921df88-6af2-46ca-a5f1-f346bf4433bb","sequence":42,"observed_at":1784190000123,"kind":"cumulative_value","value":1524}`
	if string(publication.Payload) != want {
		t.Fatalf("payload = %s\nwant    = %s", publication.Payload, want)
	}
}

func TestGenericMQTTJSONAcceptsEveryGenericObservationKind(t *testing.T) {
	descriptor := (GenericMQTTJSONAdapter{}).Descriptor()
	if err := descriptor.Validate(); err != nil {
		t.Fatal(err)
	}
	for _, kind := range []ObservationKind{
		KindNumeric,
		KindBoolean,
		KindCumulativeValue,
		KindAlarm,
	} {
		assertModeAccepts(t, descriptor, "observation", kind)
	}
}

func TestGenericMQTTJSONConfigurationIsVersionedClosedAndExact(t *testing.T) {
	adapter := GenericMQTTJSONAdapter{}
	for _, config := range []json.RawMessage{
		json.RawMessage(`{"schema_version":2,"topic":"factory/line-a/value"}`),
		json.RawMessage(`{"schema_version":1,"topic":"factory/line-a/value","unknown":true}`),
		json.RawMessage(`{"schema_version":1,"topic":"factory/+/value"}`),
		json.RawMessage(`{"schema_version":1,"topic":""}`),
	} {
		if err := adapter.ValidateConfig(
			config,
			KindNumeric,
		); !errors.Is(err, ErrInvalidConfiguration) {
			t.Fatalf("config=%s error=%v", config, err)
		}
	}
}
