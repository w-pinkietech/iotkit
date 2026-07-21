package store

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
)

func TestStorageStatusReportsDatabaseFilesystemAndQueues(t *testing.T) {
	archive := openTestStore(t)
	seedHistorySignal(t, archive, "edge-a", "signal-a", "device-01:temperature_c:na:primary")
	seedHistoryRecord(t, archive, "edge-a", "epoch-a", "device-01:temperature_c:na:primary", 1, 1_000, 20)

	status, err := archive.GetStorageStatus(context.Background(), 90)
	if err != nil {
		t.Fatal(err)
	}
	if !status.FilesystemAvailable || status.DatabaseBytes < 1 ||
		status.DiskTotalBytes < status.DiskAvailableBytes {
		t.Fatalf("storage status = %#v", status)
	}
	if status.RawRecordCount != 1 || status.WarningPercent != 90 {
		t.Fatalf("storage counts = %#v", status)
	}
	if status.State != StorageHealthy && status.State != StorageWarning {
		t.Fatalf("storage state = %q", status.State)
	}
}

func TestStorageStatusSeparatesBackupProtectedAndNewRawRecords(t *testing.T) {
	archive := openTestStore(t)
	series := "device-01:temperature_c:na:primary"
	seedHistorySignal(t, archive, "edge-a", "signal-a", series)
	seedHistoryRecord(t, archive, "edge-a", "epoch-a", series, 1, 1_000, 20)
	if _, err := archive.db.Exec(`
		INSERT INTO accepted_cursors(edge_node_id, ledger_epoch, accepted_through, updated_at)
		VALUES('edge-a', 'epoch-a', 1, 1000)
	`); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ApplyEncryptedBackup(
		context.Background(), edgeapp.LocalCLIActor(),
		filepath.Join(t.TempDir(), "edge.iotkit-backup"),
		testBackupPassphrase,
	); err != nil {
		t.Fatal(err)
	}
	seedHistoryRecord(t, archive, "edge-a", "epoch-a", series, 2, 2_000, 21)
	status, err := archive.GetStorageStatus(context.Background(), 90)
	if err != nil {
		t.Fatal(err)
	}
	if status.BackupProtectedRawCount != 1 || status.UnprotectedRawCount != 1 || status.AutomaticRawPurgeEnabled {
		t.Fatalf("retention status = %#v", status)
	}
}

func TestStorageStatusValidatesWarningThreshold(t *testing.T) {
	archive := openTestStore(t)
	for _, invalid := range []int{0, 49, 100, 101} {
		if _, err := archive.GetStorageStatus(context.Background(), invalid); err == nil {
			t.Fatalf("warning threshold %d was accepted", invalid)
		}
	}
}
