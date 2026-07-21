package mqttedge

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	edgestore "github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
)

type fakeStore struct {
	ack               contract.AcceptedThrough
	err               error
	batches           []contract.RecordBatch
	descriptors       []contract.DescriptorSnapshot
	descriptorErr     error
	activationResults []contract.ActivationResult
	activationErr     error
}

func (store *fakeStore) ApplyActivationResult(
	_ context.Context,
	result contract.ActivationResult,
) (edgestore.EdgeNodeActivation, error) {
	store.activationResults = append(store.activationResults, result)
	return edgestore.EdgeNodeActivation{State: edgestore.EdgeNodeActive}, store.activationErr
}

func (store *fakeStore) AcceptBatch(_ context.Context, batch contract.RecordBatch) (contract.AcceptedThrough, error) {
	store.batches = append(store.batches, batch)
	return store.ack, store.err
}

func (store *fakeStore) ApplyDescriptorSnapshot(_ context.Context, snapshot contract.DescriptorSnapshot) (edgestore.DescriptorApplyResult, error) {
	store.descriptors = append(store.descriptors, snapshot)
	return edgestore.DescriptorApplyResult{Status: edgestore.DescriptorApplied}, store.descriptorErr
}

func descriptorPayload(t *testing.T) []byte {
	t.Helper()
	payload, err := os.ReadFile(filepath.Join("..", "..", "..", "testdata", "egress", "v2", "descriptor-snapshot.json"))
	if err != nil {
		t.Fatal(err)
	}
	return payload
}

