package semantic

import (
	"reflect"
	"testing"
)

func TestEvaluateModes(t *testing.T) {
	if got := evaluateSequence(t, TriggerActiveSample, 1, []int{1, 1, 0, 1}); !reflect.DeepEqual(got, []bool{true, true, false, true}) {
		t.Fatalf("active_sample = %#v", got)
	}
	if got := evaluateSequence(t, TriggerActiveEdge, 1, []int{1, 1, 0, 1}); !reflect.DeepEqual(got, []bool{false, false, false, true}) {
		t.Fatalf("active_edge = %#v", got)
	}
}

func evaluateSequence(t *testing.T, mode TriggerMode, activeValue int, values []int) []bool {
	t.Helper()
	emitted := make([]bool, 0, len(values))
	var previous *int
	for _, current := range values {
		emit, next, err := Evaluate(mode, activeValue, previous, current)
		if err != nil {
			t.Fatal(err)
		}
		emitted = append(emitted, emit)
		previous = &next
	}
	return emitted
}
