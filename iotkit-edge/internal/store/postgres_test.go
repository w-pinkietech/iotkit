package store

import (
	"context"
	"crypto/rand"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
)

func openPostgresTestStore(t *testing.T) *Store {
	t.Helper()
	dsn := newPostgresTestDatabase(t)
	store, err := OpenWithOptions(OpenOptions{
		Profile:     ProfilePostgres,
		PostgresDSN: dsn,
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = store.Close() })
	return store
}

func newPostgresTestDatabase(t *testing.T) string {
	t.Helper()
	baseDSN := os.Getenv("IOTKIT_TEST_POSTGRES_DSN")
	if baseDSN == "" {
		t.Skip("IOTKIT_TEST_POSTGRES_DSN is not set")
	}
	databaseName := postgresTestDatabaseName(t)
	dsnURL, err := url.Parse(baseDSN)
	if err != nil {
		t.Fatal(err)
	}
	adminURL := *dsnURL
	adminURL.Path = "/postgres"
	admin, err := sql.Open("pgx", adminURL.String())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := admin.ExecContext(context.Background(), "CREATE DATABASE "+databaseName); err != nil {
		_ = admin.Close()
		t.Fatal(err)
	}
	_ = admin.Close()
	t.Cleanup(func() {
		admin, err := sql.Open("pgx", adminURL.String())
		if err != nil {
			t.Errorf("open PostgreSQL admin connection for cleanup: %v", err)
			return
		}
		defer admin.Close()
		if _, err := admin.ExecContext(context.Background(), "DROP DATABASE "+databaseName+" WITH (FORCE)"); err != nil {
			t.Errorf("drop PostgreSQL test database: %v", err)
		}
	})
	dsnURL.Path = "/" + databaseName
	return dsnURL.String()
}

func postgresTestDatabaseName(t *testing.T) string {
	t.Helper()
	var random [8]byte
	if _, err := rand.Read(random[:]); err != nil {
		t.Fatal(err)
	}
	return fmt.Sprintf("iotkit_test_%s", hex.EncodeToString(random[:]))
}

func TestPostgresOpenCreatesCurrentSchema(t *testing.T) {
	store := openPostgresTestStore(t)
	var version int
	if err := store.db.QueryRowContext(
		context.Background(),
		"SELECT version FROM edge_schema_meta WHERE singleton = 1",
	).Scan(&version); err != nil {
		t.Fatal(err)
	}
	if want := schemaMigrations[len(schemaMigrations)-1].version; version != want {
		t.Fatalf("schema version = %d, want %d", version, want)
	}
}

func TestPostgresUpgradeFromSchema28IsExplicitAndPreservesPrecisionContract(t *testing.T) {
	dsn := newPostgresTestDatabase(t)
	archive, err := OpenWithOptions(OpenOptions{Profile: ProfilePostgres, PostgresDSN: dsn})
	if err != nil {
		t.Fatal(err)
	}
	if err := archive.Close(); err != nil {
		t.Fatal(err)
	}
	db, err := sql.Open("pgx", dsn)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`
		DROP TABLE edge_storage_samples;
		ALTER TABLE signal_calibration_revisions_v3
			ALTER COLUMN scale TYPE REAL USING scale::REAL,
			ALTER COLUMN "offset" TYPE REAL USING "offset"::REAL;
		UPDATE edge_schema_meta SET version = 28 WHERE singleton = 1;
	`); err != nil {
		_ = db.Close()
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}
	upgraded, err := OpenWithOptions(OpenOptions{Profile: ProfilePostgres, PostgresDSN: dsn})
	if err != nil {
		t.Fatal(err)
	}
	defer upgraded.Close()
	var version int
	if err := upgraded.db.QueryRow("SELECT version FROM edge_schema_meta WHERE singleton = 1").Scan(&version); err != nil {
		t.Fatal(err)
	}
	if version != 29 {
		t.Fatalf("schema version = %d, want 29", version)
	}
}

func TestPostgresCustodyAcceptsReplayAndRejectsConflictAndGap(t *testing.T) {
	store := openPostgresTestStore(t)
	edgeNode := discoverTestEdge(t, store)
	_, command := requestTestActivation(t, store, edgeNode)
	if _, err := store.ApplyActivationResult(context.Background(), resultForCommand(t, command)); err != nil {
		t.Fatal(err)
	}

	batch := testBatch(t)
	ack, err := store.AcceptBatch(context.Background(), batch)
	if err != nil || ack.AcceptedThrough != 1 {
		t.Fatalf("first accept = %#v, %v", ack, err)
	}
	if replay, err := store.AcceptBatch(context.Background(), batch); err != nil || replay.AcceptedThrough != 1 {
		t.Fatalf("replay = %#v, %v", replay, err)
	}

	conflict := testBatch(t)
	conflict.Records[0] = []byte(`{"family":"measurement","schema_version":1,"epoch":"epoch-01","pub_seq":1,"series_key":"series-temperature-01","values":[99]}`)
	if _, err := store.AcceptBatch(context.Background(), conflict); !errors.Is(err, ErrConflict) {
		t.Fatalf("conflict error = %v, want ErrConflict", err)
	}

	gap := testBatch(t)
	gap.CursorStart, gap.CursorEnd = 3, 3
	gap.PublicationID = "edge-node-01:epoch-01:3:3"
	gap.Records[0] = []byte(`{"family":"measurement","schema_version":1,"epoch":"epoch-01","pub_seq":3,"series_key":"series-temperature-01","values":[22]}`)
	if _, err := store.AcceptBatch(context.Background(), gap); !errors.Is(err, ErrGap) {
		t.Fatalf("gap error = %v, want ErrGap", err)
	}
}

