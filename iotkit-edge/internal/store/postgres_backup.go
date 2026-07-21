package store

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

func (store *Store) createPostgresSnapshot(
	ctx context.Context,
	destination string,
) (BackupSnapshotInfo, error) {
	var empty BackupSnapshotInfo
	tx, err := store.db.raw.BeginTx(ctx, &sql.TxOptions{
		Isolation: sql.LevelRepeatableRead,
		ReadOnly:  true,
	})
	if err != nil {
		return empty, errors.New("begin PostgreSQL backup snapshot")
	}
	defer tx.Rollback()
	var snapshotID string
	if err := tx.QueryRowContext(ctx, "SELECT pg_export_snapshot()").Scan(&snapshotID); err != nil {
		return empty, errors.New("export PostgreSQL backup snapshot")
	}
	info, err := inspectPostgresSnapshot(ctx, tx)
	if err != nil {
		return empty, err
	}
	connection, password, err := postgresDumpConnection(store.postgresDSN)
	if err != nil {
		return empty, err
	}
	command := exec.CommandContext(ctx, "pg_dump",
		"--dbname="+connection,
		"--format=custom",
		"--file="+destination,
		"--no-owner",
		"--no-privileges",
		"--snapshot="+snapshotID,
	)
	command.Env = append(os.Environ(), "PGPASSWORD="+password)
	if output, err := command.CombinedOutput(); err != nil {
		_ = os.Remove(destination)
		if len(output) > 0 {
			return empty, fmt.Errorf("create PostgreSQL snapshot: %s", output)
		}
		return empty, errors.New("create PostgreSQL snapshot")
	}
	if err := os.Chmod(destination, 0o600); err != nil {
		_ = os.Remove(destination)
		return empty, fmt.Errorf("protect PostgreSQL snapshot: %w", err)
	}
	return info, nil
}

func inspectPostgresSnapshot(ctx context.Context, tx *sql.Tx) (BackupSnapshotInfo, error) {
	info := BackupSnapshotInfo{
		StorageProfile: ProfilePostgres,
		PayloadFormat:  "postgres-custom",
	}
	if err := tx.QueryRowContext(ctx,
		"SELECT version FROM edge_schema_meta WHERE singleton = 1",
	).Scan(&info.SchemaVersion); err != nil {
		return BackupSnapshotInfo{}, err
	}
	if err := tx.QueryRowContext(ctx,
		"SELECT edge_id FROM edge_meta WHERE singleton = 1",
	).Scan(&info.EdgeID); err != nil {
		return BackupSnapshotInfo{}, err
	}
	if err := tx.QueryRowContext(ctx,
		"SELECT count(*) FROM raw_records",
	).Scan(&info.RawRecordCount); err != nil {
		return BackupSnapshotInfo{}, err
	}
	rows, err := tx.QueryContext(ctx, `
		SELECT edge_node_id, ledger_epoch, accepted_through FROM accepted_cursors
		UNION ALL
		SELECT activation.edge_node_id, activation.ledger_epoch, 0
		FROM edge_node_activations AS activation
		WHERE activation.state = 'active'
			AND NOT EXISTS (
				SELECT 1 FROM accepted_cursors AS cursor
				WHERE cursor.edge_node_id = activation.edge_node_id
					AND cursor.ledger_epoch = activation.ledger_epoch
			)
		ORDER BY edge_node_id, ledger_epoch
	`)
	if err != nil {
		return BackupSnapshotInfo{}, err
	}
	defer rows.Close()
	for rows.Next() {
		var cursor BackupCursor
		if err := rows.Scan(&cursor.EdgeNodeID, &cursor.LedgerEpoch, &cursor.AcceptedThrough); err != nil {
			return BackupSnapshotInfo{}, err
		}
		info.Cursors = append(info.Cursors, cursor)
	}
	return info, rows.Err()
}

func postgresDumpConnection(dsn string) (string, string, error) {
	parsed, err := url.Parse(dsn)
	if err != nil || (parsed.Scheme != "postgres" && parsed.Scheme != "postgresql") {
		return "", "", errors.New("PostgreSQL backup requires a URL connection configuration")
	}
	password, _ := parsed.User.Password()
	if parsed.User != nil {
		parsed.User = url.User(parsed.User.Username())
	}
	return parsed.String(), password, nil
}

