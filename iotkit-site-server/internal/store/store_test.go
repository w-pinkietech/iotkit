package store

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site-server/internal/contract"
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
		SchemaVersion:   1,
		GatewayIdentity: "gateway-01",
		LedgerEpoch:     "epoch-01",
		PublicationID:   "gateway-01:epoch-01:1:1",
		CursorStart:     1,
		CursorEnd:       1,
		Records:         []json.RawMessage{record},
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
}

func TestConflictingReplayDoesNotAdvanceCursor(t *testing.T) {
	store := openTestStore(t)
	batch := testBatch(t)
	if _, err := store.AcceptBatch(context.Background(), batch); err != nil {
		t.Fatal(err)
	}
	batch.Records[0] = json.RawMessage(`{"family":"measurement","schema_version":1,"epoch":"epoch-01","pub_seq":1,"series_key":"series-temperature-01","values":[99]}`)
	if _, err := store.AcceptBatch(context.Background(), batch); !errors.Is(err, ErrConflict) {
		t.Fatalf("error = %v, want ErrConflict", err)
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
	if len(records) != 1 || records[0].PubSeq != 1 {
		t.Fatalf("unexpected records: %+v", records)
	}
	if !json.Valid(records[0].Record) {
		t.Fatalf("stored record is invalid JSON: %s", records[0].Record)
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
		WHERE gateway_identity = 'gateway-01' AND ledger_epoch = 'epoch-01'
	`).Scan(&cursor); err != nil {
		t.Fatal(err)
	}
	return cursor
}
