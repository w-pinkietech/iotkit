package store

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSQLiteDeploymentLockRejectsConcurrentOwner(t *testing.T) {
	path := filepath.Join(t.TempDir(), "edge.db")
	if err := os.WriteFile(path, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	first, err := AcquireSQLiteDeploymentLock(path)
	if err != nil {
		t.Fatal(err)
	}
	defer first.Close()
	second, err := AcquireSQLiteDeploymentLock(path)
	if second != nil {
		_ = second.Close()
	}
	if err == nil || !strings.Contains(err.Error(), "in use") {
		t.Fatalf("second deployment lock error = %v", err)
	}
}

func TestSQLiteDeploymentLockCannotBeBypassedWithSymlinkAlias(t *testing.T) {
	path := filepath.Join(t.TempDir(), "edge.db")
	archive, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer archive.Close()
	alias := filepath.Join(t.TempDir(), "edge-alias.db")
	if err := os.Symlink(path, alias); err != nil {
		t.Fatal(err)
	}
	lock, err := AcquireSQLiteDeploymentLock(alias)
	if lock != nil {
		_ = lock.Close()
	}
	if err == nil || !strings.Contains(err.Error(), "in use") {
		t.Fatalf("symlink migration lock error = %v", err)
	}
}

func TestSQLiteMigrationRejectsDatabaseHeldByRunningEdge(t *testing.T) {
	path := filepath.Join(t.TempDir(), "edge.db")
	archive, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := archive.Close(); err != nil {
		t.Fatal(err)
	}
	deploymentLock, err := AcquireSQLiteDeploymentLock(path)
	if err != nil {
		t.Fatal(err)
	}
	defer deploymentLock.Close()
	_, err = MigrateSQLiteToPostgres(context.Background(), path, "unused")
	if err == nil || !strings.Contains(err.Error(), "in use") {
		t.Fatalf("live SQLite migration error = %v", err)
	}
}

func TestMigrateSQLiteToPostgresCopiesAndVerifiesCustody(t *testing.T) {
	sourcePath := filepath.Join(t.TempDir(), "edge.db")
	source, err := OpenWithEdgeID(sourcePath, "edge-0123456789abcdef0123456789abcdef")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := acceptBatchForTest(t, source, testBatch(t)); err != nil {
		t.Fatal(err)
	}
	if err := source.Close(); err != nil {
		t.Fatal(err)
	}

	targetDSN := newPostgresTestDatabase(t)
	report, err := MigrateSQLiteToPostgres(context.Background(), sourcePath, targetDSN)
	if err != nil {
		t.Fatal(err)
	}
	if !report.Completed || report.EdgeID != "edge-0123456789abcdef0123456789abcdef" ||
		report.TableCounts["raw_records"] != 1 || report.ContentDigest == "" {
		t.Fatalf("migration report = %#v", report)
	}
	target, err := OpenWithOptions(OpenOptions{
		Profile: ProfilePostgres, PostgresDSN: targetDSN, EdgeID: report.EdgeID,
	})
	if err != nil {
		t.Fatal(err)
	}
	defer target.Close()
	records, err := target.ListRawRecords(context.Background(), 10)
	if err != nil || len(records) != 1 || records[0].PubSeq != 1 {
		t.Fatalf("migrated raw records = %#v, %v", records, err)
	}
}

func TestMigrateSQLiteToPostgresRejectsNonEmptyTarget(t *testing.T) {
	sourcePath := filepath.Join(t.TempDir(), "edge.db")
	source, err := Open(sourcePath)
	if err != nil {
		t.Fatal(err)
	}
	if err := source.Close(); err != nil {
		t.Fatal(err)
	}
	target := openPostgresTestStore(t)
	if _, err := target.db.Exec(`
		INSERT INTO edge_accounts(
			account_ref, login_id, login_id_normalized, display_name,
			password_phc, role, state, must_change_password,
			created_at, updated_at, revision
		) VALUES('acct-existing', 'existing', 'existing', 'Existing',
			'phc', 'viewer', 'active', 0, 1, 1, 1)
	`); err != nil {
		t.Fatal(err)
	}
	if _, err := MigrateSQLiteToPostgres(
		context.Background(), sourcePath, target.postgresDSN,
	); err == nil {
		t.Fatal("non-empty PostgreSQL migration target was accepted")
	}
}

func TestMigrateSQLiteToPostgresRejectsTargetHeldByRunningEdge(t *testing.T) {
	sourcePath := filepath.Join(t.TempDir(), "edge.db")
	source, err := Open(sourcePath)
	if err != nil {
		t.Fatal(err)
	}
	if err := source.Close(); err != nil {
		t.Fatal(err)
	}

	target := openPostgresTestStore(t)
	defer target.Close()
	_, err = MigrateSQLiteToPostgres(context.Background(), sourcePath, target.postgresDSN)
	if err == nil || !strings.Contains(err.Error(), "in use by another IoTKit process") {
		t.Fatalf("live PostgreSQL migration target error = %v", err)
	}
}
