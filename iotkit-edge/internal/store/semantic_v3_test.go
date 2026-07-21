package store

import (
	"context"
	"encoding/json"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
)

func semanticV3Signal(t *testing.T, archive *Store) string {
	t.Helper()
	acceptSemanticBatch(t, archive, "edge-node-01", "epoch-a", 1, []float64{0})
	return reconcileSingleSignal(t, archive)
}

func TestSemanticV3SignalStartsWithIdentityCalibration(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)

	configuration, err := archive.GetSemanticConfiguration(
		context.Background(),
		signalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	if configuration.Revision != 1 ||
		configuration.Calibration.Revision != 1 ||
		configuration.Calibration.Scale != 1 ||
		configuration.Calibration.Offset != 0 ||
		len(configuration.Rules) != 0 {
		t.Fatalf("configuration = %#v", configuration)
	}
}

func TestSemanticV3CreatesTwoIndependentRulesAndStartsNewSeriesWhenMeaningChanges(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, err := archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}

	counter, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		"生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	configuration, err = archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	alarm, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		"停止アラーム",
		semantics.RuleSpec{
			Kind: semantics.KindAlarm,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanLowActive,
			},
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	if counter.ID == alarm.ID || counter.SeriesID == alarm.SeriesID {
		t.Fatalf("rules share identity: counter=%#v alarm=%#v", counter, alarm)
	}

	updated, err := archive.UpdateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		counter.ID,
		"良品回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerActiveSample,
		},
		edgeapp.RevisionPrecondition{Expected: &counter.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	if updated.ID != counter.ID ||
		updated.SeriesID == counter.SeriesID ||
		updated.Revision != 2 ||
		updated.Kind != counter.Kind {
		t.Fatalf("updated rule series generation is wrong: before=%#v after=%#v", counter, updated)
	}

	configuration, err = archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	if len(configuration.Rules) != 2 {
		t.Fatalf("active rules = %#v", configuration.Rules)
	}
}

func TestSemanticV3DisplayNameOnlyUpdateKeepsSeries(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, err := archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	rule, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		"室温",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	updated, err := archive.UpdateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		rule.ID,
		"炉内温度",
		rule.RuleSpec,
		edgeapp.RevisionPrecondition{Expected: &rule.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	if updated.SeriesID != rule.SeriesID {
		t.Fatalf("display name update changed series: before=%#v after=%#v", rule, updated)
	}
}

func TestSemanticV3MeaningChangeRestartsSequenceInNewSeries(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, err := archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	rule, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		"運転状態",
		semantics.RuleSpec{
			Kind: semantics.KindBoolean,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 2, []float64{1},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 10); err != nil {
		t.Fatal(err)
	}
	updated, err := archive.UpdateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		rule.ID,
		rule.DisplayName,
		semantics.RuleSpec{
			Kind: semantics.KindBoolean,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanLowActive,
			},
		},
		edgeapp.RevisionPrecondition{Expected: &rule.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 3, []float64{0},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 10); err != nil {
		t.Fatal(err)
	}
	rows, err := archive.db.Query(`
		SELECT series_id, sequence
		FROM semantic_observations_v3
		WHERE rule_id = ? ORDER BY observation_row_id
	`, rule.ID)
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	var generations []struct {
		seriesID string
		sequence int64
	}
	for rows.Next() {
		var generation struct {
			seriesID string
			sequence int64
		}
		if err := rows.Scan(&generation.seriesID, &generation.sequence); err != nil {
			t.Fatal(err)
		}
		generations = append(generations, generation)
	}
	if len(generations) != 2 ||
		generations[0].seriesID == generations[1].seriesID ||
		generations[0].sequence != 1 ||
		generations[1].sequence != 1 ||
		updated.SeriesID != generations[1].seriesID {
		t.Fatalf("generations=%#v updated=%#v", generations, updated)
	}
}

