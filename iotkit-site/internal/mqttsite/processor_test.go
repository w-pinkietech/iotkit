package mqttsite

import (
	"context"
	"encoding/json"
	"errors"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/contract"
)

type fakeStore struct {
	ack     contract.AcceptedThrough
	err     error
	batches []contract.RecordBatch
}

func (store *fakeStore) AcceptBatch(_ context.Context, batch contract.RecordBatch) (contract.AcceptedThrough, error) {
	store.batches = append(store.batches, batch)
	return store.ack, store.err
}

func validPayload(t *testing.T) []byte {
	t.Helper()
	record := json.RawMessage(`{"family":"measurement","schema_version":1,"epoch":"epoch-01","pub_seq":1}`)
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

func TestProcessRejectsTopicBodyEdgeNodeMismatchWithoutStoreOrAck(t *testing.T) {
	store := &fakeStore{}
	processor := Processor{Store: store}
	called := false
	err := processor.Process(context.Background(), "iotkit/v1/edge-nodes/edge-node-other/records", validPayload(t), func(string, []byte) error {
		called = true
		return nil
	})
	if err == nil {
		t.Fatal("edge node mismatch accepted")
	}
	if called {
		t.Fatal("ack published for edge node mismatch")
	}
	if len(store.batches) != 0 {
		t.Fatalf("store called for edge node mismatch: %+v", store.batches)
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

func TestRecordsTopicEdgeNodeAcceptsOnlyEdgeNodeRecordsTopic(t *testing.T) {
	tests := []struct {
		name    string
		topic   string
		wantID  string
		wantErr bool
	}{
		{name: "edge node", topic: "iotkit/v1/edge-nodes/edge-node-01/records", wantID: "edge-node-01"},
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