func RestoreEncryptedBackupToPostgres(
	ctx context.Context,
	source string,
	targetDSN string,
	passphrase string,
) (BackupManifest, error) {
	var empty BackupManifest
	if err := validateBackupPassphrase(passphrase); err != nil {
		return empty, err
	}
	if err := ensurePostgresRestoreTargetEmpty(ctx, targetDSN); err != nil {
		return empty, err
	}
	payload, err := os.CreateTemp(filepath.Dir(source), ".iotkit-edge-postgres-restore-*")
	if err != nil {
		return empty, err
	}
	payloadPath := payload.Name()
	defer os.Remove(payloadPath)
	if err := payload.Chmod(0o600); err != nil {
		_ = payload.Close()
		return empty, err
	}
	if err := decryptBackupContainer(source, payload, passphrase); err != nil {
		_ = payload.Close()
		return empty, err
	}
	if _, err := payload.Seek(0, io.SeekStart); err != nil {
		_ = payload.Close()
		return empty, err
	}
	manifest, err := readBackupManifest(payload)
	if err != nil {
		_ = payload.Close()
		return empty, err
	}
	if manifest.StorageProfile != string(ProfilePostgres) || manifest.PayloadFormat != "postgres-custom" {
		_ = payload.Close()
		return empty, errors.New("backup storage profile does not match PostgreSQL restore destination")
	}
	dump, err := os.CreateTemp(filepath.Dir(source), ".iotkit-edge-postgres-dump-*")
	if err != nil {
		_ = payload.Close()
		return empty, err
	}
	dumpPath := dump.Name()
	defer os.Remove(dumpPath)
	if err := dump.Chmod(0o600); err != nil {
		_ = payload.Close()
		_ = dump.Close()
		return empty, err
	}
	hash := sha256.New()
	if _, err := io.Copy(io.MultiWriter(dump, hash), payload); err != nil {
		_ = payload.Close()
		_ = dump.Close()
		return empty, err
	}
	_ = payload.Close()
	if err := dump.Sync(); err != nil {
		_ = dump.Close()
		return empty, err
	}
	if err := dump.Close(); err != nil {
		return empty, err
	}
	if got := hex.EncodeToString(hash.Sum(nil)); !strings.EqualFold(got, manifest.DatabaseSHA256) {
		return empty, errors.New("Edge backup database checksum does not match its manifest")
	}
	connection, password, err := postgresDumpConnection(targetDSN)
	if err != nil {
		return empty, err
	}
	command := exec.CommandContext(ctx, "pg_restore",
		"--dbname="+connection,
		"--no-owner",
		"--no-privileges",
		"--exit-on-error",
		"--single-transaction",
		dumpPath,
	)
	command.Env = append(os.Environ(), "PGPASSWORD="+password)
	if err := command.Run(); err != nil {
		return empty, errors.New("restore PostgreSQL snapshot")
	}
	restored, err := OpenWithOptions(OpenOptions{
		Profile: ProfilePostgres, PostgresDSN: targetDSN,
	})
	if err != nil {
		return empty, err
	}
	defer restored.Close()
	tx, err := restored.db.raw.BeginTx(ctx, &sql.TxOptions{
		Isolation: sql.LevelRepeatableRead, ReadOnly: true,
	})
	if err != nil {
		return empty, err
	}
	info, err := inspectPostgresSnapshot(ctx, tx)
	_ = tx.Rollback()
	if err != nil {
		return empty, err
	}
	if err := validateSnapshotInfo(info, manifest); err != nil {
		return empty, err
	}
	if err := prepareRestoredStore(ctx, restored, manifest); err != nil {
		return empty, err
	}
	return manifest, nil
}

func ensurePostgresRestoreTargetEmpty(ctx context.Context, dsn string) error {
	db, err := sql.Open("pgx", dsn)
	if err != nil {
		return errors.New("open PostgreSQL restore destination")
	}
	defer db.Close()
	var tables int
	if err := db.QueryRowContext(ctx, `
		SELECT count(*) FROM pg_catalog.pg_tables
		WHERE schemaname NOT IN ('pg_catalog', 'information_schema')
	`).Scan(&tables); err != nil {
		return errors.New("inspect PostgreSQL restore destination")
	}
	if tables != 0 {
		return errors.New("PostgreSQL restore destination is not empty")
	}
	return nil
}

func validateSnapshotInfo(info BackupSnapshotInfo, manifest BackupManifest) error {
	if info.EdgeID != manifest.EdgeID || info.SchemaVersion != manifest.SchemaVersion ||
		info.RawRecordCount != manifest.RawRecordCount {
		return errors.New("Edge backup manifest does not match the restored database")
	}
	if len(info.Cursors) != len(manifest.Cursors) {
		return errors.New("Edge backup cursor manifest does not match the restored database")
	}
	for index := range info.Cursors {
		if info.Cursors[index] != manifest.Cursors[index] {
			return errors.New("Edge backup cursor manifest does not match the restored database")
		}
	}
	return nil
}
