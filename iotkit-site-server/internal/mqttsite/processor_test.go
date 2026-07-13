package mqttsite

import (
	"context"
	"encoding/json"
	"errors"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site-server/internal/contract"
)

type fakeStore struct {
	ack contract.AcceptedThrough
	err error
}

func (store fakeStore) AcceptBatch(context.Context, contract.RecordBatch) (contract.AcceptedThrough, error) {
	return store.ack, store.err
}

func validPayload(t *testing.T) []byte {
	t.Helper()
	record := json.RawMessage(`{"family":"measurement","schema_version":1,"epoch":"epoch-01","pub_seq":1}`)
	payload, err := json.Marshal(contract.RecordBatch{
		SchemaVersion:   1,
		GatewayIdentity: "gateway-01",
		LedgerEpoch:     "epoch-01",
		PublicationID:   "gateway-01:epoch-01:1:1",
		CursorStart:     1,
		CursorEnd:       1,
		Records:         []json.RawMessage{record},
	})
	if err != nil {
		t.Fatal(err)
	}
	return payload
}

func TestProcessPublishesAckOnlyAfterStoreSuccess(t *testing.T) {
	processor := Processor{Store: fakeStore{ack: contract.AcceptedThrough{
		SchemaVersion:   1,
		GatewayIdentity: "gateway-01",
		LedgerEpoch:     "epoch-01",
		PublicationID:   "gateway-01:epoch-01:1:1",
		AcceptedThrough: 1,
	}}}
	var topic string
	var payload []byte
	err := processor.Process(context.Background(), "iotkit/v1/gateways/gateway-01/records", validPayload(t), func(gotTopic string, gotPayload []byte) error {
		topic = gotTopic
		payload = gotPayload
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if topic != "iotkit/v1/gateways/gateway-01/accepted-through" {
		t.Fatalf("topic = %q", topic)
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
	processor := Processor{Store: fakeStore{err: errors.New("injected commit failure")}}
	called := false
	err := processor.Process(context.Background(), "iotkit/v1/gateways/gateway-01/records", validPayload(t), func(string, []byte) error {
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

func TestProcessRejectsTopicBodyGatewayMismatch(t *testing.T) {
	processor := Processor{Store: fakeStore{}}
	called := false
	err := processor.Process(context.Background(), "iotkit/v1/gateways/gateway-other/records", validPayload(t), func(string, []byte) error {
		called = true
		return nil
	})
	if err == nil {
		t.Fatal("gateway mismatch accepted")
	}
	if called {
		t.Fatal("ack published for gateway mismatch")
	}
}