func TestSemanticV3CalibrationAndRetirementAreFutureOnly(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, err := archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	rule, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		"温度",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	configuration, err = archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	updated, err := archive.UpdateSignalCalibration(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		2,
		1,
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	if updated.Calibration.Revision != 2 ||
		updated.Calibration.Scale != 2 ||
		updated.Calibration.Offset != 1 {
		t.Fatalf("updated calibration = %#v", updated.Calibration)
	}
	if len(updated.Rules) != 1 ||
		updated.Rules[0].SeriesID == rule.SeriesID ||
		updated.Rules[0].Revision != rule.Revision+1 {
		t.Fatalf("calibration did not start a new rule series: %#v", updated.Rules)
	}
	var calibrationBoundary int64
	if err := archive.db.QueryRow(`
		SELECT start_after_pub_seq
		FROM signal_calibration_starts_v3
		WHERE signal_ref = ? AND calibration_revision = 2
			AND ledger_epoch = 'epoch-a'
	`, signalRef).Scan(&calibrationBoundary); err != nil {
		t.Fatal(err)
	}
	if calibrationBoundary != 1 {
		t.Fatalf("calibration boundary = %d, want 1", calibrationBoundary)
	}

	retired, err := archive.RetireSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		rule.ID,
		edgeapp.RevisionPrecondition{Expected: &updated.Rules[0].Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	if retired.Active || retired.RetiredAt == nil {
		t.Fatalf("retired rule = %#v", retired)
	}
	var ruleBoundary int64
	if err := archive.db.QueryRow(`
		SELECT end_at_pub_seq
		FROM semantic_rule_ends_v3
		WHERE rule_id = ? AND rule_revision = 1
			AND ledger_epoch = 'epoch-a'
	`, rule.ID).Scan(&ruleBoundary); err != nil {
		t.Fatal(err)
	}
	if ruleBoundary != 1 {
		t.Fatalf("rule boundary = %d, want 1", ruleBoundary)
	}
}

func TestSemanticV3RejectsKindChangeAndSeventeenthActiveRule(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, err := archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	var first semantics.Rule
	for index := 0; index < maxActiveSemanticRulesV3; index++ {
		rule, createErr := archive.CreateSemanticRule(
			ctx,
			edgeapp.LocalCLIActor(),
			signalRef,
			"数値"+string(rune('A'+index)),
			semantics.RuleSpec{Kind: semantics.KindNumeric},
			edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
		)
		if createErr != nil {
			t.Fatal(createErr)
		}
		if index == 0 {
			first = rule
		}
		configuration, err = archive.GetSemanticConfiguration(ctx, signalRef)
		if err != nil {
			t.Fatal(err)
		}
	}
	if _, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		"多すぎるルール",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	); err == nil {
		t.Fatal("seventeenth active rule was accepted")
	}
	if _, err := archive.UpdateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		first.ID,
		first.DisplayName,
		semantics.RuleSpec{
			Kind: semantics.KindAlarm,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
		},
		edgeapp.RevisionPrecondition{Expected: &first.Revision},
	); err == nil {
		t.Fatal("semantic rule kind change was accepted")
	}
}

