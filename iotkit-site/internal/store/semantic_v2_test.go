package store

import (
	"context"
	"encoding/json"
	"reflect"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

func TestSemanticDefinitionIsFutureOnlyAndProjectionIsIdempotent(t *testing.T) {
	archive := openTestStore(t)
	ctx := context.Background()
	acceptSemanticBatch(t, archive, "edge-node-01", "epoch-a", 1, []float64{1})
	signalRef := reconcileSingleSignal(t, archive)

	definition, err := archive.ApplySemanticDefinition(
		ctx,
		siteapp.LocalCLIActor(),
		signalRef,
		semantics.DefinitionSpec{
			Kind:  semantics.KindBoolean,
			Scale: 1,
			Condition: semantics.Condition{
				Mode:      semantics.ConditionBoolean,
				BoolValue: true,
			},
		},
		siteapp.RevisionPrecondition{},
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
		siteapp.LocalCLIActor(),
		signalRef,
		semantics.DefinitionSpec{
			Kind:  semantics.KindCumulativeCounter,
			Scale: 1,
			Condition: semantics.Condition{
				Mode:      semantics.ConditionBoolean,
				BoolValue: true,
			},
			Trigger: semantics.TriggerTransition,
		},
		siteapp.RevisionPrecondition{},
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
			siteapp.LocalCLIActor(),
			signal.SignalRef,
			semantics.DefinitionSpec{
				Kind:  semantics.KindBoolean,
				Scale: 1,
				Condition: semantics.Condition{
					Mode:      semantics.ConditionBoolean,
					BoolValue: true,
				},
			},
			siteapp.RevisionPrecondition{},
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
		t.Fatalf("multi-Edge observations = %#v", observations)
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
			ctx, siteapp.LocalCLIActor(), signal.SignalRef,
			semantics.DefinitionSpec{
				Kind: semantics.KindBoolean, Scale: 1,
				Condition: semantics.Condition{
					Mode: semantics.ConditionBoolean, BoolValue: true,
				},
			}, siteapp.RevisionPrecondition{},
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
		record, err := json.Marshal(map[string]any{
			"family":         "measurement",
			"schema_version": 1,
			"epoch":          ledgerEpoch,
			"pub_seq":        pubSeq,
			"series_key":     inventorySeriesKey,
			"values":         values,
			"event_time":     pubSeq * 1_000,
		})
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
	if _, err := archive.AcceptBatch(context.Background(), batch); err != nil {
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
