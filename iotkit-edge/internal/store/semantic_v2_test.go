package store

import (
	"context"
	"encoding/json"
	"errors"
	"reflect"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
)

func TestListSemanticPreviewWindowUsesLatestBoundedInputsInCustodyOrder(t *testing.T) {
	archive := openTestStore(t)
	acceptSemanticBatch(
		t,
		archive,
		"edge-node-01",
		"epoch-a",
		1,
		[]float64{1},
		[]float64{2},
		[]float64{3},
		[]float64{4},
		[]float64{5},
	)
	signalRef := reconcileSingleSignal(t, archive)
	if _, err := archive.db.Exec(`
		UPDATE raw_records SET received_at = pub_seq * 1000
	`); err != nil {
		t.Fatal(err)
	}

	window, err := archive.ListSemanticPreviewWindow(
		context.Background(),
		signalRef,
		2_000,
		3,
	)
	if err != nil {
		t.Fatal(err)
	}
	if len(window.Inputs) != 3 ||
		window.Inputs[0].Value != 3 ||
		window.Inputs[2].Value != 5 {
		t.Fatalf("preview inputs = %#v", window.Inputs)
	}
	if window.WindowStart != 3_000 || window.WindowEnd != 5_000 ||
		window.TruncatedBy != PreviewTruncatedByInputCount {
		t.Fatalf("preview window = %#v", window)
	}

	timeWindow, err := archive.ListSemanticPreviewWindow(
		context.Background(),
		signalRef,
		4_000,
		10,
	)
	if err != nil {
		t.Fatal(err)
	}
	if len(timeWindow.Inputs) != 2 ||
		timeWindow.TruncatedBy != PreviewTruncatedByTime {
		t.Fatalf("time-limited preview window = %#v", timeWindow)
	}
}

func TestListSemanticPreviewWindowRejectsUnknownSignalAndInvalidLimit(t *testing.T) {
	archive := openTestStore(t)
	if _, err := archive.ListSemanticPreviewWindow(
		context.Background(),
		"sig_00000000000000000000000000000000",
		0,
		100,
	); !errors.Is(err, edgeapp.ErrNotFound) {
		t.Fatalf("unknown signal error = %v", err)
	}
	if _, err := archive.ListSemanticPreviewWindow(
		context.Background(),
		"sig_00000000000000000000000000000000",
		0,
		0,
	); err == nil {
		t.Fatal("zero preview limit was accepted")
	}
}

func TestSemanticDefinitionIsFutureOnlyAndProjectionIsIdempotent(t *testing.T) {
	archive := openTestStore(t)
	ctx := context.Background()
	acceptSemanticBatch(t, archive, "edge-node-01", "epoch-a", 1, []float64{1})
	signalRef := reconcileSingleSignal(t, archive)

	definition, err := archive.ApplySemanticDefinition(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		semantics.DefinitionSpec{
			Kind:  semantics.KindBoolean,
			Scale: 1,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
		},
		edgeapp.RevisionPrecondition{},
	)
	if err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t,
		archive,
		"edge-node-01",
		"epoch-a",
		2,
		[]float64{0},
		[]float64{1},
	)
	if _, err := archive.ProjectSemanticObservations(ctx, 100); err != nil {
		t.Fatal(err)
	}
	first, err := archive.ListSemanticObservations(ctx, 100)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ProjectSemanticObservations(ctx, 100); err != nil {
		t.Fatal(err)
	}
	second, err := archive.ListSemanticObservations(ctx, 100)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(first, second) {
		t.Fatalf("projection was not idempotent: first=%#v second=%#v", first, second)
	}
	if len(first) != 2 || first[0].SourcePubSeq != 2 || first[1].SourcePubSeq != 3 {
		t.Fatalf("future-only observations = %#v", first)
	}
	if first[0].DefinitionID != definition.ID ||
		first[0].SeriesID != definition.SeriesID ||
		first[0].Sequence != 1 ||
		first[1].Sequence != 2 {
		t.Fatalf("observation identity = %#v", first)
	}
	var firstValue bool
	if err := json.Unmarshal(first[0].Value, &firstValue); err != nil || firstValue {
		t.Fatalf("first value = %s, err=%v", first[0].Value, err)
	}
	var secondValue bool
	if err := json.Unmarshal(first[1].Value, &secondValue); err != nil || !secondValue {
		t.Fatalf("second value = %s, err=%v", first[1].Value, err)
	}
}

