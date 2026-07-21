package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"os"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

type BackupCursor = siteapp.BackupCursor

type BackupSnapshotInfo struct {
	SiteID         string         `json:"site_id"`
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
	if _, err := store.db.ExecContext(ctx, "VACUUM INTO ?", destination); err != nil {
		return BackupSnapshotInfo{}, fmt.Errorf("create consistent Site snapshot: %w", err)
	}
	if err := os.Chmod(destination, 0o600); err != nil {
		_ = os.Remove(destination)
		return BackupSnapshotInfo{}, fmt.Errorf("protect Site snapshot: %w", err)
	}
	info, err := inspectSnapshot(ctx, destination)
	if err != nil {
		_ = os.Remove(destination)
		return BackupSnapshotInfo{}, err
	}
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
		return BackupSnapshotInfo{}, fmt.Errorf("check Site snapshot: %w", err)
	}
	if quickCheck != "ok" {
		return BackupSnapshotInfo{}, errors.New("Site snapshot integrity check failed")
	}
	var info BackupSnapshotInfo
	if err := db.QueryRowContext(ctx, "PRAGMA user_version").Scan(&info.SchemaVersion); err != nil {
		return BackupSnapshotInfo{}, err
	}
	if err := db.QueryRowContext(ctx,
		"SELECT site_id FROM site_meta WHERE singleton = 1").Scan(&info.SiteID); err != nil {
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
		FROM edge_activations AS activation
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
