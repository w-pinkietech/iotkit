package store

import (
	"context"
	"path/filepath"
	"testing"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
)

func TestDiagnosticsReportsMissingBackupWithoutInventingServiceHealth(t *testing.T) {
	archive := openTestStore(t)
	report, err := archive.GetDiagnostics(context.Background(), 90, time.UnixMilli(10_000))
	if err != nil {
		t.Fatal(err)
	}
	if report.State != DiagnosticAttention || len(report.Issues) != 1 || report.Issues[0].Code != "edge_backup_missing" {
		t.Fatalf("report = %#v", report)
	}
	if len(report.Limitations) < 2 {
		t.Fatalf("diagnostic limitations = %#v", report.Limitations)
	}
}

func TestDiagnosticsClearsMissingBackupAfterVerifiedBackup(t *testing.T) {
	archive := openTestStore(t)
	if _, err := archive.ApplyEncryptedBackup(
		context.Background(),
		edgeapp.LocalCLIActor(),
		filepath.Join(t.TempDir(), "edge.iotkit-backup"),
		testBackupPassphrase,
	); err != nil {
		t.Fatal(err)
	}
	report, err := archive.GetDiagnostics(context.Background(), 90, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	for _, issue := range report.Issues {
		if issue.Code == "edge_backup_missing" {
			t.Fatalf("verified backup still reported missing: %#v", report)
		}
	}
}
