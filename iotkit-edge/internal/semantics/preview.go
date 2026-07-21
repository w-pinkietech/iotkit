package semantics

import "errors"

type PreviewInput struct {
	ReceivedAt int64   `json:"received_at"`
	ObservedAt int64   `json:"observed_at,omitempty"`
	Value      float64 `json:"value"`
}

type PreviewPoint struct {
	ReceivedAt    int64   `json:"received_at"`
	Input         float64 `json:"input"`
	InputMin      float64 `json:"input_min"`
	InputMax      float64 `json:"input_max"`
	Calibrated    float64 `json:"calibrated"`
	CalibratedMin float64 `json:"calibrated_min"`
	CalibratedMax float64 `json:"calibrated_max"`
	Active        *bool   `json:"active,omitempty"`
	Counter       *int64  `json:"counter,omitempty"`
	SampleCount   int     `json:"sample_count"`
	ActiveSamples int     `json:"active_samples,omitempty"`
	Transitions   int     `json:"transitions,omitempty"`
	Increment     int64   `json:"increment,omitempty"`
}

type Preview struct {
	InputCount int            `json:"input_count"`
	PlotCount  int            `json:"plot_count"`
	Points     []PreviewPoint `json:"points"`
	TestResult *Result        `json:"test_result,omitempty"`
}

func BuildPreview(
	spec DefinitionSpec,
	inputs []PreviewInput,
	maxPoints int,
	testValue *float64,
) (Preview, error) {
	if maxPoints < 1 {
		return Preview{}, errors.New("semantic preview plot limit must be positive")
	}
	if err := spec.Validate(); err != nil {
		return Preview{}, err
	}

	points := make([]PreviewPoint, 0, len(inputs))
	state := State{}
	for _, input := range inputs {
		previous := state
		observedAt := input.ObservedAt
		if observedAt == 0 {
			observedAt = input.ReceivedAt
		}
		result, next, err := EvaluateAt(spec, state, input.Value, observedAt)
		if err != nil {
			return Preview{}, err
		}
		state = next
		point := PreviewPoint{
			ReceivedAt:    input.ReceivedAt,
			Input:         input.Value,
			InputMin:      input.Value,
			InputMax:      input.Value,
			Calibrated:    result.Calibrated,
			CalibratedMin: result.Calibrated,
			CalibratedMax: result.Calibrated,
			SampleCount:   1,
		}
		if spec.Kind != KindNumeric {
			active := state.Active
			point.Active = &active
			if active {
				point.ActiveSamples = 1
			}
			if previous.Initialized && previous.Active != state.Active {
				point.Transitions = 1
			}
		}
		if spec.Kind == KindCumulativeCounter {
			counter := state.Counter
			point.Counter = &counter
			point.Increment = state.Counter - previous.Counter
		}
		points = append(points, point)
	}
	if len(points) > maxPoints {
		points = summarizePreviewPoints(points, maxPoints)
	}

	preview := Preview{
		InputCount: len(inputs),
		PlotCount:  len(points),
		Points:     points,
	}
	if testValue != nil {
		result, _, err := Evaluate(spec, State{}, *testValue)
		if err != nil {
			return Preview{}, err
		}
		preview.TestResult = &result
	}
	return preview, nil
}

func summarizePreviewPoints(points []PreviewPoint, maxPoints int) []PreviewPoint {
	summarized := make([]PreviewPoint, 0, maxPoints)
	for bucket := 0; bucket < maxPoints; bucket++ {
		start := bucket * len(points) / maxPoints
		end := (bucket + 1) * len(points) / maxPoints
		if start == end {
			continue
		}
		point := points[end-1]
		point.InputMin = points[start].InputMin
		point.InputMax = points[start].InputMax
		point.CalibratedMin = points[start].CalibratedMin
		point.CalibratedMax = points[start].CalibratedMax
		point.SampleCount = 0
		point.ActiveSamples = 0
		point.Transitions = 0
		point.Increment = 0
		for _, source := range points[start:end] {
			point.InputMin = min(point.InputMin, source.InputMin)
			point.InputMax = max(point.InputMax, source.InputMax)
			point.CalibratedMin = min(point.CalibratedMin, source.CalibratedMin)
			point.CalibratedMax = max(point.CalibratedMax, source.CalibratedMax)
			point.SampleCount += source.SampleCount
			point.ActiveSamples += source.ActiveSamples
			point.Transitions += source.Transitions
			point.Increment += source.Increment
		}
		summarized = append(summarized, point)
	}
	return summarized
}
