package semantics

import (
	"encoding/json"
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

func TestEvaluateRuleUsesAlreadyCalibratedInput(t *testing.T) {
	result, state, err := EvaluateRule(
		RuleSpec{Kind: KindNumeric},
		State{},
		21.5,
		1000,
	)
	if err != nil {
		t.Fatal(err)
	}
	if !result.Emitted || result.Number == nil || *result.Number != 21.5 {
		t.Fatalf("numeric rule result = %#v", result)
	}
	if result.Calibrated != 21.5 || !state.Initialized {
		t.Fatalf("numeric rule state=%#v result=%#v", state, result)
	}
}

func TestCalibrationRejectsNonFiniteOutput(t *testing.T) {
	calibration := Calibration{Scale: math.MaxFloat64, Offset: 0}
	if _, err := calibration.Apply(math.MaxFloat64); err == nil {
		t.Fatal("overflowing calibration was accepted")
	}
}

func TestEvaluateBooleanThresholdUsesHysteresisAndEmitsChanges(t *testing.T) {
	spec := DefinitionSpec{
		Kind:  KindBoolean,
		Scale: 1,
		Detector: Detector{
			Mode:          DetectorHighActive,
			RiseThreshold: 100,
			FallThreshold: 90,
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
				Detector: Detector{
					Mode: DetectorBooleanHighActive,
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
		Detector: Detector{
			Mode:          DetectorLowActive,
			RiseThreshold: 5,
			FallThreshold: 5,
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
		{Kind: KindCumulativeCounter, Scale: 1},
		{Kind: KindCumulativeCounter, Scale: 1, Detector: Detector{Mode: DetectorBooleanHighActive}},
		{Kind: KindAlarm, Scale: 1, Detector: Detector{
			Mode: DetectorHighActive, RiseThreshold: 1, FallThreshold: 2,
		}},
		{Kind: KindBoolean, Scale: 1, Detector: Detector{
			Mode: DetectorHighActive, RiseDebounceMS: maxDebounceMS + 1,
		}},
	} {
		if err := spec.Validate(); err == nil {
			t.Fatalf("invalid spec was accepted: %#v", spec)
		}
	}
	if _, _, err := Evaluate(DefinitionSpec{Kind: KindNumeric, Scale: 1}, State{}, math.NaN()); err == nil {
		t.Fatal("NaN input was accepted")
	}
}

func TestEvaluateAtUsesIndependentRiseAndFallDebounce(t *testing.T) {
	spec := DefinitionSpec{
		Kind:  KindBoolean,
		Scale: 1,
		Detector: Detector{
			Mode:           DetectorHighActive,
			RiseThreshold:  10,
			FallThreshold:  4,
			RiseDebounceMS: 2_000,
			FallDebounceMS: 3_000,
		},
	}
	result, state, err := EvaluateAt(spec, State{}, 0, 1_000)
	if err != nil || !result.Emitted || state.Active {
		t.Fatalf("initial result=%#v state=%#v err=%v", result, state, err)
	}
	result, state, err = EvaluateAt(spec, state, 11, 2_000)
	if err != nil || result.Emitted || !state.Pending || !state.PendingActive {
		t.Fatalf("rise start result=%#v state=%#v err=%v", result, state, err)
	}
	result, state, err = EvaluateAt(spec, state, 9, 3_000)
	if err != nil || result.Emitted || state.Pending {
		t.Fatalf("cancelled rise result=%#v state=%#v err=%v", result, state, err)
	}
	_, state, _ = EvaluateAt(spec, state, 12, 4_000)
	result, state, err = EvaluateAt(spec, state, 12, 6_000)
	if err != nil || !result.Emitted || result.Boolean == nil || !*result.Boolean ||
		!state.Active || state.Pending {
		t.Fatalf("confirmed rise result=%#v state=%#v err=%v", result, state, err)
	}
	_, state, _ = EvaluateAt(spec, state, 3, 7_000)
	result, state, err = EvaluateAt(spec, state, 3, 9_999)
	if err != nil || result.Emitted || !state.Active || !state.Pending {
		t.Fatalf("early fall result=%#v state=%#v err=%v", result, state, err)
	}
	result, state, err = EvaluateAt(spec, state, 3, 10_000)
	if err != nil || !result.Emitted || result.Boolean == nil || *result.Boolean ||
		state.Active || state.Pending {
		t.Fatalf("confirmed fall result=%#v state=%#v err=%v", result, state, err)
	}
}

func TestEvaluateAtLowActiveCountsFallingTransition(t *testing.T) {
	spec := DefinitionSpec{
		Kind:  KindCumulativeCounter,
		Scale: 1,
		Detector: Detector{
			Mode:          DetectorLowActive,
			RiseThreshold: 8,
			FallThreshold: 2,
		},
		Trigger: TriggerTransition,
	}
	state := State{}
	for index, input := range []float64{10, 1, 1, 9, 0} {
		result, next, err := EvaluateAt(spec, state, input, int64(index*1000))
		if err != nil {
			t.Fatal(err)
		}
		state = next
		if index == 1 && (!result.Emitted || result.Integer == nil || *result.Integer != 1) {
			t.Fatalf("first falling transition = %#v", result)
		}
	}
	if state.Counter != 2 {
		t.Fatalf("counter = %d, want 2", state.Counter)
	}
}

func TestDefinitionSpecReadsLegacyThresholdContract(t *testing.T) {
	var high DefinitionSpec
	if err := json.Unmarshal([]byte(`{
		"kind":"boolean","scale":1,"offset":0,
		"condition":{"mode":"above","threshold":100,"hysteresis":10},
		"trigger":""
	}`), &high); err != nil {
		t.Fatal(err)
	}
	if high.Detector.Mode != DetectorHighActive ||
		high.Detector.RiseThreshold != 100 ||
		high.Detector.FallThreshold != 90 {
		t.Fatalf("legacy high detector = %#v", high.Detector)
	}

	var low DefinitionSpec
	if err := json.Unmarshal([]byte(`{
		"kind":"cumulative_counter","scale":1,"offset":0,
		"condition":{"mode":"boolean_equals","bool_value":false},
		"trigger":"on_transition"
	}`), &low); err != nil {
		t.Fatal(err)
	}
	if low.Detector.Mode != DetectorBooleanLowActive {
		t.Fatalf("legacy boolean detector = %#v", low.Detector)
	}
}
