package semantics

import "testing"

func TestBuildPreviewKeepsNumericSpikesWithinPlotLimit(t *testing.T) {
	inputs := make([]PreviewInput, 1_000)
	for index := range inputs {
		inputs[index] = PreviewInput{
			ReceivedAt: int64(index + 1),
			Value:      float64(index % 10),
		}
	}
	inputs[517].Value = 1_000

	preview, err := BuildPreview(
		DefinitionSpec{Kind: KindNumeric, Scale: 2, Offset: 1},
		inputs,
		300,
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if preview.InputCount != 1_000 || len(preview.Points) > 300 {
		t.Fatalf("preview counts = input %d points %d", preview.InputCount, len(preview.Points))
	}
	foundSpike := false
	for _, point := range preview.Points {
		if point.InputMax == 1_000 && point.CalibratedMax == 2_001 {
			foundSpike = true
		}
	}
	if !foundSpike {
		t.Fatalf("numeric spike was lost: %#v", preview.Points)
	}
}

func TestBuildPreviewEvaluatesCumulativeInputBeforeSummarizing(t *testing.T) {
	inputs := make([]PreviewInput, 1_000)
	for index := range inputs {
		inputs[index] = PreviewInput{
			ReceivedAt: int64(index + 1),
			Value:      float64(index % 2),
		}
	}
	spec := DefinitionSpec{
		Kind:  KindCumulativeCounter,
		Scale: 1,
		Detector: Detector{
			Mode: DetectorBooleanHighActive,
		},
		Trigger: TriggerTransition,
	}

	preview, err := BuildPreview(spec, inputs, 300, nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(preview.Points) > 300 {
		t.Fatalf("points = %d, want at most 300", len(preview.Points))
	}
	last := preview.Points[len(preview.Points)-1]
	if last.Counter == nil || *last.Counter != 500 {
		t.Fatalf("last point = %#v, want cumulative count 500", last)
	}
	var increments int64
	for _, point := range preview.Points {
		increments += point.Increment
	}
	if increments != 500 {
		t.Fatalf("summarized increments = %d, want 500", increments)
	}
}

func TestBuildPreviewReturnsSinglePointAndIndependentTestValue(t *testing.T) {
	testValue := 12.5
	preview, err := BuildPreview(
		DefinitionSpec{Kind: KindNumeric, Scale: 2, Offset: -1},
		[]PreviewInput{{ReceivedAt: 123, Value: 4}},
		300,
		&testValue,
	)
	if err != nil {
		t.Fatal(err)
	}
	if len(preview.Points) != 1 || preview.Points[0].Calibrated != 7 {
		t.Fatalf("points = %#v", preview.Points)
	}
	if preview.TestResult == nil || preview.TestResult.Calibrated != 24 {
		t.Fatalf("test result = %#v", preview.TestResult)
	}
}

func TestBuildPreviewRejectsInvalidPlotLimit(t *testing.T) {
	if _, err := BuildPreview(
		DefinitionSpec{Kind: KindNumeric, Scale: 1},
		nil,
		0,
		nil,
	); err == nil {
		t.Fatal("zero plot limit was accepted")
	}
}
