package contract

import (
	"bytes"
	"encoding/json"
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

func TestDecodeBatchRejectsUnknownRecordFields(t *testing.T) {
	payload := bytes.Replace(
		fixture(t, "record-batch.json"),
		[]byte(`"device_time": null`),
		[]byte(`"device_time": null, "unexpected": true`),
		1,
	)
	if _, err := DecodeBatch(payload); err == nil {
		t.Fatal("record with fields outside the v1 family schema decoded successfully")
	}
}

func TestDecodeBatchRejectsUnknownRecordFamily(t *testing.T) {
	payload := []byte(`{
		"schema_version": 1,
		"edge_node_id": "edge-node-01",
		"ledger_epoch": "epoch-01",
		"publication_id": "edge-node-01:epoch-01:1:1",
		"cursor_start": 1,
		"cursor_end": 1,
		"records": [{
			"family": "future_family",
			"schema_version": 1,
			"epoch": "epoch-01",
			"pub_seq": 1
		}]
	}`)
	if _, err := DecodeBatch(payload); err == nil {
		t.Fatal("batch with unknown record family decoded successfully")
	}
}

func TestDecodeBatchRejectsMalformedKnownRecordFamilies(t *testing.T) {
	tests := []struct {
		name   string
		record string
	}{
		{
			name: "measurement missing payload",
			record: `{
				"family":"measurement","schema_version":1,
				"epoch":"epoch-01","pub_seq":1
			}`,
		},
		{
			name: "epoch annotation missing prior epoch",
			record: `{
				"family":"annotation","schema_version":1,
				"epoch":"epoch-01","pub_seq":1,"subtype":"epoch_start"
			}`,
		},
		{
			name: "commissioning smoke with malformed test id",
			record: `{
				"family":"commissioning_smoke","schema_version":1,
				"epoch":"epoch-01","pub_seq":1,"test_id":"smoke-invalid"
			}`,
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			payload := []byte(`{
				"schema_version":1,
				"edge_node_id":"edge-node-01",
				"ledger_epoch":"epoch-01",
				"publication_id":"edge-node-01:epoch-01:1:1",
				"cursor_start":1,
				"cursor_end":1,
				"records":[` + test.record + `]
			}`)
			if _, err := DecodeBatch(payload); err == nil {
				t.Fatal("malformed record decoded successfully")
			}
		})
	}
}

func TestRecordFamilyConformanceCases(t *testing.T) {
	var fixtureCases struct {
		Cases []struct {
			Name   string          `json:"name"`
			Valid  bool            `json:"valid"`
			Record json.RawMessage `json:"record"`
		} `json:"cases"`
	}
	if err := json.Unmarshal(fixture(t, "record-family-cases.json"), &fixtureCases); err != nil {
		t.Fatal(err)
	}
	for _, test := range fixtureCases.Cases {
		t.Run(test.Name, func(t *testing.T) {
			batch := RecordBatch{
				SchemaVersion: SchemaVersion,
				EdgeNodeID:    "edge-node-01",
				LedgerEpoch:   "epoch-01",
				PublicationID: "edge-node-01:epoch-01:1:1",
				CursorStart:   1,
				CursorEnd:     1,
				Records:       []json.RawMessage{test.Record},
			}
			payload, err := json.Marshal(batch)
			if err != nil {
				t.Fatal(err)
			}
			_, err = DecodeBatch(payload)
			if test.Valid && err != nil {
				t.Fatalf("valid record rejected: %v", err)
			}
			if !test.Valid && err == nil {
				t.Fatal("invalid record decoded successfully")
			}
		})
	}
}

func TestTransportPUBACKIsNotApplicationAck(t *testing.T) {
	if _, err := DecodeAcceptedThrough([]byte(`{"packet_id":7}`)); err == nil {
		t.Fatal("transport PUBACK decoded as application acknowledgement")
	}
}
