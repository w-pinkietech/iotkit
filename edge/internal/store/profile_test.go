package store

import (
	"strings"
	"testing"
)

func TestOpenOptionsNormalizeEmbeddedProfile(t *testing.T) {
	options, err := (OpenOptions{SQLitePath: "edge.db"}).normalized()
	if err != nil {
		t.Fatal(err)
	}
	if options.Profile != ProfileEmbedded {
		t.Fatalf("profile = %q, want %q", options.Profile, ProfileEmbedded)
	}
}

func TestOpenOptionsRejectMixedStorageConfigurationWithoutLeakingDSN(t *testing.T) {
	const secret = "must-not-leak"
	_, err := (OpenOptions{
		Profile:     ProfilePostgres,
		SQLitePath:  "edge.db",
		PostgresDSN: "postgres://iotkit:" + secret + "@localhost/iotkit",
	}).normalized()
	if err == nil {
		t.Fatal("mixed PostgreSQL and SQLite configuration was accepted")
	}
	if strings.Contains(err.Error(), secret) {
		t.Fatalf("storage validation leaked DSN secret: %v", err)
	}
}

func TestOpenOptionsRejectUnknownProfile(t *testing.T) {
	_, err := (OpenOptions{Profile: "timeseries", SQLitePath: "edge.db"}).normalized()
	if err == nil || err.Error() != "unsupported storage profile" {
		t.Fatalf("error = %v, want unsupported storage profile", err)
	}
}

func TestRebindPostgresPlaceholdersSkipsQuotedQuestionMarks(t *testing.T) {
	query := `SELECT '?', "?", value FROM readings WHERE a = ? AND note = 'it''s ?' AND b = ?`
	want := `SELECT '?', "?", value FROM readings WHERE a = $1 AND note = 'it''s ?' AND b = $2`
	if got := rebindPostgresPlaceholders(query); got != want {
		t.Fatalf("rebound query:\n%s\nwant:\n%s", got, want)
	}
}

func TestNormalizePortableSQLRewritesInsertOrIgnore(t *testing.T) {
	query := "INSERT OR IGNORE INTO readings(id) VALUES (?)"
	want := "INSERT INTO readings(id) VALUES (?) ON CONFLICT DO NOTHING"
	if got := normalizePortableSQL(query); got != want {
		t.Fatalf("normalized query = %q, want %q", got, want)
	}
}