func TestSemanticV3ProjectionRunsCounterAndAlarmIndependently(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, err := archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	counter, err := archive.CreateSemanticRule(
		ctx, edgeapp.LocalCLIActor(), signalRef, "生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	configuration, _ = archive.GetSemanticConfiguration(ctx, signalRef)
	alarm, err := archive.CreateSemanticRule(
		ctx, edgeapp.LocalCLIActor(), signalRef, "停止アラーム",
		semantics.RuleSpec{
			Kind: semantics.KindAlarm,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanLowActive,
			},
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 2,
		[]float64{0}, []float64{1},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	observations, err := archive.ListSemanticRuleObservations(ctx, 100)
	if err != nil {
		t.Fatal(err)
	}
	var counterValues []int64
	var alarmValues []bool
	for _, observation := range observations {
		switch observation.RuleID {
		case counter.ID:
			var value int64
			if err := json.Unmarshal(observation.Value, &value); err != nil {
				t.Fatal(err)
			}
			counterValues = append(counterValues, value)
		case alarm.ID:
			var value bool
			if err := json.Unmarshal(observation.Value, &value); err != nil {
				t.Fatal(err)
			}
			alarmValues = append(alarmValues, value)
		}
	}
	if len(counterValues) != 1 || counterValues[0] != 1 {
		t.Fatalf("counter observations = %#v", counterValues)
	}
	if len(alarmValues) != 2 || !alarmValues[0] || alarmValues[1] {
		t.Fatalf("alarm observations = %#v", alarmValues)
	}
}

func TestSemanticV3ProjectionFailureDoesNotBlockAnotherRule(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, _ := archive.GetSemanticConfiguration(ctx, signalRef)
	if _, err := archive.CreateSemanticRule(
		ctx, edgeapp.LocalCLIActor(), signalRef, "壊れる状態",
		semantics.RuleSpec{
			Kind: semantics.KindBoolean,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	); err != nil {
		t.Fatal(err)
	}
	configuration, _ = archive.GetSemanticConfiguration(ctx, signalRef)
	numeric, err := archive.CreateSemanticRule(
		ctx, edgeapp.LocalCLIActor(), signalRef, "生値",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(t, archive, "edge-node-01", "epoch-a", 2, []float64{2})
	if _, err := archive.ProjectSemanticRules(ctx, 100); err == nil {
		t.Fatal("poison rule projection did not report an error")
	}
	observations, err := archive.ListSemanticRuleObservations(ctx, 100)
	if err != nil {
		t.Fatal(err)
	}
	if len(observations) != 1 || observations[0].RuleID != numeric.ID {
		t.Fatalf("independent observations = %#v", observations)
	}
	failures, err := archive.SemanticRuleProjectionFailureCount(ctx)
	if err != nil || failures != 1 {
		t.Fatalf("failure count = %d, err=%v", failures, err)
	}
}

func TestSemanticV3CounterResetWaitsForAcceptedBoundaryAndIsIdempotent(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, _ := archive.GetSemanticConfiguration(ctx, signalRef)
	counter, err := archive.CreateSemanticRule(
		ctx, edgeapp.LocalCLIActor(), signalRef, "生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 2,
		[]float64{0}, []float64{1},
	)
	reset, err := archive.RequestSemanticCounterReset(
		ctx, edgeapp.LocalCLIActor(), counter.ID, "reset-once",
	)
	if err != nil {
		t.Fatal(err)
	}
	if reset.ApplyAfterPubSeq != 3 || reset.AppliedAt != nil {
		t.Fatalf("pending reset = %#v", reset)
	}
	again, err := archive.RequestSemanticCounterReset(
		ctx, edgeapp.LocalCLIActor(), counter.ID, "reset-once",
	)
	if err != nil || again.ID != reset.ID {
		t.Fatalf("idempotent reset = %#v err=%v", again, err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 4,
		[]float64{0}, []float64{1},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	observations, err := archive.ListSemanticRuleObservations(ctx, 100)
	if err != nil {
		t.Fatal(err)
	}
	var values []int64
	for _, observation := range observations {
		if observation.RuleID != counter.ID {
			continue
		}
		var value int64
		if err := json.Unmarshal(observation.Value, &value); err != nil {
			t.Fatal(err)
		}
		values = append(values, value)
	}
	if len(values) != 3 || values[0] != 1 || values[1] != 0 || values[2] != 1 {
		t.Fatalf("counter reset observations = %#v", values)
	}
}

func TestSemanticV3CounterResetAfterSeriesRotationUsesNewGeneration(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, err := archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	rule, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		"累積値",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 2,
		[]float64{0}, []float64{1},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 10); err != nil {
		t.Fatal(err)
	}
	configuration, err = archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	rotated, err := archive.UpdateSignalCalibration(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		-1,
		1,
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.RequestSemanticCounterReset(
		ctx, edgeapp.LocalCLIActor(), rule.ID, "reset-after-rotation",
	); err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 4,
		[]float64{1}, []float64{0},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 10); err != nil {
		t.Fatal(err)
	}
	var sequences []int64
	rows, err := archive.db.Query(`
		SELECT sequence FROM semantic_observations_v3
		WHERE rule_id = ? AND series_id = ?
		ORDER BY sequence
	`, rule.ID, rotated.Rules[0].SeriesID)
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	for rows.Next() {
		var sequence int64
		if err := rows.Scan(&sequence); err != nil {
			t.Fatal(err)
		}
		sequences = append(sequences, sequence)
	}
	if len(sequences) != 2 || sequences[0] != 1 || sequences[1] != 2 {
		t.Fatalf("new generation sequences=%#v", sequences)
	}
}

func TestSemanticV3CounterResetBlocksInputFromLaterLedgerEpoch(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, _ := archive.GetSemanticConfiguration(ctx, signalRef)
	counter, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		"生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 2,
		[]float64{0},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 3,
		[]float64{1},
	)
	if _, err := archive.RequestSemanticCounterReset(
		ctx,
		edgeapp.LocalCLIActor(),
		counter.ID,
		"reset-before-new-epoch",
	); err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-b", 1,
		[]float64{0}, []float64{1},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	observations, err := archive.ListSemanticRuleObservations(ctx, 100)
	if err != nil {
		t.Fatal(err)
	}
	var values []int64
	for _, observation := range observations {
		if observation.RuleID != counter.ID {
			continue
		}
		var value int64
		if err := json.Unmarshal(observation.Value, &value); err != nil {
			t.Fatal(err)
		}
		values = append(values, value)
	}
	if len(values) != 3 ||
		values[0] != 1 ||
		values[1] != 0 ||
		values[2] != 1 {
		t.Fatalf("cross-epoch reset observations=%#v", values)
	}
}

func TestSemanticV3StartsActiveRulesInNewLedgerEpochWithoutFalseCount(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, _ := archive.GetSemanticConfiguration(ctx, signalRef)
	counter, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		"生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 2,
		[]float64{0}, []float64{1}, []float64{0},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-b", 1,
		[]float64{1},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-b", 2,
		[]float64{0}, []float64{1},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	observations, err := archive.ListSemanticRuleObservations(ctx, 100)
	if err != nil {
		t.Fatal(err)
	}
	var values []int64
	for _, observation := range observations {
		if observation.RuleID != counter.ID {
			continue
		}
		var value int64
		if err := json.Unmarshal(observation.Value, &value); err != nil {
			t.Fatal(err)
		}
		values = append(values, value)
	}
	if len(values) != 2 || values[0] != 1 || values[1] != 2 {
		t.Fatalf("counter observations across ledger epochs=%#v", values)
	}
}

func TestSemanticV3StartFailureDoesNotRollbackRawCustody(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	configuration, _ := archive.GetSemanticConfiguration(
		context.Background(),
		signalRef,
	)
	if _, err := archive.CreateSemanticRule(
		context.Background(),
		edgeapp.LocalCLIActor(),
		signalRef,
		"測定値",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.db.Exec(`
		CREATE TRIGGER fail_semantic_epoch_start
		BEFORE INSERT ON semantic_rule_starts_v3
		WHEN NEW.ledger_epoch = 'epoch-b'
		BEGIN
			SELECT RAISE(FAIL, 'semantic start unavailable');
		END
	`); err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-b", 1,
		[]float64{4},
	)
	var rawCount int
	if err := archive.db.QueryRow(`
		SELECT count(*) FROM raw_records
		WHERE edge_node_id = 'edge-node-01' AND ledger_epoch = 'epoch-b'
	`).Scan(&rawCount); err != nil {
		t.Fatal(err)
	}
	if rawCount != 1 {
		t.Fatalf("epoch-b raw count=%d", rawCount)
	}
	if _, err := archive.ProjectSemanticRules(context.Background(), 100); err == nil {
		t.Fatal("semantic projection ignored start materialization failure")
	}
}

func TestSemanticV3UsesLatestCalibrationWhenSeveralChangesShareCursor(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, _ := archive.GetSemanticConfiguration(ctx, signalRef)
	rule, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		"補正値",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	configuration, _ = archive.GetSemanticConfiguration(ctx, signalRef)
	if _, err := archive.UpdateSignalCalibration(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		2,
		0,
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	); err != nil {
		t.Fatal(err)
	}
	configuration, _ = archive.GetSemanticConfiguration(ctx, signalRef)
	if _, err := archive.UpdateSignalCalibration(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		3,
		0,
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	); err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 2,
		[]float64{2},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	observations, err := archive.ListSemanticRuleObservations(ctx, 100)
	if err != nil {
		t.Fatal(err)
	}
	if len(observations) != 1 || observations[0].RuleID != rule.ID {
		t.Fatalf("observations=%#v", observations)
	}
	var value float64
	if err := json.Unmarshal(observations[0].Value, &value); err != nil {
		t.Fatal(err)
	}
	if value != 6 || observations[0].CalibrationRevision != 3 {
		t.Fatalf("value=%v observation=%#v", value, observations[0])
	}
}

func TestSemanticV3FinishesLaggedInputBeforeRebaseliningRuleRevision(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, _ := archive.GetSemanticConfiguration(ctx, signalRef)
	counter, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		"生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 2,
		[]float64{0},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 3,
		[]float64{1},
	)
	if _, err := archive.UpdateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		counter.ID,
		"良品数",
		counter.RuleSpec,
		edgeapp.RevisionPrecondition{Expected: &counter.Revision},
	); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	observations, err := archive.ListSemanticRuleObservations(ctx, 100)
	if err != nil {
		t.Fatal(err)
	}
	if len(observations) != 1 {
		t.Fatalf("lagged observations=%#v", observations)
	}
	var value int64
	if err := json.Unmarshal(observations[0].Value, &value); err != nil {
		t.Fatal(err)
	}
	if value != 1 || observations[0].RuleRevision != 1 {
		t.Fatalf("lagged value=%d observation=%#v", value, observations[0])
	}
}
