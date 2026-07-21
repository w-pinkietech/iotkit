package store

import (
	"context"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"path/filepath"
	"strings"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantic"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
)

const inventorySeriesKey = "018f0000-0000-7000-8000-000000000001:temperature:na:primary"

func TestDescriptorInventoryRefsAreStableAcrossReplayAndReopen(t *testing.T) {
	path := filepath.Join(t.TempDir(), "edge.db")
	first, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	snapshot := descriptorFixture(t)
	if _, err := first.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	firstDeviceRef := testSourceRef(t, first.db, "edge_devices", "device_ref")
	firstSignalRef := testSourceRef(t, first.db, "edge_signals", "signal_ref")
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
	reopenedDeviceRef := testSourceRef(t, reopened.db, "edge_devices", "device_ref")
	reopenedSignalRef := testSourceRef(t, reopened.db, "edge_signals", "signal_ref")
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
	if _, err := acceptBatchForTest(t, store, batch); err != nil {
		t.Fatal(err)
	}
	count, err := store.ReconcileInventorySources(context.Background(), 100)
	if err != nil {
		t.Fatal(err)
	}
	if count != 1 {
		t.Fatalf("reconciled = %d, want 1", count)
	}
	if got := testTableCount(t, store.db, "edge_signals"); got != 1 {
		t.Fatalf("edge signal sources = %d, want 1", got)
	}
	if got := testTableCount(t, store.db, "edge_devices"); got != 1 {
		t.Fatalf("edge device sources = %d, want 1", got)
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

func TestReconcileInventorySourcesSkipsNonCanonicalSeriesAndAdvancesProgress(t *testing.T) {
	store := openTestStore(t)
	if _, err := acceptBatchForTest(t, store, testBatch(t)); err != nil {
		t.Fatal(err)
	}
	processed, err := store.ReconcileInventorySources(context.Background(), 1)
	if err != nil {
		t.Fatal(err)
	}
	if processed != 1 {
		t.Fatalf("processed = %d, want 1", processed)
	}
	if got := testTableCount(t, store.db, "edge_signals"); got != 0 {
		t.Fatalf("non-canonical signal sources = %d, want 0", got)
	}
	processed, err = store.ReconcileInventorySources(context.Background(), 1)
	if err != nil {
		t.Fatal(err)
	}
	if processed != 0 {
		t.Fatalf("reprocessed rows = %d, want 0", processed)
	}
	var lastPubSeq int64
	if err := store.db.QueryRow(`
		SELECT last_pub_seq FROM inventory_projection_cursors
		WHERE edge_node_id = 'edge-node-01' AND ledger_epoch = 'epoch-01'
	`).Scan(&lastPubSeq); err != nil {
		t.Fatal(err)
	}
	if lastPubSeq != 1 {
		t.Fatalf("projection cursor = %d, want 1", lastPubSeq)
	}
}

func TestReconcileInventorySourcesProcessesAtMostLimitAndResumes(t *testing.T) {
	store := openTestStore(t)
	records := []json.RawMessage{
		inventoryMeasurementRecord(t, 1, inventorySeriesKey, []any{20.0}, 1000),
		inventoryMeasurementRecord(t, 2, inventorySeriesKey, []any{21.0}, 2000),
		inventoryMeasurementRecord(t, 3, inventorySeriesKey, []any{22.0}, 3000),
	}
	batch := contract.RecordBatch{
		SchemaVersion: 1,
		EdgeNodeID:    "edge-node-01",
		LedgerEpoch:   "epoch-01",
		PublicationID: contract.PublicationID("edge-node-01", "epoch-01", 1, 3),
		CursorStart:   1,
		CursorEnd:     3,
		Records:       records,
	}
	if _, err := acceptBatchForTest(t, store, batch); err != nil {
		t.Fatal(err)
	}
	for wantCursor := int64(1); wantCursor <= 3; wantCursor++ {
		processed, err := store.ReconcileInventorySources(context.Background(), 1)
		if err != nil {
			t.Fatal(err)
		}
		if processed != 1 {
			t.Fatalf("processed at cursor %d = %d, want 1", wantCursor, processed)
		}
		var cursor int64
		if err := store.db.QueryRow(`
			SELECT last_pub_seq FROM inventory_projection_cursors
			WHERE edge_node_id = 'edge-node-01' AND ledger_epoch = 'epoch-01'
		`).Scan(&cursor); err != nil {
			t.Fatal(err)
		}
		if cursor != wantCursor {
			t.Fatalf("projection cursor = %d, want %d", cursor, wantCursor)
		}
	}
	processed, err := store.ReconcileInventorySources(context.Background(), 1)
	if err != nil {
		t.Fatal(err)
	}
	if processed != 0 {
		t.Fatalf("processed after convergence = %d, want 0", processed)
	}
}

func TestListInventorySummariesExposeExplicitEdgeNodeIdentityWithoutInternalKeys(t *testing.T) {
	store := openTestStore(t)
	snapshot := descriptorFixture(t)
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	deviceRef := testSourceRef(t, store.db, "edge_devices", "device_ref")
	signalRef := testSourceRef(t, store.db, "edge_signals", "signal_ref")
	if _, err := store.UpdateDeviceProfile(
		context.Background(), edgeapp.LocalCLIActor(), deviceRef,
		edgeapp.DeviceProfileInput{DisplayName: "プレス機", Location: "第2工場"},
		edgeapp.RevisionPrecondition{},
	); err != nil {
		t.Fatal(err)
	}
	if _, err := store.UpdateSignalProfile(
		context.Background(), edgeapp.LocalCLIActor(), signalRef,
		edgeapp.SignalProfileInput{
			DisplayName:       "生産接点",
			DisplaySensorType: "contact",
			DisplayValueKind:  "boolean",
			DisplayUnitMode:   "dimensionless",
		},
		edgeapp.RevisionPrecondition{},
	); err != nil {
		t.Fatal(err)
	}

	devices, err := store.ListInventoryDevices(context.Background(), 10, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(devices) != 1 || devices[0].DisplayName != "プレス機" ||
		devices[0].Location != "第2工場" || devices[0].DescriptorPresence != "current" ||
		devices[0].EdgeNodeID != "edge-node-01" {
		t.Fatalf("devices = %#v", devices)
	}
	signals, err := store.ListInventorySignals(context.Background(), 10, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(signals) != 1 || signals[0].DisplayName != "生産接点" ||
		signals[0].DeviceRef == nil || *signals[0].DeviceRef != deviceRef ||
		signals[0].EdgeNodeID != "edge-node-01" {
		t.Fatalf("signals = %#v", signals)
	}
	encoded, err := json.Marshal(struct {
		Devices []edgeapp.DeviceSummary `json:"devices"`
		Signals []edgeapp.SignalSummary `json:"signals"`
	}{devices, signals})
	if err != nil {
		t.Fatal(err)
	}
	for _, forbidden := range []string{
		"system_id", "series_key", "identifier",
		"device_display_name", "device_location",
	} {
		if strings.Contains(string(encoded), forbidden) {
			t.Fatalf("inventory JSON exposes %q: %s", forbidden, encoded)
		}
	}
}

func TestListSetupDevicesGroupsSignalsAndIncludesSetupFacts(t *testing.T) {
	store := openTestStore(t)
	snapshot := descriptorFixture(t)
	systemID := snapshot.Devices[0].SystemID
	channel := int32(1)
	unit := "Cel"
	snapshot.Signals = append(snapshot.Signals, contract.DescriptorSignal{
		SeriesKey:      systemID + ":temperature_c:1:primary",
		SystemID:       systemID,
		MeasurementKey: "temperature_c",
		ChannelIndex:   &channel,
		Variant:        "primary",
		Unit:           &unit,
		ValueType:      "float",
	})
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	signalRef := testSourceRef(t, store.db, "edge_signals", "signal_ref")
	if _, err := store.UpdateSignalProfile(
		context.Background(),
		edgeapp.LocalCLIActor(),
		signalRef,
		edgeapp.SignalProfileInput{
			DisplayName:       "接点入力",
			DisplaySensorType: "contact",
			DisplayValueKind:  "boolean",
			DisplayUnitMode:   "dimensionless",
		},
		edgeapp.RevisionPrecondition{},
	); err != nil {
		t.Fatal(err)
	}

	devices, err := store.ListSetupDevices(context.Background(), 100)
	if err != nil {
		t.Fatal(err)
	}
	if len(devices) != 1 || devices[0].Identifier == nil ||
		*devices[0].Identifier != "01234567" || len(devices[0].Signals) != 2 {
		t.Fatalf("setup devices = %#v", devices)
	}
	var foundProfile, foundChannel bool
	for _, signal := range devices[0].Signals {
		if signal.Profile != nil && signal.Profile.DisplayName == "接点入力" &&
			signal.Profile.Complete() {
			foundProfile = true
		}
		if signal.ChannelIndex != nil && *signal.ChannelIndex == 1 &&
			signal.Signal.SensorType != nil &&
			*signal.Signal.SensorType == "temperature_c" {
			foundChannel = true
		}
	}
	if !foundProfile || !foundChannel {
		t.Fatalf("grouped setup signals = %#v", devices[0].Signals)
	}
}

func TestListSetupDevicesDoesNotSilentlyTruncateAfterOneSignalPage(t *testing.T) {
	store := openTestStore(t)
	snapshot := descriptorFixture(t)
	systemID := snapshot.Devices[0].SystemID
	for index := int32(0); index < 100; index++ {
		channel := index
		snapshot.Signals = append(snapshot.Signals, contract.DescriptorSignal{
			SeriesKey:      fmt.Sprintf("%s:temperature_c:%d:primary", systemID, index),
			SystemID:       systemID,
			MeasurementKey: "temperature_c",
			ChannelIndex:   &channel,
			Variant:        "primary",
			ValueType:      "float",
		})
	}
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	devices, err := store.ListSetupDevices(context.Background(), 100)
	if err != nil {
		t.Fatal(err)
	}
	if len(devices) != 1 || len(devices[0].Signals) != 101 {
		t.Fatalf("setup signal count = %d, want 101; devices=%d",
			len(devices[0].Signals), len(devices))
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
	if _, err := acceptBatchForTest(t, store, batch); err != nil {
		t.Fatal(err)
	}
	if _, err := store.db.Exec(`
		UPDATE raw_records
		SET received_at = CASE pub_seq
			WHEN 1 THEN 1000
			WHEN 2 THEN 2000
			WHEN 3 THEN 3000
		END
	`); err != nil {
		t.Fatal(err)
	}
	if _, err := store.ReconcileInventorySources(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
	var materializedDeviceLast int64
	if err := store.db.QueryRow(`
		SELECT last_received_at FROM edge_devices LIMIT 1
	`).Scan(&materializedDeviceLast); err != nil {
		t.Fatal(err)
	}
	if materializedDeviceLast != 3000 {
		t.Fatalf("materialized device last received = %d, want 3000", materializedDeviceLast)
	}
	if _, err := store.db.Exec(`DELETE FROM raw_records`); err != nil {
		t.Fatal(err)
	}

	signals, err := store.ListInventorySignals(context.Background(), 10, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(signals) != 1 || signals[0].Latest == nil {
		t.Fatalf("signals = %#v", signals)
	}
	if string(signals[0].Latest.Values) != `[21.5]` || signals[0].Latest.EventTime != 2000 ||
		signals[0].Latest.EdgeReceivedAt != 2000 {
		t.Fatalf("latest = %#v", signals[0].Latest)
	}
	if signals[0].LastReceivedAt == nil || *signals[0].LastReceivedAt != 3000 {
		t.Fatalf("signal last received = %#v, want 3000", signals[0].LastReceivedAt)
	}
	devices, err := store.ListInventoryDevices(context.Background(), 10, "")
	if err != nil {
		t.Fatal(err)
	}
	if devices[0].LastReceivedAt == nil || *devices[0].LastReceivedAt != 3000 {
		t.Fatalf("device last received = %#v, want 3000", devices[0].LastReceivedAt)
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
		edgeapp.LocalCLIActor(),
		semantic.MappingSpec{
			EdgeNodeID:  snapshot.EdgeNodeID,
			SeriesKey:   snapshot.Signals[0].SeriesKey,
			Meaning:     semantic.MeaningProductionPulse,
			TriggerMode: semantic.TriggerActiveEdge,
			ActiveValue: 1,
		},
		edgeapp.RevisionPrecondition{},
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

func TestListInventorySignalsReportsActiveMultipleRuleConfiguration(t *testing.T) {
	archive := openTestStore(t)
	snapshot := descriptorFixture(t)
	if _, err := archive.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	signals, err := archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	configuration, err := archive.GetSemanticConfiguration(
		context.Background(),
		signals[0].SignalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	expected := configuration.Revision
	if _, err := archive.CreateSemanticRule(
		context.Background(),
		edgeapp.LocalCLIActor(),
		signals[0].SignalRef,
		"測定値",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		edgeapp.RevisionPrecondition{Expected: &expected},
	); err != nil {
		t.Fatal(err)
	}
	signals, err = archive.ListInventorySignals(context.Background(), 10, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(signals) != 1 || !signals[0].HasSemanticMapping {
		t.Fatalf("signals=%#v", signals)
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

type testQueryRower interface {
	QueryRow(query string, args ...any) *sql.Row
}

func testTableCount(t *testing.T, db testQueryRower, table string) int {
	t.Helper()
	switch table {
	case "edge_devices", "edge_signals", "descriptor_signals", "signal_profiles":
	default:
		t.Fatalf("unsupported count table %q", table)
	}
	var count int
	if err := db.QueryRow("SELECT count(*) FROM " + table).Scan(&count); err != nil {
		t.Fatal(err)
	}
	return count
}

func testSourceRef(t *testing.T, db testQueryRower, table, column string) string {
	t.Helper()
	if table != "edge_devices" && table != "edge_signals" {
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