func TestPostgresConcurrentOverlappingBatchesNeverRegressCursor(t *testing.T) {
	archive := openPostgresTestStore(t)
	if _, err := acceptBatchForTest(t, archive, testBatch(t)); err != nil {
		t.Fatal(err)
	}

	const largestCursor = 40
	start := make(chan struct{})
	errorsCh := make(chan error, largestCursor-1)
	var wait sync.WaitGroup
	for cursorEnd := int64(2); cursorEnd <= largestCursor; cursorEnd++ {
		cursorEnd := cursorEnd
		wait.Add(1)
		go func() {
			defer wait.Done()
			<-start
			records := make([]json.RawMessage, 0, cursorEnd)
			for sequence := int64(1); sequence <= cursorEnd; sequence++ {
				value := 20.0
				if sequence == 1 {
					value = 21.5
				}
				record, err := json.Marshal(map[string]any{
					"family": "measurement", "schema_version": 1,
					"epoch": "epoch-01", "pub_seq": sequence,
					"series_key": "series-temperature-01", "values": []float64{value},
				})
				if err != nil {
					errorsCh <- err
					return
				}
				records = append(records, record)
			}
			batch := contract.RecordBatch{
				SchemaVersion: 1, EdgeNodeID: "edge-node-01", LedgerEpoch: "epoch-01",
				CursorStart: 1, CursorEnd: cursorEnd, Records: records,
			}
			batch.PublicationID = contract.PublicationID(
				batch.EdgeNodeID, batch.LedgerEpoch, batch.CursorStart, batch.CursorEnd,
			)
			_, err := archive.AcceptBatch(context.Background(), batch)
			errorsCh <- err
		}()
	}
	close(start)
	wait.Wait()
	close(errorsCh)
	for err := range errorsCh {
		if err != nil {
			t.Fatal(err)
		}
	}
	if cursor := archive.testCursor(t); cursor != largestCursor {
		t.Fatalf("accepted cursor = %d, want %d", cursor, largestCursor)
	}
}

func TestPostgresCreatesConsistentCustomSnapshot(t *testing.T) {
	store := openPostgresTestStore(t)
	if _, err := acceptBatchForTest(t, store, testBatch(t)); err != nil {
		t.Fatal(err)
	}
	destination := filepath.Join(t.TempDir(), "edge.pgdump")
	info, err := store.CreateConsistentSnapshot(context.Background(), destination)
	if err != nil {
		t.Fatal(err)
	}
	file, err := os.Stat(destination)
	if err != nil {
		t.Fatal(err)
	}
	if info.StorageProfile != ProfilePostgres ||
		info.PayloadFormat != "postgres-custom" ||
		info.RawRecordCount != 1 || file.Size() == 0 {
		t.Fatalf("PostgreSQL snapshot = %#v size=%d", info, file.Size())
	}
	if file.Mode().Perm() != 0o600 {
		t.Fatalf("PostgreSQL snapshot mode = %v", file.Mode().Perm())
	}
}

func TestPostgresEncryptedBackupIdentifiesPayloadProfile(t *testing.T) {
	store := openPostgresTestStore(t)
	destination := filepath.Join(t.TempDir(), "edge.iotkit-backup")
	manifest, err := store.ApplyEncryptedBackup(
		context.Background(), edgeapp.LocalCLIActor(), destination,
		testBackupPassphrase,
	)
	if err != nil {
		t.Fatal(err)
	}
	if manifest.StorageProfile != string(ProfilePostgres) ||
		manifest.PayloadFormat != "postgres-custom" {
		t.Fatalf("PostgreSQL backup manifest = %#v", manifest)
	}
}

