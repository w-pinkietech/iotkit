package semantics

import (
	"math"
	"testing"
)

func TestEvaluateNumericAppliesScaleAndOffset(t *testing.T) {
	spec := DefinitionSpec{
		Kind:   KindNumeric,
		Scale:  1.8,
		Offset: 32,
	}
	result, state, err := Evaluate(spec, State{}, 10)
	if err != nil {
		t.Fatal(err)
	}
	if !result.Emitted || result.Number == nil || *result.Number != 50 {
		t.Fatalf("numeric result = %#v", result)
	}
	if !state.Initialized {
		t.Fatalf("numeric state = %#v", state)
	}
}

func TestEvaluateBooleanThresholdUsesHysteresisAndEmitsChanges(t *testing.T) {
	spec := DefinitionSpec{
		Kind:  KindBoolean,
		Scale: 1,
		Condition: Condition{
			Mode:       ConditionAbove,
			Threshold:  100,
			Hysteresis: 10,
		},
	}
	result, state, err := Evaluate(spec, State{}, 95)
	if err != nil {
		t.Fatal(err)
	}
	if !result.Emitted || result.Boolean == nil || *result.Boolean {
		t.Fatalf("initial result = %#v", result)
	}
	result, state, err = Evaluate(spec, state, 105)
	if err != nil {
		t.Fatal(err)
	}
	if !result.Emitted || result.Boolean == nil || !*result.Boolean {
		t.Fatalf("activation result = %#v", result)
	}
	result, state, err = Evaluate(spec, state, 95)
	if err != nil {
		t.Fatal(err)
	}
	if result.Emitted {
		t.Fatalf("hysteresis band emitted a change: %#v", result)
	}
	result, _, err = Evaluate(spec, state, 89)
	if err != nil {
		t.Fatal(err)
	}
	if !result.Emitted || result.Boolean == nil || *result.Boolean {
		t.Fatalf("clear result = %#v", result)
	}
}

func TestEvaluateCumulativeCounterSupportsTransitionAndActiveSample(t *testing.T) {
	for _, test := range []struct {
		name       string
		trigger    TriggerMode
		inputs     []float64
		wantValues []int64
	}{
		{
			name:       "transition",
			trigger:    TriggerTransition,
			inputs:     []float64{0, 1, 1, 0, 1},
			wantValues: []int64{1, 2},
		},
		{
			name:       "active sample",
			trigger:    TriggerActiveSample,
			inputs:     []float64{0, 1, 1, 0, 1},
			wantValues: []int64{1, 2, 3},
		},
	} {
		t.Run(test.name, func(t *testing.T) {
			spec := DefinitionSpec{
				Kind:  KindCumulativeCounter,
				Scale: 1,
				Condition: Condition{
					Mode:      ConditionBoolean,
					BoolValue: true,
				},
				Trigger: test.trigger,
			}
			state := State{}
			got := make([]int64, 0)
			for _, input := range test.inputs {
				result, next, err := Evaluate(spec, state, input)
				if err != nil {
					t.Fatal(err)
				}
				state = next
				if result.Emitted {
					if result.Integer == nil {
						t.Fatalf("counter result has no integer: %#v", result)
					}
					got = append(got, *result.Integer)
				}
			}
			if len(got) != len(test.wantValues) {
				t.Fatalf("counter values = %#v, want %#v", got, test.wantValues)
			}
			for index := range got {
				if got[index] != test.wantValues[index] {
					t.Fatalf("counter values = %#v, want %#v", got, test.wantValues)
				}
			}
		})
	}
}

func TestEvaluateAlarmEmitsInitialStateAndTransitions(t *testing.T) {
	spec := DefinitionSpec{
		Kind:  KindAlarm,
		Scale: 1,
		Condition: Condition{
			Mode:      ConditionBelow,
			Threshold: 5,
		},
	}
	state := State{}
	for index, test := range []struct {
		input       float64
		wantEmitted bool
		wantActive  bool
	}{
		{input: 8, wantEmitted: true, wantActive: false},
		{input: 7, wantEmitted: false},
		{input: 4, wantEmitted: true, wantActive: true},
		{input: 3, wantEmitted: false},
		{input: 6, wantEmitted: true, wantActive: false},
	} {
		result, next, err := Evaluate(spec, state, test.input)
		if err != nil {
			t.Fatalf("step %d: %v", index, err)
		}
		state = next
		if result.Emitted != test.wantEmitted {
			t.Fatalf("step %d emitted = %v, want %v", index, result.Emitted, test.wantEmitted)
		}
		if test.wantEmitted && (result.Boolean == nil || *result.Boolean != test.wantActive) {
			t.Fatalf("step %d result = %#v", index, result)
		}
	}
}

func TestDefinitionValidationRejectsInvalidCombinationsAndNonFiniteInput(t *testing.T) {
	for _, spec := range []DefinitionSpec{
		{Kind: KindNumeric, Scale: 0},
		{Kind: KindCumulativeCounter, Scale: 1, Condition: Condition{Mode: ConditionNone}},
		{Kind: KindCumulativeCounter, Scale: 1, Condition: Condition{Mode: ConditionBoolean}},
		{Kind: KindAlarm, Scale: 1, Condition: Condition{Mode: ConditionAbove, Hysteresis: -1}},
	} {
		if err := spec.Validate(); err == nil {
			t.Fatalf("invalid spec was accepted: %#v", spec)
		}
	}
	if _, _, err := Evaluate(DefinitionSpec{Kind: KindNumeric, Scale: 1}, State{}, math.NaN()); err == nil {
		t.Fatal("NaN input was accepted")
	}
}
