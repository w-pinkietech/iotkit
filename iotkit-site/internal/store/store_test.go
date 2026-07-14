package store

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantic"
)

const contactSeries = "subject:contact_state:na:primary"

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

func TestPutSemanticMappingCapturesEveryExistingEpochCursor(t *testing.T) {
	store := openTestStore(t)
	acceptEpoch(t, store, "edge-node-01", "epoch-a", 1, 0)
	acceptEpoch(t, store, "edge-node-01", "epoch-b", 1, 1)
	acceptEpoch(t, store, "edge-node-02", "epoch-other-edge", 1, 1)

	mapping, err := store.PutSemanticMapping(context.Background(), semantic.MappingSpec{
		EdgeNodeID:  "edge-node-01",
		SeriesKey:   contactSeries,
		Meaning:     semantic.MeaningProductionPulse,
		TriggerMode: semantic.TriggerActiveSample,
		ActiveValue: 1,
	})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(mapping.ID, "sm-") || len(mapping.ID) != len("sm-")+32 {
		t.Fatalf("mapping ID = %q, want sm- followed by 128-bit hex", mapping.ID)
	}
	if got := store.testMappingStarts(t, mapping.ID, mapping.Revision); !reflect.DeepEqual(got, map[string]int64{
		"epoch-a": 1,
		"epoch-b": 1,
	}) {
		t.Fatalf("starts = %#v", got)
	}
	if got := store.testMappingEnds(t, mapping.ID, mapping.Revision); len(got) != 0 {
		t.Fatalf("initial mapping ends = %#v, want none", got)
	}
}

func TestPutSemanticMappingIDGenerationFailureDoesNotWriteMapping(t *testing.T) {
	store := openTestStore(t)
	generationErr := errors.New("injected semantic mapping ID generation failure")
	originalGenerator := newSemanticMappingID
	newSemanticMappingID = func() (string, error) {
		return "", generationErr
	}
	t.Cleanup(func() { newSemanticMappingID = originalGenerator })

	if _, err := store.PutSemanticMapping(context.Background(), semantic.MappingSpec{
		EdgeNodeID:  "edge-node-01",
		SeriesKey:   contactSeries,
		Meaning:     semantic.MeaningProductionPulse,
		TriggerMode: semantic.TriggerActiveSample,
		ActiveValue: 1,
	}); !errors.Is(err, generationErr) {
		t.Fatalf("error = %v, want %v", err, generationErr)
	}
	if got := store.testCount(t, "semantic_mappings"); got != 0 {
		t.Fatalf("semantic mapping count = %d, want 0", got)
	}
}

