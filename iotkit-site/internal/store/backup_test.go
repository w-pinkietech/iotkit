package store

import (
	"bytes"
	"context"
	"database/sql"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

const testBackupPassphrase = "工場バックアップを守る十分に長い合言葉"

func TestCreateConsistentSnapshotIncludesCommittedStateAndIdentity(t *testing.T) {
	archive := openTestStore(t)
	seedHistorySignal(t, archive, "edge-a", "signal-a", "device-01:temperature_c:na:primary")
	seedHistoryRecord(t, archive, "edge-a", "epoch-a", "device-01:temperature_c:na:primary", 1, 1_000, 20)
	destination := filepath.Join(t.TempDir(), "snapshot.db")

	info, err := archive.CreateConsistentSnapshot(context.Background(), destination)
	if err != nil {
		t.Fatal(err)
	}
	if info.SiteID == "" || info.SchemaVersion != 28 || info.RawRecordCount != 1 {
		t.Fatalf("snapshot info = %#v", info)
	}

	db, err := sql.Open("sqlite", destination)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	var quickCheck string
	if err := db.QueryRow("PRAGMA quick_check").Scan(&quickCheck); err != nil || quickCheck != "ok" {
		t.Fatalf("quick_check=%q err=%v", quickCheck, err)
	}
	var count int
	if err := db.QueryRow("SELECT count(*) FROM raw_records").Scan(&count); err != nil || count != 1 {
		t.Fatalf("raw count=%d err=%v", count, err)
	}
}

func TestCreateConsistentSnapshotRefusesExistingDestination(t *testing.T) {
	archive := openTestStore(t)
	destination := filepath.Join(t.TempDir(), "snapshot.db")
	if err := os.WriteFile(destination, []byte("keep"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.CreateConsistentSnapshot(context.Background(), destination); err == nil {
		t.Fatal("snapshot overwrote an existing destination")
	}
	content, err := os.ReadFile(destination)
	if err != nil || string(content) != "keep" {
		t.Fatalf("existing destination changed: %q err=%v", content, err)
	}
}

func TestSnapshotIncludesZeroCursorForActiveEdgeWithoutAcceptedRecords(t *testing.T) {
	archive := openTestStore(t)
	if _, err := archive.db.Exec(`
		INSERT INTO edge_activations(
			edge_ref, edge_node_id, ledger_epoch, state,
			revision, created_at, updated_at
		) VALUES('edge_test', 'edge-a', 'epoch-a', 'active', 1, 1, 1)
	`); err != nil {
		t.Fatal(err)
	}
	info, err := archive.CreateConsistentSnapshot(
		context.Background(), filepath.Join(t.TempDir(), "snapshot.db"),
	)
	if err != nil {
		t.Fatal(err)
	}
	if len(info.Cursors) != 1 || info.Cursors[0].AcceptedThrough != 0 || info.Cursors[0].EdgeNodeID != "edge-a" {
		t.Fatalf("snapshot cursors = %#v", info.Cursors)
	}
}

func TestEncryptedBackupRoundTripRestoresNewDatabaseAndRevokesSessions(t *testing.T) {
	ctx := context.Background()
	archive := openTestStore(t)
	seedHistorySignal(t, archive, "edge-a", "signal-a", "device-01:temperature_c:na:primary")
	seedHistoryRecord(t, archive, "edge-a", "epoch-a", "device-01:temperature_c:na:primary", 1, 1_000, 20)
	if _, err := archive.db.Exec(`
		INSERT INTO accepted_cursors(edge_node_id, ledger_epoch, accepted_through, updated_at)
		VALUES('edge-a', 'epoch-a', 1, 1000)
	`); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.db.Exec(`
		INSERT INTO site_accounts(
			account_ref, login_id, login_id_normalized, display_name,
			password_phc, role, state, must_change_password,
			created_at, updated_at, revision
		) VALUES('acct-test', 'owner', 'owner', '管理者', '$argon2id$test',
			'system_admin', 'active', 0, 1000, 1000, 1)
	`); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.db.Exec(`
		INSERT INTO site_sessions(
			session_ref, token_sha256, csrf_sha256, account_ref,
			issued_at, last_seen_at, idle_expires_at, absolute_expires_at
		) VALUES('session-test', zeroblob(32), randomblob(32), 'acct-test',
			1000, 1000, 2000, 3000)
	`); err != nil {
		t.Fatal(err)
	}

	dir := t.TempDir()
	backupPath := filepath.Join(dir, "site.iotkit-backup")
	manifest, err := archive.ApplyEncryptedBackup(ctx, siteapp.LocalCLIActor(), backupPath, testBackupPassphrase)
	if err != nil {
		t.Fatal(err)
	}
	if manifest.SiteID == "" || manifest.RawRecordCount != 1 || manifest.DatabaseSHA256 == "" {
		t.Fatalf("backup manifest = %#v", manifest)
	}
	container, err := os.ReadFile(backupPath)
	if err != nil {
		t.Fatal(err)
	}
	for _, plaintext := range [][]byte{[]byte("SQLite format 3"), []byte("device-01:temperature_c")} {
		if bytes.Contains(container, plaintext) {
			t.Fatalf("encrypted backup exposes plaintext %q", plaintext)
		}
	}
	if info, err := os.Stat(backupPath); err != nil || info.Mode().Perm() != 0o600 {
		t.Fatalf("backup mode=%v err=%v", info.Mode().Perm(), err)
	}

	restoredPath := filepath.Join(dir, "restored.db")
	restoredManifest, err := RestoreEncryptedBackup(ctx, backupPath, restoredPath, testBackupPassphrase)
	if err != nil {
		t.Fatal(err)
	}
	if restoredManifest.DatabaseSHA256 != manifest.DatabaseSHA256 || restoredManifest.SiteID != manifest.SiteID {
		t.Fatalf("restored manifest = %#v, want %#v", restoredManifest, manifest)
	}

	restored, err := Open(restoredPath)
	if err != nil {
		t.Fatal(err)
	}
	defer restored.Close()
	var rawCount, activeSessions, backupEvents, restoreEvents, pendingChecks int
	if err := restored.db.QueryRow("SELECT count(*) FROM raw_records").Scan(&rawCount); err != nil {
		t.Fatal(err)
	}
	if err := restored.db.QueryRow("SELECT count(*) FROM site_sessions WHERE revoked_at IS NULL").Scan(&activeSessions); err != nil {
		t.Fatal(err)
	}
	if err := restored.db.QueryRow("SELECT count(*) FROM site_backup_events").Scan(&backupEvents); err != nil {
		t.Fatal(err)
	}
	if err := restored.db.QueryRow("SELECT count(*) FROM site_restore_events").Scan(&restoreEvents); err != nil {
		t.Fatal(err)
	}
	if err := restored.db.QueryRow("SELECT count(*) FROM site_restore_cursor_checks WHERE state = 'pending'").Scan(&pendingChecks); err != nil {
		t.Fatal(err)
	}
	if rawCount != 1 || activeSessions != 0 || backupEvents != 1 || restoreEvents != 1 || pendingChecks != 1 {
		t.Fatalf("restored state raw=%d active_sessions=%d backups=%d events=%d pending=%d", rawCount, activeSessions, backupEvents, restoreEvents, pendingChecks)
	}
}

func TestEncryptedBackupRejectsWrongPassphraseTamperingAndExistingDestination(t *testing.T) {
	ctx := context.Background()
	archive := openTestStore(t)
	dir := t.TempDir()
	backupPath := filepath.Join(dir, "site.iotkit-backup")
	if _, err := archive.ApplyEncryptedBackup(ctx, siteapp.LocalCLIActor(), backupPath, testBackupPassphrase); err != nil {
		t.Fatal(err)
	}

	wrongDestination := filepath.Join(dir, "wrong.db")
	if _, err := RestoreEncryptedBackup(ctx, backupPath, wrongDestination, "これは間違った十分に長い合言葉です"); err == nil {
		t.Fatal("restore accepted the wrong passphrase")
	}
	if _, err := os.Stat(wrongDestination); !os.IsNotExist(err) {
		t.Fatalf("failed restore left destination behind: %v", err)
	}

	tampered, err := os.ReadFile(backupPath)
	if err != nil {
		t.Fatal(err)
	}
	tampered[len(tampered)-1] ^= 0xff
	tamperedPath := filepath.Join(dir, "tampered.iotkit-backup")
	if err := os.WriteFile(tamperedPath, tampered, 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := RestoreEncryptedBackup(ctx, tamperedPath, filepath.Join(dir, "tampered.db"), testBackupPassphrase); err == nil {
		t.Fatal("restore accepted a tampered backup")
	}

	existing := filepath.Join(dir, "existing.db")
	if err := os.WriteFile(existing, []byte("keep"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := RestoreEncryptedBackup(ctx, backupPath, existing, testBackupPassphrase); err == nil {
		t.Fatal("restore overwrote an existing database")
	}
	content, _ := os.ReadFile(existing)
	if string(content) != "keep" {
		t.Fatalf("existing destination changed: %q", content)
	}
}

func TestEncryptedBackupRequiresStrongPassphraseAndNewDestination(t *testing.T) {
	archive := openTestStore(t)
	destination := filepath.Join(t.TempDir(), "site.iotkit-backup")
	if _, err := archive.ApplyEncryptedBackup(context.Background(), siteapp.LocalCLIActor(), destination, "short"); err == nil || !strings.Contains(err.Error(), "12") {
		t.Fatalf("weak passphrase error = %v", err)
	}
	if _, err := os.Stat(destination); !os.IsNotExist(err) {
		t.Fatalf("weak passphrase created a backup: %v", err)
	}
}

func TestRestoredCursorGapFailsClosedUntilArchiveLossIsExplicitlyAccepted(t *testing.T) {
	ctx := context.Background()
	archive := openTestStore(t)
	first := testBatch(t)
	if _, err := acceptBatchForTest(t, archive, first); err != nil {
		t.Fatal(err)
	}
	var siteID string
	if err := archive.db.QueryRow("SELECT site_id FROM site_meta WHERE singleton = 1").Scan(&siteID); err != nil {
		t.Fatal(err)
	}
	now := time.Now().UnixMilli()
	if _, err := archive.db.Exec(`
		INSERT INTO site_restore_events(
			restore_id, restored_at, backup_created_at, backup_site_id,
			backup_schema_version, backup_sha256
		) VALUES('restore-test', ?, ?, ?, 27, ?)
	`, now, now-100, siteID, strings.Repeat("a", 64)); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.db.Exec(`
		INSERT INTO site_restore_cursor_checks(
			restore_id, edge_node_id, ledger_epoch,
			backup_accepted_through, state, updated_at
		) VALUES('restore-test', ?, ?, 1, 'pending', ?)
	`, first.EdgeNodeID, first.LedgerEpoch, now); err != nil {
		t.Fatal(err)
	}

	gap := testBatch(t)
	gap.CursorStart = 5
	gap.CursorEnd = 5
	gap.PublicationID = contract.PublicationID(gap.EdgeNodeID, gap.LedgerEpoch, 5, 5)
	gap.Records[0] = []byte(`{"family":"measurement","schema_version":1,"epoch":"epoch-01","pub_seq":5,"series_key":"series-temperature-01","values":[22.5]}`)
	if _, err := archive.AcceptBatch(ctx, gap); !errors.Is(err, ErrArchiveRecoveryRequired) {
		t.Fatalf("gap error = %v, want ErrArchiveRecoveryRequired", err)
	}
	var checkState, edgeState string
	var observedStart int64
	if err := archive.db.QueryRow(`
		SELECT state, observed_cursor_start FROM site_restore_cursor_checks
		WHERE restore_id = 'restore-test'
	`).Scan(&checkState, &observedStart); err != nil {
		t.Fatal(err)
	}
	if err := archive.db.QueryRow(`
		SELECT state FROM edge_activations WHERE edge_node_id = ?
	`, first.EdgeNodeID).Scan(&edgeState); err != nil {
		t.Fatal(err)
	}
	if checkState != "recovery_required" || observedStart != 5 || edgeState != "recovery_hold" || archive.testCursor(t) != 1 {
		t.Fatalf("recovery state check=%q start=%d edge=%q cursor=%d", checkState, observedStart, edgeState, archive.testCursor(t))
	}

	if err := archive.ApplyRestoredArchiveLoss(ctx, siteapp.LocalCLIActor(), first.EdgeNodeID, first.LedgerEpoch, siteID, "保管済みバックアップが存在しないため"); err != nil {
		t.Fatal(err)
	}
	if archive.testCursor(t) != 4 {
		t.Fatalf("accepted cursor after archive loss = %d, want 4", archive.testCursor(t))
	}
	if _, err := archive.AcceptBatch(ctx, gap); err != nil {
		t.Fatal(err)
	}
	if archive.testCursor(t) != 5 {
		t.Fatalf("accepted cursor after resumed batch = %d, want 5", archive.testCursor(t))
	}
	var finalState string
	if err := archive.db.QueryRow(`
		SELECT state FROM site_restore_cursor_checks WHERE restore_id = 'restore-test'
	`).Scan(&finalState); err != nil || finalState != "archive_lost" {
		t.Fatalf("final restore check=%q err=%v", finalState, err)
	}
	events, err := archive.ListAuditEvents(ctx, 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) != 1 || events[0].Operation != "site_restore.accept_archive_loss" {
		t.Fatalf("audit events = %#v", events)
	}
}

func TestContiguousBatchAfterRestoreVerifiesCursorWithoutRecoveryHold(t *testing.T) {
	ctx := context.Background()
	archive := openTestStore(t)
	first := testBatch(t)
	if _, err := acceptBatchForTest(t, archive, first); err != nil {
		t.Fatal(err)
	}
	now := time.Now().UnixMilli()
	if _, err := archive.db.Exec(`
		INSERT INTO site_restore_events(
			restore_id, restored_at, backup_created_at, backup_site_id,
			backup_schema_version, backup_sha256
		) SELECT 'restore-test', ?, ?, site_id, 27, ? FROM site_meta
	`, now, now-100, strings.Repeat("b", 64)); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.db.Exec(`
		INSERT INTO site_restore_cursor_checks(
			restore_id, edge_node_id, ledger_epoch,
			backup_accepted_through, state, updated_at
		) VALUES('restore-test', ?, ?, 1, 'pending', ?)
	`, first.EdgeNodeID, first.LedgerEpoch, now); err != nil {
		t.Fatal(err)
	}
	second := testBatch(t)
	second.CursorStart = 2
	second.CursorEnd = 2
	second.PublicationID = contract.PublicationID(second.EdgeNodeID, second.LedgerEpoch, 2, 2)
	second.Records[0] = []byte(`{"family":"measurement","schema_version":1,"epoch":"epoch-01","pub_seq":2,"series_key":"series-temperature-01","values":[22]}`)
	if _, err := archive.AcceptBatch(ctx, second); err != nil {
		t.Fatal(err)
	}
	var state string
	if err := archive.db.QueryRow(`SELECT state FROM site_restore_cursor_checks WHERE restore_id = 'restore-test'`).Scan(&state); err != nil {
		t.Fatal(err)
	}
	if state != "verified" {
		t.Fatalf("restore cursor state = %q, want verified", state)
	}
}