func TestProcessAppliesDescriptorWithoutPublishingAcknowledgement(t *testing.T) {
	store := &fakeStore{}
	published := false
	err := (Processor{Store: store}).Process(
		context.Background(),
		"iotkit/v1/edge-nodes/edge-node-01/descriptors",
		descriptorPayload(t),
		func(string, []byte) error {
			published = true
			return nil
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	if len(store.descriptors) != 1 || store.descriptors[0].DescriptorRevision != 5 {
		t.Fatalf("descriptors = %#v", store.descriptors)
	}
	if published {
		t.Fatal("descriptor processing published accepted-through")
	}
}

func TestProcessAppliesActivationResultWithoutPublishingCustodyAcknowledgement(t *testing.T) {
	store := &fakeStore{}
	result := contract.ActivationResult{
		SchemaVersion:            1,
		ActivationID:             "act-0123456789abcdef0123456789abcdef",
		EdgeID:                   "edge-0123456789abcdef0123456789abcdef",
		EdgeNodeID:               "edge-node-01",
		LedgerEpoch:              "epoch-01",
		Status:                   "applied",
		DiscardThroughReadingSeq: 4,
		FirstPublicationSeq:      1,
		AppliedAt:                20,
	}
	payload, err := result.Encode()
	if err != nil {
		t.Fatal(err)
	}
	published := false

	err = (Processor{Store: store}).Process(
		context.Background(),
		"iotkit/v1/edge-nodes/edge-node-01/activation/result",
		payload,
		func(string, []byte) error {
			published = true
			return nil
		},
	)

	if err != nil {
		t.Fatal(err)
	}
	if len(store.activationResults) != 1 || store.activationResults[0] != result {
		t.Fatalf("activation results = %#v", store.activationResults)
	}
	if published {
		t.Fatal("activation result processing published accepted-through")
	}
}

func TestProcessRejectsActivationTopicBodyMismatchWithoutApplying(t *testing.T) {
	store := &fakeStore{}
	result := contract.ActivationResult{
		SchemaVersion:            1,
		ActivationID:             "act-0123456789abcdef0123456789abcdef",
		EdgeID:                   "edge-0123456789abcdef0123456789abcdef",
		EdgeNodeID:               "edge-node-01",
		LedgerEpoch:              "epoch-01",
		Status:                   "applied",
		DiscardThroughReadingSeq: 0,
		FirstPublicationSeq:      1,
		AppliedAt:                20,
	}
	payload, err := result.Encode()
	if err != nil {
		t.Fatal(err)
	}

	err = (Processor{Store: store}).Process(
		context.Background(),
		"iotkit/v1/edge-nodes/edge-other/activation/result",
		payload,
		nil,
	)

	if err == nil {
		t.Fatal("activation topic/body mismatch was accepted")
	}
	if len(store.activationResults) != 0 {
		t.Fatalf("activation results = %#v", store.activationResults)
	}
}

func TestDescriptorFailureDoesNotPreventLaterRecordCustody(t *testing.T) {
	store := &fakeStore{
		descriptorErr: errors.New("descriptor database unavailable"),
		ack: contract.AcceptedThrough{
			SchemaVersion: 1, EdgeNodeID: "edge-node-01", LedgerEpoch: "epoch-01",
			PublicationID: "edge-node-01:epoch-01:1:1", AcceptedThrough: 1,
		},
	}
	processor := Processor{Store: store}
	if err := processor.Process(context.Background(), "iotkit/v1/edge-nodes/edge-node-01/descriptors", descriptorPayload(t), nil); err == nil {
		t.Fatal("descriptor failure was hidden")
	}
	published := 0
	if err := processor.Process(context.Background(), "iotkit/v1/edge-nodes/edge-node-01/records", validPayload(t), func(string, []byte) error {
		published++
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	if len(store.batches) != 1 || published != 1 {
		t.Fatalf("record custody after descriptor failure: batches=%d published=%d", len(store.batches), published)
	}
}

func payloadWithMarker(t *testing.T, marker string) []byte {
	t.Helper()
	record, err := json.Marshal(map[string]any{
		"family":         "measurement",
		"schema_version": 1,
		"epoch":          "epoch-01",
		"pub_seq":        1,
		"marker":         marker,
	})
	if err != nil {
		t.Fatal(err)
	}
	payload, err := json.Marshal(contract.RecordBatch{
		SchemaVersion: 1,
		EdgeNodeID:    "edge-node-01",
		LedgerEpoch:   "epoch-01",
		PublicationID: "edge-node-01:epoch-01:1:1",
		CursorStart:   1,
		CursorEnd:     1,
		Records:       []json.RawMessage{record},
	})
	if err != nil {
		t.Fatal(err)
	}
	return payload
}

func validPayload(t *testing.T) []byte {
	t.Helper()
	return payloadWithMarker(t, "original")
}

func activateRealStore(t *testing.T, archive *edgestore.Store) {
	t.Helper()
	snapshot, err := contract.DecodeDescriptorSnapshot(descriptorPayload(t))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	edgeNodes, err := archive.ListEdgeNodes(context.Background())
	if err != nil || len(edgeNodes) != 1 {
		t.Fatalf("edgeNodes = %#v, %v", edgeNodes, err)
	}
	requested, err := archive.RequestEdgeNodeActivation(
		context.Background(),
		edgeapp.LocalCLIActor(),
		edgeNodes[0].EdgeNodeRef,
		edgeapp.RevisionPrecondition{},
	)
	if err != nil {
		t.Fatal(err)
	}
	commands, err := archive.ListPendingActivationCommands(context.Background(), 10)
	if err != nil || len(commands) != 1 {
		t.Fatalf("commands = %#v, %v", commands, err)
	}
	request, err := contract.DecodeActivationRequest(commands[0].PayloadJSON)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ApplyActivationResult(context.Background(), contract.ActivationResult{
		SchemaVersion:            1,
		ActivationID:             requested.ActivationID,
		EdgeID:                   request.EdgeID,
		EdgeNodeID:               request.EdgeNodeID,
		LedgerEpoch:              request.ExpectedLedgerEpoch,
		Status:                   "applied",
		DiscardThroughReadingSeq: 0,
		FirstPublicationSeq:      1,
		AppliedAt:                request.IssuedAt + 1,
	}); err != nil {
		t.Fatal(err)
	}
}

func TestProcessPublishesAckOnlyAfterStoreSuccess(t *testing.T) {
	store := &fakeStore{ack: contract.AcceptedThrough{
		SchemaVersion:   1,
		EdgeNodeID:      "edge-node-01",
		LedgerEpoch:     "epoch-01",
		PublicationID:   "edge-node-01:epoch-01:1:1",
		AcceptedThrough: 1,
	}}
	processor := Processor{Store: store}
	var topic string
	var payload []byte
	err := processor.Process(context.Background(), "iotkit/v1/edge-nodes/edge-node-01/records", validPayload(t), func(gotTopic string, gotPayload []byte) error {
		topic = gotTopic
		payload = gotPayload
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if topic != "iotkit/v1/edge-nodes/edge-node-01/accepted-through" {
		t.Fatalf("topic = %q", topic)
	}
	if len(store.batches) != 1 || store.batches[0].EdgeNodeID != "edge-node-01" {
		t.Fatalf("stored batches = %+v", store.batches)
	}
	ack, err := contract.DecodeAcceptedThrough(payload)
	if err != nil {
		t.Fatal(err)
	}
	if ack.AcceptedThrough != 1 {
		t.Fatalf("accepted_through = %d", ack.AcceptedThrough)
	}
}

func TestProcessDoesNotPublishWhenStoreFails(t *testing.T) {
	processor := Processor{Store: &fakeStore{err: errors.New("injected commit failure")}}
	called := false
	err := processor.Process(context.Background(), "iotkit/v1/edge-nodes/edge-node-01/records", validPayload(t), func(string, []byte) error {
		called = true
		return nil
	})
	if err == nil {
		t.Fatal("Process succeeded despite store failure")
	}
	if called {
		t.Fatal("accepted-through published despite store failure")
	}
}

func TestProcessDoesNotPublishForRealStoreConflict(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "edge.db")
	archive, err := edgestore.Open(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = archive.Close() })
	activateRealStore(t, archive)

	processor := Processor{Store: archive}
	published := 0
	publish := func(string, []byte) error {
		published++
		return nil
	}
	topic := "iotkit/v1/edge-nodes/edge-node-01/records"
	if err := processor.Process(
		context.Background(),
		topic,
		payloadWithMarker(t, "original"),
		publish,
	); err != nil {
		t.Fatal(err)
	}
	if err := processor.Process(
		context.Background(),
		topic,
		payloadWithMarker(t, "changed"),
		publish,
	); !errors.Is(err, edgestore.ErrConflict) {
		t.Fatalf("conflicting Process error = %v, want ErrConflict", err)
	}
	if published != 1 {
		t.Fatalf("accepted-through publishes = %d, want only the initial success", published)
	}
	records, err := archive.ListRawRecords(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != 1 || !bytes.Contains(records[0].Record, []byte(`"marker":"original"`)) {
		t.Fatalf("stored record changed after conflict: %+v", records)
	}
}

func TestProcessDoesNotPublishWhenRealStoreTransactionFails(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "edge.db")
	archive, err := edgestore.Open(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = archive.Close() })
	activateRealStore(t, archive)

	injector, err := sql.Open("sqlite", dbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = injector.Close() })
	if _, err := injector.Exec(`
		CREATE TRIGGER fail_cursor BEFORE INSERT ON accepted_cursors
		BEGIN SELECT RAISE(ABORT, 'injected cursor failure'); END;
	`); err != nil {
		t.Fatal(err)
	}

	published := false
	err = (Processor{Store: archive}).Process(
		context.Background(),
		"iotkit/v1/edge-nodes/edge-node-01/records",
		validPayload(t),
		func(string, []byte) error {
			published = true
			return nil
		},
	)
	if err == nil {
		t.Fatal("Process succeeded despite injected transaction failure")
	}
	if published {
		t.Fatal("accepted-through published despite transaction failure")
	}
	records, err := archive.ListRawRecords(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != 0 {
		t.Fatalf("raw records survived failed transaction: %+v", records)
	}
	var cursors int
	if err := injector.QueryRow("SELECT count(*) FROM accepted_cursors").Scan(&cursors); err != nil {
		t.Fatal(err)
	}
	if cursors != 0 {
		t.Fatalf("cursor rows survived failed transaction: %d", cursors)
	}
}

func TestProcessRejectsTopicBodyEdgeNodeMismatchWithoutStoreOrAck(t *testing.T) {
	store := &fakeStore{}
	processor := Processor{Store: store}
	called := false
	err := processor.Process(context.Background(), "iotkit/v1/edge-nodes/edge-node-other/records", validPayload(t), func(string, []byte) error {
		called = true
		return nil
	})
	if err == nil {
		t.Fatal("Edge Node mismatch accepted")
	}
	if called {
		t.Fatal("ack published for Edge Node mismatch")
	}
	if len(store.batches) != 0 {
		t.Fatalf("store called for Edge Node mismatch: %+v", store.batches)
	}
}

func TestProcessRejectsLegacyGatewayTopicWithoutStoreOrAck(t *testing.T) {
	store := &fakeStore{}
	processor := Processor{Store: store}
	called := false
	err := processor.Process(context.Background(), "iotkit/v1/gateways/edge-node-01/records", validPayload(t), func(string, []byte) error {
		called = true
		return nil
	})
	if err == nil {
		t.Fatal("legacy gateway topic accepted")
	}
	if called {
		t.Fatal("ack published for legacy gateway topic")
	}
	if len(store.batches) != 0 {
		t.Fatalf("store called for legacy gateway topic: %+v", store.batches)
	}
}

func TestProcessRejectsMixedEdgeAndGatewayIdentityWithoutStoreOrAck(t *testing.T) {
	store := &fakeStore{}
	processor := Processor{Store: store}
	payload := []byte(`{
		"schema_version": 1,
		"edge_node_id": "edge-node-01",
		"gateway_identity": "gateway-01",
		"ledger_epoch": "epoch-01",
		"publication_id": "edge-node-01:epoch-01:1:1",
		"cursor_start": 1,
		"cursor_end": 1,
		"records": [{"schema_version": 1, "epoch": "epoch-01", "pub_seq": 1}]
	}`)
	published := false
	err := processor.Process(context.Background(), "iotkit/v1/edge-nodes/edge-node-01/records", payload, func(string, []byte) error {
		published = true
		return nil
	})
	if err == nil {
		t.Fatal("mixed edge_node_id and gateway_identity payload accepted")
	}
	if len(store.batches) != 0 {
		t.Fatalf("store called for mixed identity payload: %+v", store.batches)
	}
	if published {
		t.Fatal("ack published for mixed identity payload")
	}
}

func TestRecordsTopicEdgeNodeAcceptsOnlyEdgeNodeRecordsTopic(t *testing.T) {
	tests := []struct {
		name    string
		topic   string
		wantID  string
		wantErr bool
	}{
		{name: "Edge Node", topic: "iotkit/v1/edge-nodes/edge-node-01/records", wantID: "edge-node-01"},
		{name: "gateway", topic: "iotkit/v1/gateways/edge-node-01/records", wantErr: true},
		{name: "empty ID", topic: "iotkit/v1/edge-nodes//records", wantErr: true},
		{name: "wildcard", topic: "iotkit/v1/edge-nodes/+/records", wantErr: true},
		{name: "extra segment", topic: "iotkit/v1/edge-nodes/edge-node-01/records/extra", wantErr: true},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			gotID, err := recordsTopicEdgeNode(test.topic)
			if (err != nil) != test.wantErr {
				t.Fatalf("recordsTopicEdgeNode(%q) error = %v", test.topic, err)
			}
			if gotID != test.wantID {
				t.Fatalf("recordsTopicEdgeNode(%q) = %q, want %q", test.topic, gotID, test.wantID)
			}
		})
	}
}
