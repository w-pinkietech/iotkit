package store

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"path/filepath"
	"strings"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/contract"
)

func testBatch(t *testing.T) contract.RecordBatch {
	t.Helper()
	record, err := json.Marshal(map[string]any{
		"family":         "measurement",
		"schema_version": 1,
		"epoch":          "epoch-01",
		"pub_seq":        1,
		"series_key":     "series-temperature-01",
		"values":         []float64{21.5},
	})
	if err != nil {
		t.Fatal(err)
	}
	return contract.RecordBatch{
		SchemaVersion: 1,
		EdgeNodeID:    "edge-node-01",
		LedgerEpoch:   "epoch-01",
		PublicationID: "edge-node-01:epoch-01:1:1",
		CursorStart:   1,
		CursorEnd:     1,
		Records:       []json.RawMessage{record},
	}
}

func TestOpenCreatesEdgeNodeIdentitySchema(t *testing.T) {
	store := openTestStore(t)

	rawRecordPK := store.testPrimaryKey(t, "raw_records")
	if rawRecordPK["edge_node_id"] != 1 || rawRecordPK["ledger_epoch"] != 2 || rawRecordPK["pub_seq"] != 3 {
		t.Fatalf("raw_records primary key = %#v", rawRecordPK)
	}
	if _, found := rawRecordPK["gateway_identity"]; found {
		t.Fatalf("raw_records retains gateway_identity: %#v", rawRecordPK)
	}

	acceptedCursorPK := store.testPrimaryKey(t, "accepted_cursors")
	if acceptedCursorPK["edge_node_id"] != 1 || acceptedCursorPK["ledger_epoch"] != 2 {
		t.Fatalf("accepted_cursors primary key = %#v", acceptedCursorPK)
	}
	if _, found := acceptedCursorPK["gateway_identity"]; found {
		t.Fatalf("accepted_cursors retains gateway_identity: %#v", acceptedCursorPK)
	}
}

func TestOpenRejectsLegacyGatewayIdentitySchemaBeforeCreatingTables(t *testing.T) {
	tests := []struct {
		name            string
		legacySchema    string
		unexpectedTable string
	}{
		{
			name: "raw records",
			legacySchema: `
				CREATE TABLE raw_records (
					gateway_identity TEXT NOT NULL,
					ledger_epoch TEXT NOT NULL,
					pub_seq INTEGER NOT NULL,
					PRIMARY KEY (gateway_identity, ledger_epoch, pub_seq)
				)
			`,
			unexpectedTable: "accepted_cursors",
		},
		{
			name: "accepted cursors",
			legacySchema: `
				CREATE TABLE accepted_cursors (
					gateway_identity TEXT NOT NULL,
					ledger_epoch TEXT NOT NULL,
					accepted_through INTEGER NOT NULL,
					PRIMARY KEY (gateway_identity, ledger_epoch)
				)
			`,
			unexpectedTable: "raw_records",
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "legacy-site.db")
			db, err := sql.Open("sqlite", path)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := db.Exec(test.legacySchema); err != nil {
				_ = db.Close()
				t.Fatal(err)
			}
			if err := db.Close(); err != nil {
				t.Fatal(err)
			}

			store, err := Open(path)
			if store != nil {
				_ = store.Close()
			}
			if err == nil || !strings.Contains(err.Error(), "unsupported pre-release Site database; recreate it") {
				t.Fatalf("Open error = %v, want unsupported pre-release Site database", err)
			}

			db, err = sql.Open("sqlite", path)
			if err != nil {
				t.Fatal(err)
			}
			defer db.Close()
			var unexpectedTables int
			if err := db.QueryRow(`
				SELECT count(*) FROM sqlite_master
				WHERE type = 'table' AND name = ?
			`, test.unexpectedTable).Scan(&unexpectedTables); err != nil {
				t.Fatal(err)
			}
			if unexpectedTables != 0 {
				t.Fatalf("Open created %s before rejecting legacy database", test.unexpectedTable)
			}
		})
	}
}