func TestPutSemanticMappingCreatesFutureOnlyRevision(t *testing.T) {
	store := openTestStore(t)
	first, err := store.PutSemanticMapping(context.Background(), semantic.MappingSpec{
		EdgeNodeID:  "edge-node-01",
		SeriesKey:   contactSeries,
		Meaning:     semantic.MeaningProductionPulse,
		TriggerMode: semantic.TriggerActiveSample,
		ActiveValue: 1,
	})
	if err != nil {
		t.Fatal(err)
	}
	second, err := store.PutSemanticMapping(context.Background(), semantic.MappingSpec{
		EdgeNodeID:  "edge-node-01",
		SeriesKey:   contactSeries,
		Meaning:     semantic.MeaningProductionPulse,
		TriggerMode: semantic.TriggerActiveEdge,
		ActiveValue: 0,
	})
	if err != nil {
		t.Fatal(err)
	}
	if second.ID != first.ID || first.Revision != 1 || second.Revision != 2 {
		t.Fatalf("revisions = %#v then %#v", first, second)
	}

	mappings, err := store.ListSemanticMappings(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(mappings) != 2 {
		t.Fatalf("mapping count = %d, want 2", len(mappings))
	}
	if mappings[0].Active || !mappings[1].Active {
		t.Fatalf("active revisions = %t then %t", mappings[0].Active, mappings[1].Active)
	}
	if mappings[1].TriggerMode != semantic.TriggerActiveEdge || mappings[1].ActiveValue != 0 {
		t.Fatalf("second revision = %#v", mappings[1])
	}
}

func TestPutSemanticMappingRevisionClosesOldCursorBoundary(t *testing.T) {
	store := openTestStore(t)
	acceptEpoch(t, store, "edge-node-01", "epoch-a", 1, 0)
	first, err := store.PutSemanticMapping(context.Background(), semantic.MappingSpec{
		EdgeNodeID:  "edge-node-01",
		SeriesKey:   contactSeries,
		Meaning:     semantic.MeaningProductionPulse,
		TriggerMode: semantic.TriggerActiveSample,
		ActiveValue: 1,
	})
	if err != nil {
		t.Fatal(err)
	}
	if got := store.testMappingStarts(t, first.ID, first.Revision); !reflect.DeepEqual(got, map[string]int64{"epoch-a": 1}) {
		t.Fatalf("first revision starts = %#v", got)
	}
	acceptEpoch(t, store, "edge-node-01", "epoch-a", 2, 1)
	acceptEpoch(t, store, "edge-node-02", "epoch-other-edge", 1, 1)

	second, err := store.PutSemanticMapping(context.Background(), semantic.MappingSpec{
		EdgeNodeID:  "edge-node-01",
		SeriesKey:   contactSeries,
		Meaning:     semantic.MeaningProductionPulse,
		TriggerMode: semantic.TriggerActiveEdge,
		ActiveValue: 1,
	})
	if err != nil {
		t.Fatal(err)
	}
	wantBoundary := map[string]int64{"epoch-a": 2}
	if got := store.testMappingEnds(t, first.ID, first.Revision); !reflect.DeepEqual(got, wantBoundary) {
		t.Fatalf("first revision ends = %#v, want %#v", got, wantBoundary)
	}
	if got := store.testMappingStarts(t, second.ID, second.Revision); !reflect.DeepEqual(got, wantBoundary) {
		t.Fatalf("second revision starts = %#v, want %#v", got, wantBoundary)
	}
}

func TestPutSemanticMappingRollsBackRevisionWhenStartSnapshotFails(t *testing.T) {
	store := openTestStore(t)
	acceptEpoch(t, store, "edge-node-01", "epoch-a", 1, 1)
	first, err := store.PutSemanticMapping(context.Background(), semantic.MappingSpec{
		EdgeNodeID:  "edge-node-01",
		SeriesKey:   contactSeries,
		Meaning:     semantic.MeaningProductionPulse,
		TriggerMode: semantic.TriggerActiveSample,
		ActiveValue: 1,
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.db.Exec(`
		CREATE TRIGGER fail_mapping_start BEFORE INSERT ON semantic_mapping_starts
		BEGIN SELECT RAISE(ABORT, 'injected mapping start failure'); END;
	`); err != nil {
		t.Fatal(err)
	}
	if _, err := store.PutSemanticMapping(context.Background(), semantic.MappingSpec{
		EdgeNodeID:  "edge-node-01",
		SeriesKey:   contactSeries,
		Meaning:     semantic.MeaningProductionPulse,
		TriggerMode: semantic.TriggerActiveEdge,
		ActiveValue: 1,
	}); err == nil {
		t.Fatal("PutSemanticMapping succeeded despite snapshot failure")
	}
	mappings, err := store.ListSemanticMappings(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(mappings) != 1 || mappings[0].ID != first.ID || !mappings[0].Active {
		t.Fatalf("mappings after rollback = %#v", mappings)
	}
	if got := store.testMappingEnds(t, first.ID, first.Revision); len(got) != 0 {
		t.Fatalf("mapping ends after rollback = %#v, want none", got)
	}
}

func TestProjectSemanticEventsIsFutureOnlyAndIdempotent(t *testing.T) {
	store := openTestStore(t)
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 1, []float64{1})
	mapping := putSemanticMapping(t, store, semantic.TriggerActiveSample, 1)
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 2,
		[]float64{1}, []float64{0}, []float64{1})

	if _, err := store.ProjectSemanticEvents(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
	first := listSemanticEvents(t, store)
	if _, err := store.ProjectSemanticEvents(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
	second := listSemanticEvents(t, store)

	if !reflect.DeepEqual(first, second) {
		t.Fatalf("events changed after idempotent projection: first=%#v second=%#v", first, second)
	}
	if len(first) != 2 {
		t.Fatalf("events = %#v, want two", first)
	}
	for index, event := range first {
		if event.MappingID != mapping.ID || event.MappingRevision != mapping.Revision {
			t.Fatalf("event mapping = %#v, want %s revision %d", event, mapping.ID, mapping.Revision)
		}
		if event.EventSequence != int64(index+1) {
			t.Fatalf("event sequence = %d, want %d", event.EventSequence, index+1)
		}
		if len(event.EventID) != 64 || strings.ToLower(event.EventID) != event.EventID {
			t.Fatalf("event ID = %q, want lowercase SHA-256 hex", event.EventID)
		}
	}
	if first[0].SourcePubSeq != 2 || first[1].SourcePubSeq != 4 {
		t.Fatalf("source pub seqs = %d, %d; want 2, 4", first[0].SourcePubSeq, first[1].SourcePubSeq)
	}
}

func TestProjectSemanticEventsClosesOldRevisionWithoutDroppingLaggedRaw(t *testing.T) {
	store := openTestStore(t)
	first := putSemanticMapping(t, store, semantic.TriggerActiveSample, 1)
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 1, []float64{1})
	second := putSemanticMapping(t, store, semantic.TriggerActiveEdge, 1)
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 2, []float64{0}, []float64{1})

	if _, err := store.ProjectSemanticEvents(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
	events := listSemanticEvents(t, store)
	if len(events) != 2 {
		t.Fatalf("events = %#v, want two", events)
	}
	if events[0].MappingRevision != first.Revision || events[0].SourcePubSeq != 1 {
		t.Fatalf("old revision event = %#v, want revision %d pub_seq 1", events[0], first.Revision)
	}
	if events[1].MappingRevision != second.Revision || events[1].SourcePubSeq != 3 {
		t.Fatalf("new revision event = %#v, want revision %d pub_seq 3", events[1], second.Revision)
	}
}

func TestProjectSemanticEventsDoesNotExtendInactiveRevisionIntoNewEpoch(t *testing.T) {
	store := openTestStore(t)
	first := putSemanticMapping(t, store, semantic.TriggerActiveSample, 1)
	second := putSemanticMapping(t, store, semantic.TriggerActiveSample, 1)
	acceptContactBatch(t, store, "edge-node-01", "epoch-new", 1, []float64{1})

	if _, err := store.ProjectSemanticEvents(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
	events := listSemanticEvents(t, store)
	if len(events) != 1 || events[0].MappingRevision != second.Revision || events[0].SourcePubSeq != 1 {
		t.Fatalf("events = %#v, want only new revision %d", events, second.Revision)
	}
	if events[0].MappingRevision == first.Revision {
		t.Fatalf("new epoch was projected under inactive revision %d", first.Revision)
	}
}

func TestProjectSemanticEventsActiveEdgeStoresFirstSampleAsBaseline(t *testing.T) {
	store := openTestStore(t)
	putSemanticMapping(t, store, semantic.TriggerActiveEdge, 1)
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 1,
		[]float64{1}, []float64{1}, []float64{0}, []float64{1})

	if _, err := store.ProjectSemanticEvents(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
	events := listSemanticEvents(t, store)
	if len(events) != 1 || events[0].SourcePubSeq != 4 {
		t.Fatalf("events = %#v, want only transition at pub_seq 4", events)
	}
}

func TestProjectSemanticEventsOrdersInputsDeterministically(t *testing.T) {
	store := openTestStore(t)
	putSemanticMapping(t, store, semantic.TriggerActiveSample, 1)
	acceptContactBatch(t, store, "edge-node-01", "epoch-b", 1, []float64{1})
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 1, []float64{1})
	if _, err := store.db.Exec(`UPDATE raw_records SET received_at = 100`); err != nil {
		t.Fatal(err)
	}

	if _, err := store.ProjectSemanticEvents(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
	events := listSemanticEvents(t, store)
	if len(events) != 2 || events[0].LedgerEpoch != "epoch-a" || events[1].LedgerEpoch != "epoch-b" {
		t.Fatalf("event order = %#v, want epoch-a then epoch-b", events)
	}
}

func TestProjectSemanticEventsRejectsInvalidInputWithoutAdvancingIt(t *testing.T) {
	for _, test := range []struct {
		name   string
		mutate func(map[string]any)
	}{
		{name: "non binary", mutate: func(record map[string]any) { record["values"] = []any{2} }},
		{name: "non scalar", mutate: func(record map[string]any) { record["values"] = []any{0, 1} }},
		{name: "null value", mutate: func(record map[string]any) { record["values"] = []any{nil} }},
		{name: "wrong family", mutate: func(record map[string]any) { record["family"] = "annotation" }},
		{name: "missing family", mutate: func(record map[string]any) { delete(record, "family") }},
		{name: "missing event time", mutate: func(record map[string]any) { delete(record, "event_time") }},
		{name: "null event time", mutate: func(record map[string]any) { record["event_time"] = nil }},
		{name: "fractional event time", mutate: func(record map[string]any) { record["event_time"] = 1.5 }},
		{name: "negative event time", mutate: func(record map[string]any) { record["event_time"] = -1 }},
	} {
		t.Run(test.name, func(t *testing.T) {
			store := openTestStore(t)
			mapping := putSemanticMapping(t, store, semantic.TriggerActiveEdge, 1)
			baseline := contactRecord("epoch-a", 1, []any{0})
			invalid := contactRecord("epoch-a", 2, []any{1})
			test.mutate(invalid)
			acceptContactRecords(t, store, "edge-node-01", "epoch-a", baseline, invalid)

			if _, err := store.ProjectSemanticEvents(context.Background(), 100); err == nil {
				t.Fatal("ProjectSemanticEvents accepted invalid contact input")
			}
			if got := store.testCount(t, "semantic_results"); got != 1 {
				t.Fatalf("semantic result count = %d, want only valid baseline", got)
			}
			var lastValue, nextSequence int64
			if err := store.db.QueryRow(`
				SELECT last_value, next_event_sequence
				FROM semantic_mapping_state
				WHERE mapping_id = ? AND mapping_revision = ?
			`, mapping.ID, mapping.Revision).Scan(&lastValue, &nextSequence); err != nil {
				t.Fatal(err)
			}
			if lastValue != 0 || nextSequence != 1 {
				t.Fatalf("state = last %d next %d, want last 0 next 1", lastValue, nextSequence)
			}
			if got := store.testCount(t, "semantic_events"); got != 0 {
				t.Fatalf("semantic event count = %d, want 0", got)
			}
		})
	}
}

func TestProjectSemanticEventsIsolatesPoisonMappingAndFairlyProcessesIndependentMapping(t *testing.T) {
	store := openTestStore(t)
	originalGenerator := newSemanticMappingID
	ids := []string{"sm-a-poison", "sm-z-valid"}
	newSemanticMappingID = func() (string, error) {
		id := ids[0]
		ids = ids[1:]
		return id, nil
	}
	t.Cleanup(func() { newSemanticMappingID = originalGenerator })

	poison := putSemanticMapping(t, store, semantic.TriggerActiveSample, 1)
	valid, err := store.PutSemanticMapping(context.Background(), semantic.MappingSpec{
		EdgeNodeID:  "edge-node-02",
		SeriesKey:   contactSeries,
		Meaning:     semantic.MeaningProductionPulse,
		TriggerMode: semantic.TriggerActiveSample,
		ActiveValue: 1,
	})
	if err != nil {
		t.Fatal(err)
	}
	acceptContactRecords(t, store, "edge-node-01", "epoch-poison",
		contactRecord("epoch-poison", 1, []any{2}),
		contactRecord("epoch-poison", 2, []any{1}),
		contactRecord("epoch-poison", 3, []any{1}),
	)
	acceptContactRecords(t, store, "edge-node-02", "epoch-valid",
		contactRecord("epoch-valid", 1, []any{1}),
	)

	processed, err := store.ProjectSemanticEvents(context.Background(), 3)
	if err == nil {
		t.Fatal("ProjectSemanticEvents succeeded despite poison mapping")
	}
	if processed != 1 {
		t.Fatalf("processed = %d, want one independent valid input", processed)
	}
	var poisonResults int
	if err := store.db.QueryRow(`
		SELECT count(*) FROM semantic_results
		WHERE mapping_id = ? AND mapping_revision = ?
	`, poison.ID, poison.Revision).Scan(&poisonResults); err != nil {
		t.Fatal(err)
	}
	if poisonResults != 0 {
		t.Fatalf("poison mapping results = %d, want 0", poisonResults)
	}
	events := listSemanticEvents(t, store)
	if len(events) != 1 || events[0].MappingID != valid.ID || events[0].EventSequence != 1 {
		t.Fatalf("events = %#v, want only independent valid mapping event", events)
	}
}

func contactRecord(ledgerEpoch string, pubSeq int64, values []any) map[string]any {
	return map[string]any{
		"family":         "measurement",
		"schema_version": 1,
		"epoch":          ledgerEpoch,
		"pub_seq":        pubSeq,
		"series_key":     contactSeries,
		"values":         values,
		"event_time":     pubSeq * 1_000,
	}
}

func acceptContactRecords(t *testing.T, store *Store, edgeNodeID, ledgerEpoch string, records ...map[string]any) {
	t.Helper()
	rawRecords := make([]json.RawMessage, 0, len(records))
	for _, record := range records {
		raw, err := json.Marshal(record)
		if err != nil {
			t.Fatal(err)
		}
		rawRecords = append(rawRecords, raw)
	}
	start := records[0]["pub_seq"].(int64)
	end := start + int64(len(records)) - 1
	batch := contract.RecordBatch{
		SchemaVersion: 1,
		EdgeNodeID:    edgeNodeID,
		LedgerEpoch:   ledgerEpoch,
		PublicationID: contract.PublicationID(edgeNodeID, ledgerEpoch, start, end),
		CursorStart:   start,
		CursorEnd:     end,
		Records:       rawRecords,
	}
	if _, err := store.AcceptBatch(context.Background(), batch); err != nil {
		t.Fatal(err)
	}
}

func putSemanticMapping(t *testing.T, store *Store, mode semantic.TriggerMode, activeValue int) semantic.Mapping {
	t.Helper()
	mapping, err := store.PutSemanticMapping(context.Background(), semantic.MappingSpec{
		EdgeNodeID:  "edge-node-01",
		SeriesKey:   contactSeries,
		Meaning:     semantic.MeaningProductionPulse,
		TriggerMode: mode,
		ActiveValue: activeValue,
	})
	if err != nil {
		t.Fatal(err)
	}
	return mapping
}

func acceptContactBatch(t *testing.T, store *Store, edgeNodeID, ledgerEpoch string, start int64, samples ...[]float64) {
	t.Helper()
	records := make([]json.RawMessage, 0, len(samples))
	for index, values := range samples {
		pubSeq := start + int64(index)
		record, err := json.Marshal(map[string]any{
			"family":         "measurement",
			"schema_version": 1,
			"epoch":          ledgerEpoch,
			"pub_seq":        pubSeq,
			"series_key":     contactSeries,
			"values":         values,
			"event_time":     pubSeq * 1_000,
		})
		if err != nil {
			t.Fatal(err)
		}
		records = append(records, record)
	}
	batch := contract.RecordBatch{
		SchemaVersion: 1,
		EdgeNodeID:    edgeNodeID,
		LedgerEpoch:   ledgerEpoch,
		PublicationID: contract.PublicationID(edgeNodeID, ledgerEpoch, start, start+int64(len(records))-1),
		CursorStart:   start,
		CursorEnd:     start + int64(len(records)) - 1,
		Records:       records,
	}
	if _, err := store.AcceptBatch(context.Background(), batch); err != nil {
		t.Fatal(err)
	}
}

func listSemanticEvents(t *testing.T, store *Store) []SemanticEvent {
	t.Helper()
	events, err := store.ListSemanticEvents(context.Background(), 100)
	if err != nil {
		t.Fatal(err)
	}
	return events
}

func acceptEpoch(t *testing.T, store *Store, edgeNodeID, ledgerEpoch string, pubSeq, value int64) {
	t.Helper()
	record, err := json.Marshal(map[string]any{
		"family":         "measurement",
		"schema_version": 1,
		"epoch":          ledgerEpoch,
		"pub_seq":        pubSeq,
		"series_key":     contactSeries,
		"values":         []int64{value},
	})
	if err != nil {
		t.Fatal(err)
	}
	batch := contract.RecordBatch{
		SchemaVersion: 1,
		EdgeNodeID:    edgeNodeID,
		LedgerEpoch:   ledgerEpoch,
		PublicationID: contract.PublicationID(edgeNodeID, ledgerEpoch, pubSeq, pubSeq),
		CursorStart:   pubSeq,
		CursorEnd:     pubSeq,
		Records:       []json.RawMessage{record},
	}
	if _, err := store.AcceptBatch(context.Background(), batch); err != nil {
		t.Fatal(err)
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

func (store *Store) testMappingStarts(t *testing.T, mappingID string, mappingRevision int64) map[string]int64 {
	t.Helper()
	rows, err := store.db.Query(`
		SELECT ledger_epoch, start_after_pub_seq
		FROM semantic_mapping_starts
		WHERE mapping_id = ? AND mapping_revision = ?
	`, mappingID, mappingRevision)
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()

	starts := make(map[string]int64)
	for rows.Next() {
		var ledgerEpoch string
		var startAfter int64
		if err := rows.Scan(&ledgerEpoch, &startAfter); err != nil {
			t.Fatal(err)
		}
		starts[ledgerEpoch] = startAfter
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	return starts
}

func (store *Store) testMappingEnds(t *testing.T, mappingID string, mappingRevision int64) map[string]int64 {
	t.Helper()
	rows, err := store.db.Query(`
		SELECT ledger_epoch, end_at_pub_seq
		FROM semantic_mapping_ends
		WHERE mapping_id = ? AND mapping_revision = ?
	`, mappingID, mappingRevision)
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()

	ends := make(map[string]int64)
	for rows.Next() {
		var ledgerEpoch string
		var endAt int64
		if err := rows.Scan(&ledgerEpoch, &endAt); err != nil {
			t.Fatal(err)
		}
		ends[ledgerEpoch] = endAt
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	return ends
}
