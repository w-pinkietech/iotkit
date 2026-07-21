package store

import (
	"context"
	"path/filepath"
	"testing"
)

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
