package store

import (
	"encoding/json"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
)

func TestOutputObservationMapsInternalCounterToGenericCumulativeValue(t *testing.T) {
	observation, err := outputObservation(semantics.Observation{
		ObservationID: "d36cb7b3-7010-43b3-afc6-1931ed705dea",
		SeriesID:      "a921df88-6af2-46ca-a5f1-f346bf4433bb",
		Sequence:      42,
		ObservedAt:    1784190000123,
		Kind:          semantics.KindCumulativeCounter,
		Value:         json.RawMessage(`1524`),
	})
	if err != nil {
		t.Fatal(err)
	}
	if observation.Kind != outputadapter.KindCumulativeValue {
		t.Fatalf("kind = %q", observation.Kind)
	}
}