func TestPostgresEncryptedBackupRestoresOnlyIntoEmptyDatabase(t *testing.T) {
	source := openPostgresTestStore(t)
	if _, err := acceptBatchForTest(t, source, testBatch(t)); err != nil {
		t.Fatal(err)
	}
	backupPath := filepath.Join(t.TempDir(), "edge.iotkit-backup")
	manifest, err := source.ApplyEncryptedBackup(
		context.Background(), edgeapp.LocalCLIActor(), backupPath,
		testBackupPassphrase,
	)
	if err != nil {
		t.Fatal(err)
	}
	targetDSN := newPostgresTestDatabase(t)
	restored, err := RestoreEncryptedBackupToPostgres(
		context.Background(), backupPath, targetDSN, testBackupPassphrase,
	)
	if err != nil {
		t.Fatal(err)
	}
	if restored.BackupID != manifest.BackupID || restored.EdgeID != manifest.EdgeID {
		t.Fatalf("restored manifest = %#v, want %#v", restored, manifest)
	}
	target, err := OpenWithOptions(OpenOptions{Profile: ProfilePostgres, PostgresDSN: targetDSN})
	if err != nil {
		t.Fatal(err)
	}
	defer target.Close()
	if records, err := target.ListRawRecords(context.Background(), 10); err != nil || len(records) != 1 {
		t.Fatalf("restored records = %#v, %v", records, err)
	}
	if _, err := RestoreEncryptedBackupToPostgres(
		context.Background(), backupPath, targetDSN, testBackupPassphrase,
	); err == nil {
		t.Fatal("non-empty PostgreSQL restore destination was accepted")
	}
}

func TestPostgresSchema28BackupRestoresAndUpgradesInsideCandidate(t *testing.T) {
	source := openPostgresTestStore(t)
	if _, err := source.db.Exec(`
		DROP TABLE edge_storage_samples;
		ALTER TABLE signal_calibration_revisions_v3
			ALTER COLUMN scale TYPE REAL USING scale::REAL,
			ALTER COLUMN "offset" TYPE REAL USING "offset"::REAL;
		UPDATE edge_schema_meta SET version = 28 WHERE singleton = 1;
	`); err != nil {
		t.Fatal(err)
	}
	backupPath := filepath.Join(t.TempDir(), "edge-v28.iotkit-backup")
	manifest, err := source.ApplyEncryptedBackup(
		context.Background(), edgeapp.LocalCLIActor(), backupPath, testBackupPassphrase,
	)
	if err != nil {
		t.Fatal(err)
	}
	if manifest.SchemaVersion != 28 {
		t.Fatalf("backup schema = %d, want 28", manifest.SchemaVersion)
	}
	targetDSN := newPostgresTestDatabase(t)
	if _, err := RestoreEncryptedBackupToPostgres(
		context.Background(), backupPath, targetDSN, testBackupPassphrase,
	); err != nil {
		t.Fatal(err)
	}
	restored, err := OpenWithOptions(OpenOptions{Profile: ProfilePostgres, PostgresDSN: targetDSN})
	if err != nil {
		t.Fatal(err)
	}
	defer restored.Close()
	var version int
	if err := restored.db.QueryRow("SELECT version FROM edge_schema_meta WHERE singleton = 1").Scan(&version); err != nil {
		t.Fatal(err)
	}
	if version != 29 {
		t.Fatalf("restored schema = %d, want 29", version)
	}
}

func TestPostgresOpenRejectsIncompleteRestore(t *testing.T) {
	dsn := newPostgresTestDatabase(t)
	if err := setPostgresRestoreState(context.Background(), dsn, "incomplete"); err != nil {
		t.Fatal(err)
	}
	archive, err := OpenWithOptions(OpenOptions{Profile: ProfilePostgres, PostgresDSN: dsn})
	if archive != nil {
		_ = archive.Close()
	}
	if err == nil || !strings.Contains(err.Error(), "restore state is not ready") {
		t.Fatalf("incomplete restore open error = %v", err)
	}
	overriddenDSN := dsn + "&options=-c%20iotkit.restore_state=ready"
	archive, err = OpenWithOptions(OpenOptions{Profile: ProfilePostgres, PostgresDSN: overriddenDSN})
	if archive != nil {
		_ = archive.Close()
	}
	if err == nil || !strings.Contains(err.Error(), "restore state is not ready") {
		t.Fatalf("session override bypass error = %v", err)
	}
}

func TestPostgresDumpConnectionRejectsSecretAndOverrideParameters(t *testing.T) {
	for _, dsn := range []string{
		"postgres://iotkit@127.0.0.1/iotkit?sslmode=require&password=visible-secret",
		"postgres://iotkit@127.0.0.1/iotkit?sslpassword=visible-secret",
		"postgres://iotkit@127.0.0.1/iotkit?options=-c%20iotkit.restore_state=ready",
		"postgres://iotkit@127.0.0.1/iotkit#unexpected",
	} {
		connection, password, err := postgresDumpConnection(dsn)
		if err == nil || connection != "" || password != "" {
			t.Fatalf("unsafe DSN result for %q = %q, %q, %v", dsn, connection, password, err)
		}
	}
}
