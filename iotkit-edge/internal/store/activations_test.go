package store

import (
	"context"
	"encoding/json"
	"errors"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
)

func discoverTestEdge(t *testing.T, store *Store) EdgeNodeActivation {
	t.Helper()
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), descriptorFixture(t)); err != nil {
		t.Fatal(err)
	}
	edgeNodes, err := store.ListEdgeNodes(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(edgeNodes) != 1 {
		t.Fatalf("edgeNodes = %#v", edgeNodes)
	}
	return edgeNodes[0]
}

func requestTestActivation(t *testing.T, store *Store, edgeNode EdgeNodeActivation) (EdgeNodeActivation, ActivationCommand) {
	t.Helper()
	expected := edgeNode.Revision
	requested, err := store.RequestEdgeNodeActivation(
		context.Background(),
		edgeapp.LocalCLIActor(),
		edgeNode.EdgeNodeRef,
		edgeapp.RevisionPrecondition{Expected: &expected},
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
		EdgeID:                   request.EdgeID,
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
	edgeNode := discoverTestEdge(t, store)

	if edgeNode.State != EdgeNodeDiscovered || edgeNode.EdgeNodeID != "edge-node-01" ||
		edgeNode.LedgerEpoch != "epoch-01" || edgeNode.EdgeNodeRef == "" {
		t.Fatalf("edgeNode = %#v", edgeNode)
	}
	if _, err := store.AcceptBatch(context.Background(), testBatch(t)); !errors.Is(err, ErrEdgeNodeNotActive) {
		t.Fatalf("AcceptBatch error = %v, want ErrEdgeNodeNotActive", err)
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
	edgeNode := discoverTestEdge(t, store)
	requested, command := requestTestActivation(t, store, edgeNode)

	if requested.State != EdgeNodeActivating || requested.ActivationID == "" {
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
	if request.EdgeNodeID != edgeNode.EdgeNodeID || request.ExpectedLedgerEpoch != edgeNode.LedgerEpoch {
		t.Fatalf("request = %#v", request)
	}

	duplicate, err := store.RequestEdgeNodeActivation(
		context.Background(),
		edgeapp.LocalCLIActor(),
		edgeNode.EdgeNodeRef,
		edgeapp.RevisionPrecondition{},
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
	if active.State != EdgeNodeActive {
		t.Fatalf("active = %#v", active)
	}
	replayed, err := store.ApplyActivationResult(context.Background(), result)
	if err != nil || replayed.State != EdgeNodeActive {
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
		if event.Operation == "edge_node.activation.request" && event.Outcome == "success" {
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
	edgeNode := discoverTestEdge(t, store)
	_, command := requestTestActivation(t, store, edgeNode)
	result := resultForCommand(t, command)
	result.LedgerEpoch = "epoch-other"

	if _, err := store.ApplyActivationResult(context.Background(), result); !errors.Is(err, ErrActivationConflict) {
		t.Fatalf("error = %v, want ErrActivationConflict", err)
	}
	edgeNodes, err := store.ListEdgeNodes(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(edgeNodes) != 1 || edgeNodes[0].State != EdgeNodeRecoveryHold {
		t.Fatalf("edgeNodes = %#v", edgeNodes)
	}
	if _, err := store.AcceptBatch(context.Background(), testBatch(t)); !errors.Is(err, ErrEdgeNodeNotActive) {
		t.Fatalf("AcceptBatch error = %v, want ErrEdgeNodeNotActive", err)
	}
}

func TestActiveEdgeRejectsUnexpectedEpochWithoutStoringOrAcknowledging(t *testing.T) {
	store := openTestStore(t)
	edgeNode := discoverTestEdge(t, store)
	_, command := requestTestActivation(t, store, edgeNode)
	if _, err := store.ApplyActivationResult(context.Background(), resultForCommand(t, command)); err != nil {
		t.Fatal(err)
	}
	batch := testBatch(t)
	batch.LedgerEpoch = "epoch-other"
	batch.PublicationID = contract.PublicationID(batch.EdgeNodeID, batch.LedgerEpoch, 1, 1)
	batch.Records[0] = encodedTestMeasurement(
		t, "epoch-other", 1, "series-temperature-01", []float64{21.5}, 1_000,
	)

	if _, err := store.AcceptBatch(context.Background(), batch); !errors.Is(err, ErrEdgeNodeNotActive) {
		t.Fatalf("error = %v, want ErrEdgeNodeNotActive", err)
	}
	if store.testCount(t, "raw_records") != 0 || store.testCount(t, "accepted_cursors") != 0 {
		t.Fatal("unexpected epoch changed raw custody state")
	}
}