func TestCumulativeDefinitionUsesBaselineAndPersistsCounter(t *testing.T) {
	archive := openTestStore(t)
	ctx := context.Background()
	acceptSemanticBatch(t, archive, "edge-node-01", "epoch-a", 1, []float64{0})
	signalRef := reconcileSingleSignal(t, archive)
	_, err := archive.ApplySemanticDefinition(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		semantics.DefinitionSpec{
			Kind:  semantics.KindCumulativeCounter,
			Scale: 1,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{},
	)
	if err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t,
		archive,
		"edge-node-01",
		"epoch-a",
		2,
		[]float64{1},
		[]float64{0},
		[]float64{1},
	)
	if _, err := archive.ProjectSemanticObservations(ctx, 1); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ProjectSemanticObservations(ctx, 100); err != nil {
		t.Fatal(err)
	}
	observations, err := archive.ListSemanticObservations(ctx, 100)
	if err != nil {
		t.Fatal(err)
	}
	if len(observations) != 1 {
		t.Fatalf("counter observations = %#v, want one after baseline", observations)
	}
	var value int64
	if err := json.Unmarshal(observations[0].Value, &value); err != nil || value != 1 {
		t.Fatalf("counter value = %s, err=%v", observations[0].Value, err)
	}
}

func TestSemanticDefinitionsKeepTwoEdgesIsolated(t *testing.T) {
	archive := openTestStore(t)
	ctx := context.Background()
	for _, edgeNodeID := range []string{"edge-node-01", "edge-node-02"} {
		acceptSemanticBatch(t, archive, edgeNodeID, "epoch-a", 1, []float64{0})
	}
	if _, err := archive.ReconcileInventorySources(ctx, 100); err != nil {
		t.Fatal(err)
	}
	signals, err := archive.ListInventorySignals(ctx, 100, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(signals) != 2 {
		t.Fatalf("signals = %#v, want two", signals)
	}
	for _, signal := range signals {
		if _, err := archive.ApplySemanticDefinition(
			ctx,
			edgeapp.LocalCLIActor(),
			signal.SignalRef,
			semantics.DefinitionSpec{
				Kind:  semantics.KindBoolean,
				Scale: 1,
				Detector: semantics.Detector{
					Mode: semantics.DetectorBooleanHighActive,
				},
			},
			edgeapp.RevisionPrecondition{},
		); err != nil {
			t.Fatal(err)
		}
	}
	acceptSemanticBatch(t, archive, "edge-node-01", "epoch-a", 2, []float64{1})
	acceptSemanticBatch(t, archive, "edge-node-02", "epoch-a", 2, []float64{0})
	if _, err := archive.ProjectSemanticObservations(ctx, 100); err != nil {
		t.Fatal(err)
	}
	observations, err := archive.ListSemanticObservations(ctx, 100)
	if err != nil {
		t.Fatal(err)
	}
	if len(observations) != 2 ||
		observations[0].EdgeNodeID == observations[1].EdgeNodeID {
		t.Fatalf("multi-EdgeNode observations = %#v", observations)
	}
}

func TestSemanticProjectionRecordsPoisonAndContinuesIndependentDefinition(t *testing.T) {
	archive := openTestStore(t)
	ctx := context.Background()
	for _, edgeNodeID := range []string{"edge-node-01", "edge-node-02"} {
		acceptSemanticBatch(t, archive, edgeNodeID, "epoch-a", 1, []float64{0})
	}
	if _, err := archive.ReconcileInventorySources(ctx, 100); err != nil {
		t.Fatal(err)
	}
	signals, err := archive.ListInventorySignals(ctx, 100, "")
	if err != nil || len(signals) != 2 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	for _, signal := range signals {
		if _, err := archive.ApplySemanticDefinition(
			ctx, edgeapp.LocalCLIActor(), signal.SignalRef,
			semantics.DefinitionSpec{
				Kind: semantics.KindBoolean, Scale: 1,
				Detector: semantics.Detector{
					Mode: semantics.DetectorBooleanHighActive,
				},
			}, edgeapp.RevisionPrecondition{},
		); err != nil {
			t.Fatal(err)
		}
	}
	acceptSemanticBatch(t, archive, "edge-node-01", "epoch-a", 2, []float64{0, 1})
	acceptSemanticBatch(t, archive, "edge-node-02", "epoch-a", 2, []float64{1})
	if _, err := archive.ProjectSemanticObservations(ctx, 100); err == nil {
		t.Fatal("poison semantic input did not report an error")
	}
	observations, err := archive.ListSemanticObservations(ctx, 100)
	if err != nil {
		t.Fatal(err)
	}
	if len(observations) != 1 || observations[0].EdgeNodeID != "edge-node-02" {
		t.Fatalf("independent projection observations=%#v", observations)
	}
	failures, err := archive.SemanticProjectionFailureCount(ctx)
	if err != nil || failures != 1 {
		t.Fatalf("projection failures=%d err=%v", failures, err)
	}
}

func TestSemanticProjectionPersistsPendingDebounceAcrossRuns(t *testing.T) {
	archive := openTestStore(t)
	ctx := context.Background()
	acceptSemanticBatch(t, archive, "edge-node-01", "epoch-a", 1, []float64{0})
	signalRef := reconcileSingleSignal(t, archive)
	if _, err := archive.ApplySemanticDefinition(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		semantics.DefinitionSpec{
			Kind:  semantics.KindBoolean,
			Scale: 1,
			Detector: semantics.Detector{
				Mode:           semantics.DetectorBooleanHighActive,
				RiseDebounceMS: 1_000,
			},
		},
		edgeapp.RevisionPrecondition{},
	); err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 2,
		[]float64{0}, []float64{1}, []float64{1},
	)
	if _, err := archive.db.Exec(`
		UPDATE raw_records SET received_at = pub_seq * 1000
	`); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ProjectSemanticObservations(ctx, 2); err != nil {
		t.Fatal(err)
	}
	var pending bool
	var pendingActive bool
	var pendingSince int64
	if err := archive.db.QueryRow(`
		SELECT pending, pending_active, pending_since
		FROM semantic_definition_state_v2
	`).Scan(&pending, &pendingActive, &pendingSince); err != nil {
		t.Fatal(err)
	}
	if !pending || !pendingActive || pendingSince != 3_000 {
		t.Fatalf(
			"pending=%v active=%v since=%d",
			pending, pendingActive, pendingSince,
		)
	}
	if _, err := archive.ProjectSemanticObservations(ctx, 100); err != nil {
		t.Fatal(err)
	}
	observations, err := archive.ListSemanticObservations(ctx, 100)
	if err != nil {
		t.Fatal(err)
	}
	if len(observations) != 2 {
		t.Fatalf("observations = %#v, want baseline and confirmed transition", observations)
	}
	var active bool
	if err := json.Unmarshal(observations[1].Value, &active); err != nil || !active {
		t.Fatalf("confirmed value=%s err=%v", observations[1].Value, err)
	}
}

