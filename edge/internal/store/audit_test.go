package store

import (
	"context"
	"errors"
	"reflect"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantic"
)

func TestApplySemanticMappingCommitsAuditWithFutureOnlyRevision(t *testing.T) {
	store := openTestStore(t)
	for pubSeq := int64(1); pubSeq <= 3; pubSeq++ {
		acceptEpoch(t, store, "edge-node-01", "epoch-a", pubSeq, 1)
	}
	mapping, err := store.ApplySemanticMapping(context.Background(), edgeapp.LocalCLIActor(), semantic.MappingSpec{
		EdgeNodeID:  "edge-node-01",
		SeriesKey:   contactSeries,
		Meaning:     semantic.MeaningProductionPulse,
		TriggerMode: semantic.TriggerActiveEdge,
		ActiveValue: 1,
	}, edgeapp.RevisionPrecondition{})
	if err != nil {
		t.Fatal(err)
	}
	events, err := store.ListAuditEvents(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) != 1 || events[0].Operation != "semantic_mapping.put" || events[0].ResourceRef != mapping.ID {
		t.Fatalf("audit events = %#v", events)
	}
	if got := store.testMappingStarts(t, mapping.ID, mapping.Revision); !reflect.DeepEqual(got, map[string]int64{"epoch-a": 3}) {
		t.Fatalf("mapping starts = %#v", got)
	}
}

func TestApplySemanticMappingRejectsRevisionMismatchWithoutMutationOrAudit(t *testing.T) {
	store := openTestStore(t)
	first, err := store.ApplySemanticMapping(
		context.Background(),
		edgeapp.LocalCLIActor(),
		semantic.MappingSpec{
			EdgeNodeID:  "edge-node-01",
			SeriesKey:   contactSeries,
			Meaning:     semantic.MeaningProductionPulse,
			TriggerMode: semantic.TriggerActiveSample,
			ActiveValue: 1,
		},
		edgeapp.RevisionPrecondition{},
	)
	if err != nil {
		t.Fatal(err)
	}
	wrongRevision := first.Revision + 1
	_, err = store.ApplySemanticMapping(
		context.Background(),
		edgeapp.LocalCLIActor(),
		semantic.MappingSpec{
			EdgeNodeID:  "edge-node-01",
			SeriesKey:   contactSeries,
			Meaning:     semantic.MeaningProductionPulse,
			TriggerMode: semantic.TriggerActiveEdge,
			ActiveValue: 0,
		},
		edgeapp.RevisionPrecondition{Expected: &wrongRevision},
	)
	if !errors.Is(err, edgeapp.ErrRevisionMismatch) {
		t.Fatalf("error = %v, want revision mismatch", err)
	}
	mappings, err := store.ListSemanticMappings(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(mappings) != 1 || mappings[0].Revision != first.Revision || !mappings[0].Active {
		t.Fatalf("mappings = %#v", mappings)
	}
	events, err := store.ListAuditEvents(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) != 1 {
		t.Fatalf("audit events = %#v", events)
	}
}

func TestApplySemanticMappingRollsBackWhenAuditInsertFails(t *testing.T) {
	store := openTestStore(t)
	if _, err := store.db.Exec(`
		CREATE TRIGGER fail_audit BEFORE INSERT ON audit_events
		BEGIN SELECT RAISE(ABORT, 'injected audit failure'); END;
	`); err != nil {
		t.Fatal(err)
	}
	_, err := store.ApplySemanticMapping(
		context.Background(),
		edgeapp.LocalCLIActor(),
		semantic.MappingSpec{
			EdgeNodeID:  "edge-node-01",
			SeriesKey:   contactSeries,
			Meaning:     semantic.MeaningProductionPulse,
			TriggerMode: semantic.TriggerActiveSample,
			ActiveValue: 1,
		},
		edgeapp.RevisionPrecondition{},
	)
	if err == nil {
		t.Fatal("mapping mutation succeeded despite audit failure")
	}
	if got := store.testCount(t, "semantic_mappings"); got != 0 {
		t.Fatalf("mapping count = %d, want 0", got)
	}
}

func TestDeactivateSemanticMappingClosesCurrentRevisionAndAudits(t *testing.T) {
	store := openTestStore(t)
	mapping, err := store.ApplySemanticMapping(
		context.Background(),
		edgeapp.LocalCLIActor(),
		semantic.MappingSpec{
			EdgeNodeID:  "edge-node-01",
			SeriesKey:   contactSeries,
			Meaning:     semantic.MeaningProductionPulse,
			TriggerMode: semantic.TriggerActiveSample,
			ActiveValue: 1,
		},
		edgeapp.RevisionPrecondition{},
	)
	if err != nil {
		t.Fatal(err)
	}
	for pubSeq := int64(1); pubSeq <= 4; pubSeq++ {
		acceptEpoch(t, store, "edge-node-01", "epoch-a", pubSeq, 0)
	}
	inactive, err := store.DeactivateSemanticMapping(
		context.Background(),
		edgeapp.LocalCLIActor(),
		mapping.EdgeNodeID,
		mapping.SeriesKey,
		edgeapp.RevisionPrecondition{Expected: &mapping.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	if inactive.Active {
		t.Fatal("mapping remains active")
	}
	if got := store.testMappingEnds(t, mapping.ID, mapping.Revision); !reflect.DeepEqual(got, map[string]int64{"epoch-a": 4}) {
		t.Fatalf("mapping ends = %#v", got)
	}
	events, err := store.ListAuditEvents(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) != 2 || events[0].Operation != "semantic_mapping.deactivate" {
		t.Fatalf("audit events = %#v", events)
	}
}
