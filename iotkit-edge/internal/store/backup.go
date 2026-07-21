package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
)

type BackupCursor = edgeapp.BackupCursor

type BackupSnapshotInfo struct {
	StorageProfile Profile        `json:"storage_profile"`
	PayloadFormat  string         `json:"payload_format"`
	EdgeID         string         `json:"edge_id"`
	SchemaVersion  int            `json:"schema_version"`
	RawRecordCount int64          `json:"raw_record_count"`
	Cursors        []BackupCursor `json:"cursors"`
}

func (store *Store) CreateConsistentSnapshot(
	ctx context.Context,
	destination string,
) (BackupSnapshotInfo, error) {
	if destination == "" {
		return BackupSnapshotInfo{}, errors.New("snapshot destination is required")
	}
	if _, err := os.Lstat(destination); err == nil {
		return BackupSnapshotInfo{}, errors.New("snapshot destination already exists")
	} else if !errors.Is(err, os.ErrNotExist) {
		return BackupSnapshotInfo{}, fmt.Errorf("inspect snapshot destination: %w", err)
	}
	if store.profile == ProfilePostgres {
		return store.createPostgresSnapshot(ctx, destination)
	}
	stagingDirectory, err := os.MkdirTemp(filepath.Dir(destination), ".iotkit-edge-snapshot-*")
	if err != nil {
		return BackupSnapshotInfo{}, fmt.Errorf("create protected Edge snapshot staging directory: %w", err)
	}
	defer os.RemoveAll(stagingDirectory)
	stagingPath := filepath.Join(stagingDirectory, "snapshot.db")
	if _, err := store.db.ExecContext(ctx, "VACUUM INTO ?", stagingPath); err != nil {
		return BackupSnapshotInfo{}, fmt.Errorf("create consistent Edge snapshot: %w", err)
	}
	if err := os.Chmod(stagingPath, 0o600); err != nil {
		return BackupSnapshotInfo{}, fmt.Errorf("protect Edge snapshot: %w", err)
	}
	info, err := inspectSnapshot(ctx, stagingPath)
	if err != nil {
		return BackupSnapshotInfo{}, err
	}
	if err := os.Link(stagingPath, destination); err != nil {
		return BackupSnapshotInfo{}, fmt.Errorf("publish Edge snapshot: %w", err)
	}
	if err := os.Remove(stagingPath); err != nil {
		_ = os.Remove(destination)
		return BackupSnapshotInfo{}, fmt.Errorf("finish Edge snapshot publication: %w", err)
	}
	info.StorageProfile = ProfileEmbedded
	info.PayloadFormat = "sqlite-database"
	return info, nil
}

func inspectSnapshot(ctx context.Context, path string) (BackupSnapshotInfo, error) {
	db, err := sql.Open("sqlite", "file:"+path+"?mode=ro")
	if err != nil {
		return BackupSnapshotInfo{}, err
	}
	defer db.Close()
	var quickCheck string
	if err := db.QueryRowContext(ctx, "PRAGMA quick_check").Scan(&quickCheck); err != nil {
		return BackupSnapshotInfo{}, fmt.Errorf("check Edge snapshot: %w", err)
	}
	if quickCheck != "ok" {
		return BackupSnapshotInfo{}, errors.New("Edge snapshot integrity check failed")
	}
	var info BackupSnapshotInfo
	if err := db.QueryRowContext(ctx, "PRAGMA user_version").Scan(&info.SchemaVersion); err != nil {
		return BackupSnapshotInfo{}, err
	}
	if err := db.QueryRowContext(ctx,
		"SELECT edge_id FROM edge_meta WHERE singleton = 1").Scan(&info.EdgeID); err != nil {
		return BackupSnapshotInfo{}, err
	}
	if err := db.QueryRowContext(ctx,
		"SELECT count(*) FROM raw_records").Scan(&info.RawRecordCount); err != nil {
		return BackupSnapshotInfo{}, err
	}
	rows, err := db.QueryContext(ctx, `
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
	info.Cursors = make([]BackupCursor, 0)
	for rows.Next() {
		var cursor BackupCursor
		if err := rows.Scan(
			&cursor.EdgeNodeID, &cursor.LedgerEpoch, &cursor.AcceptedThrough,
		); err != nil {
			return BackupSnapshotInfo{}, err
		}
		info.Cursors = append(info.Cursors, cursor)
	}
	return info, rows.Err()
}
