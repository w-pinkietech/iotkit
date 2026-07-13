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
	if batch.EdgeNodeID != "edge-node-01" || batch.CursorStart != 1 || batch.CursorEnd != 1 {
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
	if ack.AcceptedThrough != 1 || ack.PublicationID != "edge-node-01:epoch-01:1:1" {
		t.Fatalf("unexpected ack: %+v", ack)
	}
}

func TestBatchWithOnlyLegacyGatewayIdentityIsRejected(t *testing.T) {
	payload := []byte(`{
		"schema_version": 1,
		"gateway_identity": "gateway-01",
		"ledger_epoch": "epoch-01",
		"publication_id": "gateway-01:epoch-01:1:1",
		"cursor_start": 1,
		"cursor_end": 1,
		"records": [{"schema_version": 1, "epoch": "epoch-01", "pub_seq": 1}]
	}`)
	if _, err := DecodeBatch(payload); err == nil {
		t.Fatal("legacy gateway_identity-only batch decoded successfully")
	}
}

func TestBatchWithEdgeNodeIDAndLegacyGatewayIdentityIsRejected(t *testing.T) {
	payload := []byte(`{
		"schema_version": 1,
		"edge_node_id": "edge-node-01",
		"gateway_identity": "gateway-01",
		"ledger_epoch": "epoch-01",
		"publication_id": "edge-node-01:epoch-01:1:1",
		"cursor_start": 1,
		"cursor_end": 1,
		"records": [{"schema_version": 1, "epoch": "epoch-01", "pub_seq": 1, "family": "measurement"}]
	}`)
	if _, err := DecodeBatch(payload); err == nil {
		t.Fatal("batch with edge_node_id and legacy gateway_identity decoded successfully")
	}
}

func TestAcceptedThroughWithEdgeNodeIDAndLegacyGatewayIdentityIsRejected(t *testing.T) {
	payload := []byte(`{
		"schema_version": 1,
		"edge_node_id": "edge-node-01",
		"gateway_identity": "gateway-01",
		"ledger_epoch": "epoch-01",
		"publication_id": "edge-node-01:epoch-01:1:1",
		"accepted_through": 1
	}`)
	if _, err := DecodeAcceptedThrough(payload); err == nil {
		t.Fatal("accepted-through with edge_node_id and legacy gateway_identity decoded successfully")
	}
}

func TestRecordHeaderAllowsAdditionalRecordFields(t *testing.T) {
	payload := []byte(`{
		"schema_version": 1,
		"edge_node_id": "edge-node-01",
		"ledger_epoch": "epoch-01",
		"publication_id": "edge-node-01:epoch-01:1:1",
		"cursor_start": 1,
		"cursor_end": 1,
		"records": [{"schema_version": 1, "epoch": "epoch-01", "pub_seq": 1, "family": "measurement", "value": 23.5}]
	}`)
	if _, err := DecodeBatch(payload); err != nil {
		t.Fatalf("record fields outside the header were rejected: %v", err)
	}
}

func TestTransportPUBACKIsNotApplicationAck(t *testing.T) {
	if _, err := DecodeAcceptedThrough([]byte(`{"packet_id":7}`)); err == nil {
		t.Fatal("transport PUBACK decoded as application acknowledgement")
	}
}