func openTestStore(t *testing.T) *Store {
	t.Helper()
	store, err := Open(filepath.Join(t.TempDir(), "site.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = store.Close() })
	return store
}

func TestAcceptBatchCommitsRawRecordAndCursor(t *testing.T) {
	store := openTestStore(t)
	ack, err := store.AcceptBatch(context.Background(), testBatch(t))
	if err != nil {
		t.Fatal(err)
	}
	if ack.AcceptedThrough != 1 {
		t.Fatalf("accepted_through = %d, want 1", ack.AcceptedThrough)
	}
	if got := store.testCount(t, "raw_records"); got != 1 {
		t.Fatalf("raw record count = %d, want 1", got)
	}
	if got := store.testCursor(t); got != 1 {
		t.Fatalf("cursor = %d, want 1", got)
	}
}

func TestExactReplayIsIdempotent(t *testing.T) {
	store := openTestStore(t)
	batch := testBatch(t)
	if _, err := store.AcceptBatch(context.Background(), batch); err != nil {
		t.Fatal(err)
	}
	if _, err := store.AcceptBatch(context.Background(), batch); err != nil {
		t.Fatal(err)
	}
	if got := store.testCount(t, "raw_records"); got != 1 {
		t.Fatalf("raw record count = %d, want 1", got)
	}
	if got := store.testCursor(t); got != 1 {
		t.Fatalf("cursor after exact replay = %d, want 1", got)
	}
}

func TestConflictingReplayDoesNotAdvanceCursor(t *testing.T) {
	store := openTestStore(t)
	batch := testBatch(t)
	if _, err := store.AcceptBatch(context.Background(), batch); err != nil {
		t.Fatal(err)
	}
	before, err := store.ListRawRecords(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(before) != 1 {
		t.Fatalf("records before conflict = %d, want 1", len(before))
	}
	batch.Records[0] = json.RawMessage(`{"family":"measurement","schema_version":1,"epoch":"epoch-01","pub_seq":1,"series_key":"series-temperature-01","values":[99]}`)
	if _, err := store.AcceptBatch(context.Background(), batch); !errors.Is(err, ErrConflict) {
		t.Fatalf("error = %v, want ErrConflict", err)
	}
	after, err := store.ListRawRecords(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(after) != 1 || !bytes.Equal(after[0].Record, before[0].Record) {
		t.Fatalf("conflict replaced original record: before=%s after=%v", before[0].Record, after)
	}
	if got := store.testCursor(t); got != 1 {
		t.Fatalf("cursor = %d, want 1", got)
	}
}

func TestCursorWriteFailureRollsBackRawInsert(t *testing.T) {
	store := openTestStore(t)
	if _, err := store.db.Exec(`
		CREATE TRIGGER fail_cursor BEFORE INSERT ON accepted_cursors
		BEGIN SELECT RAISE(ABORT, 'injected cursor failure'); END;
	`); err != nil {
		t.Fatal(err)
	}
	if _, err := store.AcceptBatch(context.Background(), testBatch(t)); err == nil {
		t.Fatal("AcceptBatch succeeded despite cursor failure")
	}
	if got := store.testCount(t, "raw_records"); got != 0 {
		t.Fatalf("raw record count = %d, want rollback to 0", got)
	}
	if got := store.testCount(t, "accepted_cursors"); got != 0 {
		t.Fatalf("cursor count = %d, want rollback to 0", got)
	}
}

func TestListRawRecordsReturnsCommittedJSON(t *testing.T) {
	store := openTestStore(t)
	if _, err := store.AcceptBatch(context.Background(), testBatch(t)); err != nil {
		t.Fatal(err)
	}
	records, err := store.ListRawRecords(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != 1 || records[0].EdgeNodeID != "edge-node-01" || records[0].PubSeq != 1 {
		t.Fatalf("unexpected records: %+v", records)
	}
	if !json.Valid(records[0].Record) {
		t.Fatalf("stored record is invalid JSON: %s", records[0].Record)
	}
	encoded, err := json.Marshal(records[0])
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(encoded), `"edge_node_id":"edge-node-01"`) || strings.Contains(string(encoded), "gateway_identity") {
		t.Fatalf("raw record JSON uses unexpected identity field: %s", encoded)
	}
}

func (store *Store) testCount(t *testing.T, table string) int {
	t.Helper()
	var count int
	if err := store.db.QueryRow("SELECT count(*) FROM " + table).Scan(&count); err != nil {
		t.Fatal(err)
	}
	return count
}

func (store *Store) testCursor(t *testing.T) int64 {
	t.Helper()
	var cursor int64
	if err := store.db.QueryRow(`
		SELECT accepted_through FROM accepted_cursors
		WHERE edge_node_id = 'edge-node-01' AND ledger_epoch = 'epoch-01'
	`).Scan(&cursor); err != nil {
		t.Fatal(err)
	}
	return cursor
}

func (store *Store) testPrimaryKey(t *testing.T, table string) map[string]int {
	t.Helper()
	rows, err := store.db.Query("PRAGMA table_info(" + table + ")")
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()

	primaryKey := make(map[string]int)
	for rows.Next() {
		var cid, notNull, keyPosition int
		var name, columnType string
		var defaultValue any
		if err := rows.Scan(&cid, &name, &columnType, &notNull, &defaultValue, &keyPosition); err != nil {
			t.Fatal(err)
		}
		if keyPosition > 0 {
			primaryKey[name] = keyPosition
		}
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	return primaryKey
}
