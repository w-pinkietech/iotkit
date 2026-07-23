package store

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/contract"
)

func descriptorFixture(t *testing.T) contract.DescriptorSnapshot {
	t.Helper()
	payload, err := os.ReadFile(filepath.Join("..", "..", "..", "testdata", "egress", "v2", "descriptor-snapshot.json"))
	if err != nil {
		t.Fatal(err)
	}
	snapshot, err := contract.DecodeDescriptorSnapshot(payload)
	if err != nil {
		t.Fatal(err)
	}
	return snapshot
}

func TestApplyDescriptorSnapshotPersistsCurrentReplica(t *testing.T) {
	store := openTestStore(t)
	result, err := store.ApplyDescriptorSnapshot(context.Background(), descriptorFixture(t))
	if err != nil {
		t.Fatal(err)
	}
	if result.Status != DescriptorApplied {
		t.Fatalf("result = %#v", result)
	}
	devices, err := store.ListDescriptorDevices(context.Background(), "edge-node-01")
	if err != nil {
		t.Fatal(err)
	}
	if len(devices) != 1 || devices[0].Presence != DescriptorCurrent || devices[0].Identifier == nil {
		t.Fatalf("devices = %#v", devices)
	}
	signals, err := store.ListDescriptorSignals(context.Background(), "edge-node-01")
	if err != nil {
		t.Fatal(err)
	}
	if len(signals) != 1 || signals[0].Presence != DescriptorCurrent || signals[0].ValueType != "bool" {
		t.Fatalf("signals = %#v", signals)
	}
}

func TestApplyDescriptorSnapshotPersistsAndClearsCurrentDeviceModel(t *testing.T) {
	store := openTestStore(t)
	snapshot := descriptorFixture(t)
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	devices, err := store.ListDescriptorDevices(context.Background(), "edge-node-01")
	if err != nil {
		t.Fatal(err)
	}
	if len(devices) != 1 || devices[0].ModelID == nil || *devices[0].ModelID != "mcp9600" {
		t.Fatalf("devices = %#v", devices)
	}
	inventory, err := store.ListInventoryDevices(context.Background(), 10, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(inventory) != 1 || inventory[0].ModelID == nil || *inventory[0].ModelID != "mcp9600" {
		t.Fatalf("inventory = %#v", inventory)
	}

	next := snapshot
	next.DescriptorRevision++
	next.Devices = append([]contract.DescriptorDevice(nil), snapshot.Devices...)
	next.Devices[0].ModelID = nil
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), next); err != nil {
		t.Fatal(err)
	}
	devices, err = store.ListDescriptorDevices(context.Background(), "edge-node-01")
	if err != nil {
		t.Fatal(err)
	}
	if len(devices) != 1 || devices[0].ModelID != nil {
		t.Fatalf("model was not cleared: %#v", devices)
	}
}

func TestApplyDescriptorSnapshotHandlesReplayStaleAndConflict(t *testing.T) {
	store := openTestStore(t)
	snapshot := descriptorFixture(t)
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	replay, err := store.ApplyDescriptorSnapshot(context.Background(), snapshot)
	if err != nil || replay.Status != DescriptorIdempotent {
		t.Fatalf("replay = %#v, %v", replay, err)
	}

	stale := snapshot
	stale.DescriptorRevision--
	ignored, err := store.ApplyDescriptorSnapshot(context.Background(), stale)
	if err != nil || ignored.Status != DescriptorStaleIgnored {
		t.Fatalf("stale = %#v, %v", ignored, err)
	}

	conflict := snapshot
	identifier := "different"
	conflict.Devices = append([]contract.DescriptorDevice(nil), snapshot.Devices...)
	conflict.Devices[0].Identifier = &identifier
	_, err = store.ApplyDescriptorSnapshot(context.Background(), conflict)
	if !errors.Is(err, ErrDescriptorConflict) {
		t.Fatalf("conflict error = %v", err)
	}
	events, err := store.ListAuditEvents(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) != 1 || events[0].Operation != "descriptor_snapshot.conflict" || events[0].ActorClass != "system" {
		t.Fatalf("events = %#v", events)
	}
}

func TestApplyDescriptorSnapshotMarksMissingRowsStaleAndAcceptsNewEpoch(t *testing.T) {
	store := openTestStore(t)
	snapshot := descriptorFixture(t)
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}

	next := snapshot
	next.LedgerEpoch = "epoch-02"
	next.DescriptorRevision = 1
	next.Devices = []contract.DescriptorDevice{}
	next.Signals = []contract.DescriptorSignal{}
	result, err := store.ApplyDescriptorSnapshot(context.Background(), next)
	if err != nil || result.Status != DescriptorApplied {
		t.Fatalf("new epoch = %#v, %v", result, err)
	}
	devices, err := store.ListDescriptorDevices(context.Background(), "edge-node-01")
	if err != nil {
		t.Fatal(err)
	}
	if len(devices) != 1 || devices[0].Presence != DescriptorStale ||
		devices[0].ModelID == nil || *devices[0].ModelID != "mcp9600" {
		t.Fatalf("devices = %#v", devices)
	}
	signals, err := store.ListDescriptorSignals(context.Background(), "edge-node-01")
	if err != nil {
		t.Fatal(err)
	}
	if len(signals) != 1 || signals[0].Presence != DescriptorStale {
		t.Fatalf("signals = %#v", signals)
	}
}
