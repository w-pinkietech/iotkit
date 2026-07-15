package store

import (
	"context"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantic"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
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
		"event_time":     1000,
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
	signals, err := store.ListInventorySignals(context.Background(), 10, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(signals) != 1 || signals[0].DescriptorPresence != "unknown" || signals[0].Latest == nil {
		t.Fatalf("measurement-first signals = %#v", signals)
	}
}

func TestListInventorySummariesJoinProfilesWithoutSourceIdentity(t *testing.T) {
	store := openTestStore(t)
	snapshot := descriptorFixture(t)
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	deviceRef := testSourceRef(t, store.db, "site_devices", "device_ref")
	signalRef := testSourceRef(t, store.db, "site_signals", "signal_ref")
	if _, err := store.UpdateDeviceProfile(
		context.Background(), siteapp.LocalCLIActor(), deviceRef,
		siteapp.DeviceProfileInput{DisplayName: "プレス機", Location: "第2工場"},
		siteapp.RevisionPrecondition{},
	); err != nil {
		t.Fatal(err)
	}
	if _, err := store.UpdateSignalProfile(
		context.Background(), siteapp.LocalCLIActor(), signalRef,
		siteapp.SignalProfileInput{DisplayName: "生産接点"},
		siteapp.RevisionPrecondition{},
	); err != nil {
		t.Fatal(err)
	}

	devices, err := store.ListInventoryDevices(context.Background(), 10, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(devices) != 1 || devices[0].DisplayName != "プレス機" ||
		devices[0].Location != "第2工場" || devices[0].DescriptorPresence != "current" {
		t.Fatalf("devices = %#v", devices)
	}
	signals, err := store.ListInventorySignals(context.Background(), 10, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(signals) != 1 || signals[0].DisplayName != "生産接点" ||
		signals[0].DeviceRef == nil || *signals[0].DeviceRef != deviceRef {
		t.Fatalf("signals = %#v", signals)
	}
	encoded, err := json.Marshal(struct {
		Devices []siteapp.DeviceSummary `json:"devices"`
		Signals []siteapp.SignalSummary `json:"signals"`
	}{devices, signals})
	if err != nil {
		t.Fatal(err)
	}
	for _, forbidden := range []string{"edge_node_id", "system_id", "series_key", "identifier"} {
		if strings.Contains(string(encoded), forbidden) {
			t.Fatalf("inventory JSON exposes %q: %s", forbidden, encoded)
		}
	}
}

func TestListInventorySignalsUsesLatestValidMeasurement(t *testing.T) {
	store := openTestStore(t)
	snapshot := descriptorFixture(t)
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	seriesKey := snapshot.Signals[0].SeriesKey
	records := []json.RawMessage{
		inventoryMeasurementRecord(t, 1, seriesKey, []any{20.0}, 1000),
		inventoryMeasurementRecord(t, 2, seriesKey, []any{21.5}, 2000),
		inventoryMeasurementRecord(t, 3, seriesKey, []any{"invalid"}, 3000),
	}
	batch := contract.RecordBatch{
		SchemaVersion: 1,
		EdgeNodeID:    snapshot.EdgeNodeID,
		LedgerEpoch:   snapshot.LedgerEpoch,
		PublicationID: contract.PublicationID(snapshot.EdgeNodeID, snapshot.LedgerEpoch, 1, 3),
		CursorStart:   1,
		CursorEnd:     3,
		Records:       records,
	}
	if _, err := store.AcceptBatch(context.Background(), batch); err != nil {
		t.Fatal(err)
	}

	signals, err := store.ListInventorySignals(context.Background(), 10, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(signals) != 1 || signals[0].Latest == nil {
		t.Fatalf("signals = %#v", signals)
	}
	if string(signals[0].Latest.Values) != `[21.5]` || signals[0].Latest.EventTime != 2000 {
		t.Fatalf("latest = %#v", signals[0].Latest)
	}
	devices, err := store.ListInventoryDevices(context.Background(), 10, "")
	if err != nil {
		t.Fatal(err)
	}
	if devices[0].LastReceivedAt == nil || *devices[0].LastReceivedAt != signals[0].Latest.SiteReceivedAt {
		t.Fatalf("device last received = %#v, signal latest = %#v", devices[0].LastReceivedAt, signals[0].Latest)
	}
}

func TestListInventoryUsesStableExclusiveRefPagination(t *testing.T) {
	store := openTestStore(t)
	snapshot := descriptorFixture(t)
	secondSystemID := "018f0000-0000-7000-8000-000000000002"
	snapshot.Devices = append(snapshot.Devices, contract.DescriptorDevice{
		SystemID: secondSystemID,
		State:    "active",
	})
	snapshot.Signals = append(snapshot.Signals, contract.DescriptorSignal{
		SeriesKey:      secondSystemID + ":temperature:na:primary",
		SystemID:       secondSystemID,
		MeasurementKey: "temperature",
		Variant:        "primary",
		ValueType:      "float",
	})
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	first, err := store.ListInventoryDevices(context.Background(), 1, "")
	if err != nil {
		t.Fatal(err)
	}
	second, err := store.ListInventoryDevices(context.Background(), 1, first[0].DeviceRef)
	if err != nil {
		t.Fatal(err)
	}
	if len(first) != 1 || len(second) != 1 || first[0].DeviceRef >= second[0].DeviceRef {
		t.Fatalf("paginated devices = first %#v, second %#v", first, second)
	}
	if _, err := store.ListInventoryDevices(context.Background(), 1, "sig_00000000000000000000000000000001"); err == nil {
		t.Fatal("signal ref was accepted as a device cursor")
	}
}

func TestListInventorySignalsReportsActiveSemanticMapping(t *testing.T) {
	store := openTestStore(t)
	snapshot := descriptorFixture(t)
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	if _, err := store.ApplySemanticMapping(
		context.Background(),
		siteapp.LocalCLIActor(),
		semantic.MappingSpec{
			EdgeNodeID:  snapshot.EdgeNodeID,
			SeriesKey:   snapshot.Signals[0].SeriesKey,
			Meaning:     semantic.MeaningProductionPulse,
			TriggerMode: semantic.TriggerActiveEdge,
			ActiveValue: 1,
		},
		siteapp.RevisionPrecondition{},
	); err != nil {
		t.Fatal(err)
	}
	signals, err := store.ListInventorySignals(context.Background(), 10, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(signals) != 1 || !signals[0].HasSemanticMapping {
		t.Fatalf("signals = %#v", signals)
	}
}

func inventoryMeasurementRecord(
	t *testing.T,
	pubSeq int64,
	seriesKey string,
	values []any,
	eventTime int64,
) json.RawMessage {
	t.Helper()
	encoded, err := json.Marshal(map[string]any{
		"family":         "measurement",
		"schema_version": 1,
		"epoch":          "epoch-01",
		"pub_seq":        pubSeq,
		"series_key":     seriesKey,
		"values":         values,
		"event_time":     eventTime,
	})
	if err != nil {
		t.Fatal(err)
	}
	return encoded
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