func TestResetSemanticCounterKeepsDefinitionRevisionAndWritesAudit(t *testing.T) {
	archive := openTestStore(t)
	ctx := context.Background()
	acceptSemanticBatch(t, archive, "edge-node-01", "epoch-a", 1, []float64{0})
	signalRef := reconcileSingleSignal(t, archive)
	definition, err := archive.ApplySemanticDefinition(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		semantics.DefinitionSpec{
			Kind:  semantics.KindCumulativeCounter,
			Scale: 1,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{},
	)
	if err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 2,
		[]float64{0}, []float64{1},
	)
	if _, err := archive.ProjectSemanticObservations(ctx, 100); err != nil {
		t.Fatal(err)
	}
	reset, err := archive.ResetSemanticCounter(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		edgeapp.RevisionPrecondition{Expected: &definition.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	if reset.Revision != definition.Revision {
		t.Fatalf("reset changed definition revision from %d to %d", definition.Revision, reset.Revision)
	}
	var counter int64
	if err := archive.db.QueryRow(`
		SELECT counter FROM semantic_definition_state_v2
		WHERE definition_id = ? AND definition_revision = ?
	`, definition.ID, definition.Revision).Scan(&counter); err != nil {
		t.Fatal(err)
	}
	if counter != 0 {
		t.Fatalf("counter after reset = %d", counter)
	}
	var auditCount int
	if err := archive.db.QueryRow(`
		SELECT count(*) FROM audit_events
		WHERE operation = 'semantic_counter.reset'
			AND resource_ref = ?
	`, definition.ID).Scan(&auditCount); err != nil {
		t.Fatal(err)
	}
	if auditCount != 1 {
		t.Fatalf("reset audit count = %d", auditCount)
	}
}

func acceptSemanticBatch(
	t *testing.T,
	archive *Store,
	edgeNodeID string,
	ledgerEpoch string,
	start int64,
	samples ...[]float64,
) {
	t.Helper()
	records := make([]json.RawMessage, 0, len(samples))
	for index, values := range samples {
		pubSeq := start + int64(index)
		record, err := json.Marshal(testMeasurementRecord(
			ledgerEpoch, pubSeq, inventorySeriesKey, values, pubSeq*1_000,
		))
		if err != nil {
			t.Fatal(err)
		}
		records = append(records, record)
	}
	batch := contract.RecordBatch{
		SchemaVersion: 1,
		EdgeNodeID:    edgeNodeID,
		LedgerEpoch:   ledgerEpoch,
		PublicationID: contract.PublicationID(
			edgeNodeID,
			ledgerEpoch,
			start,
			start+int64(len(records))-1,
		),
		CursorStart: start,
		CursorEnd:   start + int64(len(records)) - 1,
		Records:     records,
	}
	if _, err := acceptBatchForTest(t, archive, batch); err != nil {
		t.Fatal(err)
	}
}

func reconcileSingleSignal(t *testing.T, archive *Store) string {
	t.Helper()
	if _, err := archive.ReconcileInventorySources(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
	signals, err := archive.ListInventorySignals(context.Background(), 100, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(signals) != 1 {
		t.Fatalf("signals = %#v, want one", signals)
	}
	return signals[0].SignalRef
}
