package store

import (
	"context"
	"path/filepath"
	"testing"
)

func TestOpenAppliesMigrationsWithoutDroppingExistingData(t *testing.T) {
	path := filepath.Join(t.TempDir(), "site.db")
	first, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := first.AcceptBatch(context.Background(), testBatch(t)); err != nil {
		t.Fatal(err)
	}
	if _, err := first.db.Exec("PRAGMA user_version = 0"); err != nil {
		t.Fatal(err)
	}
	if err := first.Close(); err != nil {
		t.Fatal(err)
	}

	reopened, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = reopened.Close() })
	records, err := reopened.ListRawRecords(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != 1 {
		t.Fatalf("records = %d, want 1", len(records))
	}
	var version int
	if err := reopened.db.QueryRow("PRAGMA user_version").Scan(&version); err != nil {
		t.Fatal(err)
	}
	if version != 2 {
		t.Fatalf("schema version = %d, want 2", version)
	}
}
