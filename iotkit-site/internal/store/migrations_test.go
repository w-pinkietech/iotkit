package store

import (
	"context"
	"database/sql"
	"fmt"
	"path/filepath"
	"testing"
)

func TestOpenMigratesRealVersionThreeDatabaseWithoutDroppingData(t *testing.T) {
	path := filepath.Join(t.TempDir(), "site.db")
	db, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	for _, migration := range schemaMigrations[:3] {
		if _, err := db.Exec(migration.sql); err != nil {
			t.Fatal(err)
		}
		if _, err := db.Exec(fmt.Sprintf("PRAGMA user_version = %d", migration.version)); err != nil {
			t.Fatal(err)
		}
	}
	const systemID = "018f0000-0000-7000-8000-000000000001"
	const seriesKey = systemID + ":temperature:na:primary"
	if _, err := db.Exec(`
		INSERT INTO descriptor_devices (
			edge_node_id, system_id, identifier, state, presence,
			descriptor_revision, updated_at
		) VALUES ('edge-node-01', ?, '01234567', 'active', 'current', 1, 1000)
	`, systemID); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`
		INSERT INTO descriptor_signals (
			edge_node_id, series_key, system_id, measurement_key, channel_index,
			variant, unit, value_type, presence, descriptor_revision, updated_at
		) VALUES ('edge-node-01', ?, ?, 'temperature', NULL,
			'primary', 'C', 'float', 'current', 1, 1000)
	`, seriesKey, systemID); err != nil {
		t.Fatal(err)
	}
	recordJSON := fmt.Sprintf(
		`{"family":"measurement","schema_version":1,"epoch":"epoch-01","pub_seq":1,"series_key":%q,"values":[21.5],"event_time":1500}`,
		seriesKey,
	)
	if _, err := db.Exec(`
		INSERT INTO raw_records (
			edge_node_id, ledger_epoch, pub_seq, publication_id,
			record_json, record_sha256, received_at
		) VALUES ('edge-node-01', 'epoch-01', 1, 'publication-01', ?, zeroblob(32), 2000)
	`, recordJSON); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`
		INSERT INTO accepted_cursors (
			edge_node_id, ledger_epoch, accepted_through, updated_at
		) VALUES ('edge-node-01', 'epoch-01', 1, 2000)
	`); err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}

	store, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = store.Close() })
	var version int
	if err := store.db.QueryRow("PRAGMA user_version").Scan(&version); err != nil {
		t.Fatal(err)
	}
	if version != 9 {
		t.Fatalf("schema version = %d, want 9", version)
	}
	if got := testTableCount(t, store.db, "site_devices"); got != 1 {
		t.Fatalf("backfilled devices = %d, want 1", got)
	}
	if got := testTableCount(t, store.db, "site_signals"); got != 1 {
		t.Fatalf("backfilled signals = %d, want 1", got)
	}
	records, err := store.ListRawRecords(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != 1 {
		t.Fatalf("raw records = %d, want 1", len(records))
	}
	if processed, err := store.ReconcileInventorySources(context.Background(), 10); err != nil || processed != 1 {
		t.Fatalf("post-migration projection processed = %d, err = %v", processed, err)
	}
	signals, err := store.ListInventorySignals(context.Background(), 10, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(signals) != 1 || signals[0].Latest == nil || signals[0].Latest.EventTime != 1500 {
		t.Fatalf("migrated signals = %#v", signals)
	}
}
