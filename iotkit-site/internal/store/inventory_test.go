package store

import (
	"context"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"
)

const inventorySeriesKey = "018f0000-0000-7000-8000-000000000001:temperature:na:primary"

func TestDescriptorInventoryRefsAreStableAcrossReplayAndReopen(t *testing.T) {
	path := filepath.Join(t.TempDir(), "site.db")
	first, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	snapshot := descriptorFixture(t)
	if _, err := first.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	firstDeviceRef := testSourceRef(t, first.db, "site_devices", "device_ref")
	firstSignalRef := testSourceRef(t, first.db, "site_signals", "signal_ref")
	if err := first.Close(); err != nil {
		t.Fatal(err)
	}

	reopened, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = reopened.Close() })
	if _, err := reopened.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	reopenedDeviceRef := testSourceRef(t, reopened.db, "site_devices", "device_ref")
	reopenedSignalRef := testSourceRef(t, reopened.db, "site_signals", "signal_ref")
	if reopenedDeviceRef != firstDeviceRef || reopenedSignalRef != firstSignalRef {
		t.Fatalf("refs changed: device=%q signal=%q", reopenedDeviceRef, reopenedSignalRef)
	}
	assertResourceRef(t, firstDeviceRef, "dev_")
	assertResourceRef(t, firstSignalRef, "sig_")
}

func TestReconcileInventorySourcesCreatesMeasurementFirstPlaceholder(t *testing.T) {
	store := openTestStore(t)
	batch := testBatch(t)
	record := map[string]any{
		"family":         "measurement",
		"schema_version": 1,
		"epoch":          batch.LedgerEpoch,
		"pub_seq":        1,
		"series_key":     inventorySeriesKey,
		"values":         []float64{21.5},
	}
	encoded, err := json.Marshal(record)
	if err != nil {
		t.Fatal(err)
	}
	batch.Records[0] = encoded
	if _, err := store.AcceptBatch(context.Background(), batch); err != nil {
		t.Fatal(err)
	}
	count, err := store.ReconcileInventorySources(context.Background(), 100)
	if err != nil {
		t.Fatal(err)
	}
	if count != 1 {
		t.Fatalf("reconciled = %d, want 1", count)
	}
	if got := testTableCount(t, store.db, "site_signals"); got != 1 {
		t.Fatalf("site signal sources = %d, want 1", got)
	}
	if got := testTableCount(t, store.db, "site_devices"); got != 1 {
		t.Fatalf("site device sources = %d, want 1", got)
	}
	if got := testTableCount(t, store.db, "descriptor_signals"); got != 0 {
		t.Fatalf("descriptor signals = %d, want placeholder only", got)
	}
}

func testTableCount(t *testing.T, db *sql.DB, table string) int {
	t.Helper()
	switch table {
	case "site_devices", "site_signals", "descriptor_signals", "signal_profiles":
	default:
		t.Fatalf("unsupported count table %q", table)
	}
	var count int
	if err := db.QueryRow("SELECT count(*) FROM " + table).Scan(&count); err != nil {
		t.Fatal(err)
	}
	return count
}

func testSourceRef(t *testing.T, db *sql.DB, table, column string) string {
	t.Helper()
	if table != "site_devices" && table != "site_signals" {
		t.Fatalf("unsupported source table %q", table)
	}
	if column != "device_ref" && column != "signal_ref" {
		t.Fatalf("unsupported source ref column %q", column)
	}
	var ref string
	if err := db.QueryRow("SELECT " + column + " FROM " + table + " LIMIT 1").Scan(&ref); err != nil {
		t.Fatal(err)
	}
	return ref
}

func assertResourceRef(t *testing.T, ref, prefix string) {
	t.Helper()
	if !strings.HasPrefix(ref, prefix) {
		t.Fatalf("resource ref %q does not have prefix %q", ref, prefix)
	}
	randomPart := strings.TrimPrefix(ref, prefix)
	if len(randomPart) != 32 {
		t.Fatalf("resource ref random part length = %d, want 32", len(randomPart))
	}
	if _, err := hex.DecodeString(randomPart); err != nil {
		t.Fatalf("resource ref %q is not hexadecimal: %v", ref, err)
	}
}
