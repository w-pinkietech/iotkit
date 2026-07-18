package store

import (
	"context"
	"encoding/json"
	"errors"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

func discoverTestEdge(t *testing.T, store *Store) EdgeActivation {
	t.Helper()
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), descriptorFixture(t)); err != nil {
		t.Fatal(err)
	}
	edges, err := store.ListEdges(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(edges) != 1 {
		t.Fatalf("edges = %#v", edges)
	}
	return edges[0]
}

func requestTestActivation(t *testing.T, store *Store, edge EdgeActivation) (EdgeActivation, ActivationCommand) {
	t.Helper()
	expected := edge.Revision
	requested, err := store.RequestEdgeActivation(
		context.Background(),
		siteapp.LocalCLIActor(),
		edge.EdgeRef,
		siteapp.RevisionPrecondition{Expected: &expected},
	)
	if err != nil {
		t.Fatal(err)
	}
	commands, err := store.ListPendingActivationCommands(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(commands) != 1 {
		t.Fatalf("commands = %#v", commands)
	}
	return requested, commands[0]
}

func resultForCommand(t *testing.T, command ActivationCommand) contract.ActivationResult {
	t.Helper()
	request, err := contract.DecodeActivationRequest(command.PayloadJSON)
	if err != nil {
		t.Fatal(err)
	}
	return contract.ActivationResult{
		SchemaVersion:            1,
		ActivationID:             request.ActivationID,
		SiteID:                   request.SiteID,
		EdgeNodeID:               request.EdgeNodeID,
		LedgerEpoch:              request.ExpectedLedgerEpoch,
		Status:                   "applied",
		DiscardThroughReadingSeq: 2,
		FirstPublicationSeq:      1,
		AppliedAt:                request.IssuedAt + 1,
	}
}

func TestDescriptorDiscoversEdgeWithoutAuthorizingRawCustody(t *testing.T) {
	store := openTestStore(t)
	edge := discoverTestEdge(t, store)

	if edge.State != EdgeDiscovered || edge.EdgeNodeID != "edge-node-01" ||
		edge.LedgerEpoch != "epoch-01" || edge.EdgeRef == "" {
		t.Fatalf("edge = %#v", edge)
	}
	if _, err := store.AcceptBatch(context.Background(), testBatch(t)); !errors.Is(err, ErrEdgeNotActive) {
		t.Fatalf("AcceptBatch error = %v, want ErrEdgeNotActive", err)
	}
	if got := store.testCount(t, "raw_records"); got != 0 {
		t.Fatalf("raw records = %d, want 0", got)
	}
	if got := store.testCount(t, "accepted_cursors"); got != 0 {
		t.Fatalf("accepted cursors = %d, want 0", got)
	}
}

func TestActivationGrantResultAndDuplicateAreDurableAndIdempotent(t *testing.T) {
	store := openTestStore(t)
	edge := discoverTestEdge(t, store)
	requested, command := requestTestActivation(t, store, edge)

	if requested.State != EdgeActivating || requested.ActivationID == "" {
		t.Fatalf("requested = %#v", requested)
	}
	if command.ActivationID != requested.ActivationID ||
		command.Topic != "iotkit/v1/edge-nodes/edge-node-01/activation/request" {
		t.Fatalf("command = %#v", command)
	}
	request, err := contract.DecodeActivationRequest(command.PayloadJSON)
	if err != nil {
		t.Fatal(err)
	}
	if request.EdgeNodeID != edge.EdgeNodeID || request.ExpectedLedgerEpoch != edge.LedgerEpoch {
		t.Fatalf("request = %#v", request)
	}

	duplicate, err := store.RequestEdgeActivation(
		context.Background(),
		siteapp.LocalCLIActor(),
		edge.EdgeRef,
		siteapp.RevisionPrecondition{},
	)
	if err != nil {
		t.Fatal(err)
	}
	if duplicate.ActivationID != requested.ActivationID {
		t.Fatalf("duplicate activation = %#v, want activation_id %s", duplicate, requested.ActivationID)
	}
	if commands, err := store.ListPendingActivationCommands(context.Background(), 10); err != nil || len(commands) != 1 {
		t.Fatalf("pending commands = %#v, %v", commands, err)
	}

	result := resultForCommand(t, command)
	active, err := store.ApplyActivationResult(context.Background(), result)
	if err != nil {
		t.Fatal(err)
	}
	if active.State != EdgeActive {
		t.Fatalf("active = %#v", active)
	}
	replayed, err := store.ApplyActivationResult(context.Background(), result)
	if err != nil || replayed.State != EdgeActive {
		t.Fatalf("replayed = %#v, %v", replayed, err)
	}
	if commands, err := store.ListPendingActivationCommands(context.Background(), 10); err != nil || len(commands) != 0 {
		t.Fatalf("pending commands after result = %#v, %v", commands, err)
	}

	ack, err := store.AcceptBatch(context.Background(), testBatch(t))
	if err != nil {
		t.Fatal(err)
	}
	if ack.AcceptedThrough != 1 || store.testCount(t, "raw_records") != 1 {
		t.Fatalf("ack = %#v raw=%d", ack, store.testCount(t, "raw_records"))
	}
	events, err := store.ListAuditEvents(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	var found bool
	for _, event := range events {
		if event.Operation == "edge.activation.request" && event.Outcome == "success" {
			found = true
		}
	}
	if !found {
		encoded, _ := json.Marshal(events)
		t.Fatalf("activation audit missing: %s", encoded)
	}
}

func TestConflictingActivationResultFailsClosedIntoRecoveryHold(t *testing.T) {
	store := openTestStore(t)
	edge := discoverTestEdge(t, store)
	_, command := requestTestActivation(t, store, edge)
	result := resultForCommand(t, command)
	result.LedgerEpoch = "epoch-other"

	if _, err := store.ApplyActivationResult(context.Background(), result); !errors.Is(err, ErrActivationConflict) {
		t.Fatalf("error = %v, want ErrActivationConflict", err)
	}
	edges, err := store.ListEdges(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(edges) != 1 || edges[0].State != EdgeRecoveryHold {
		t.Fatalf("edges = %#v", edges)
	}
	if _, err := store.AcceptBatch(context.Background(), testBatch(t)); !errors.Is(err, ErrEdgeNotActive) {
		t.Fatalf("AcceptBatch error = %v, want ErrEdgeNotActive", err)
	}
}

func TestActiveEdgeRejectsUnexpectedEpochWithoutStoringOrAcknowledging(t *testing.T) {
	store := openTestStore(t)
	edge := discoverTestEdge(t, store)
	_, command := requestTestActivation(t, store, edge)
	if _, err := store.ApplyActivationResult(context.Background(), resultForCommand(t, command)); err != nil {
		t.Fatal(err)
	}
	batch := testBatch(t)
	batch.LedgerEpoch = "epoch-other"
	batch.PublicationID = contract.PublicationID(batch.EdgeNodeID, batch.LedgerEpoch, 1, 1)
	batch.Records[0] = json.RawMessage(`{"family":"measurement","schema_version":1,"epoch":"epoch-other","pub_seq":1,"series_key":"series-temperature-01","values":[21.5]}`)

	if _, err := store.AcceptBatch(context.Background(), batch); !errors.Is(err, ErrEdgeNotActive) {
		t.Fatalf("error = %v, want ErrEdgeNotActive", err)
	}
	if store.testCount(t, "raw_records") != 0 || store.testCount(t, "accepted_cursors") != 0 {
		t.Fatal("unexpected epoch changed raw custody state")
	}
}
