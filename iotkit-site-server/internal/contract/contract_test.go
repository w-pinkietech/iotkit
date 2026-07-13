package contract

import (
	"os"
	"path/filepath"
	"testing"
)

func fixture(t *testing.T, name string) []byte {
	t.Helper()
	path := filepath.Join("..", "..", "..", "testdata", "egress", "v1", name)
	payload, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return payload
}

func TestDecodeBatchFixture(t *testing.T) {
	batch, err := DecodeBatch(fixture(t, "record-batch.json"))
	if err != nil {
		t.Fatal(err)
	}
	if batch.GatewayIdentity != "gateway-01" || batch.CursorStart != 1 || batch.CursorEnd != 1 {
		t.Fatalf("unexpected batch: %+v", batch)
	}
	if len(batch.Records) != 1 {
		t.Fatalf("record count = %d, want 1", len(batch.Records))
	}
}

func TestDecodeAcceptedThroughFixture(t *testing.T) {
	ack, err := DecodeAcceptedThrough(fixture(t, "accepted-through.json"))
	if err != nil {
		t.Fatal(err)
	}
	if ack.AcceptedThrough != 1 || ack.PublicationID != "gateway-01:epoch-01:1:1" {
		t.Fatalf("unexpected ack: %+v", ack)
	}
}

func TestTransportPUBACKIsNotApplicationAck(t *testing.T) {
	if _, err := DecodeAcceptedThrough([]byte(`{"packet_id":7}`)); err == nil {
		t.Fatal("transport PUBACK decoded as application acknowledgement")
	}
}
