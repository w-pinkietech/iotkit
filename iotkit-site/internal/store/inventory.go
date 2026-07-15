package store

import (
	"context"
	"crypto/rand"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"strings"
	"time"
)

type inventorySourceCandidate struct {
	edgeNodeID string
	seriesKey  string
}

func newResourceRef(prefix string) (string, error) {
	bytes := make([]byte, 16)
	if _, err := rand.Read(bytes); err != nil {
		return "", fmt.Errorf("generate %s resource ref: %w", prefix, err)
	}
	return prefix + hex.EncodeToString(bytes), nil
}

func ensureDeviceSourceTx(ctx context.Context, tx *sql.Tx, edgeNodeID, systemID string) error {
	deviceRef, err := newResourceRef("dev_")
	if err != nil {
		return err
	}
	_, err = tx.ExecContext(ctx, `
		INSERT INTO site_devices (
			device_ref, edge_node_id, system_id, created_at
		) VALUES (?, ?, ?, ?)
		ON CONFLICT(edge_node_id, system_id) DO NOTHING
	`, deviceRef, edgeNodeID, systemID, time.Now().UnixMilli())
	return err
}

func ensureSignalSourceTx(
	ctx context.Context,
	tx *sql.Tx,
	edgeNodeID string,
	seriesKey string,
	systemID *string,
) error {
	signalRef, err := newResourceRef("sig_")
	if err != nil {
		return err
	}
	_, err = tx.ExecContext(ctx, `
		INSERT INTO site_signals (
			signal_ref, edge_node_id, series_key, system_id, created_at
		) VALUES (?, ?, ?, ?, ?)
		ON CONFLICT(edge_node_id, series_key) DO UPDATE SET
			system_id = COALESCE(excluded.system_id, site_signals.system_id)
	`, signalRef, edgeNodeID, seriesKey, systemID, time.Now().UnixMilli())
	return err
}

func (store *Store) ReconcileInventorySources(ctx context.Context, limit int) (int, error) {
	if limit < 1 || limit > 1000 {
		return 0, errors.New("inventory reconciliation limit must be between 1 and 1000")
	}
	rows, err := store.db.QueryContext(ctx, `
		SELECT r.edge_node_id,
			json_extract(CAST(r.record_json AS TEXT), '$.series_key') AS series_key
		FROM raw_records AS r
		WHERE json_extract(CAST(r.record_json AS TEXT), '$.family') = 'measurement'
			AND json_type(CAST(r.record_json AS TEXT), '$.series_key') = 'text'
			AND NOT EXISTS (
				SELECT 1 FROM site_signals AS signal
				WHERE signal.edge_node_id = r.edge_node_id
					AND signal.series_key = json_extract(CAST(r.record_json AS TEXT), '$.series_key')
			)
		GROUP BY r.edge_node_id, series_key
		ORDER BY r.edge_node_id, series_key
		LIMIT ?
	`, limit)
	if err != nil {
		return 0, err
	}
	candidates := make([]inventorySourceCandidate, 0)
	for rows.Next() {
		var candidate inventorySourceCandidate
		if err := rows.Scan(&candidate.edgeNodeID, &candidate.seriesKey); err != nil {
			_ = rows.Close()
			return 0, err
		}
		candidates = append(candidates, candidate)
	}
	if err := rows.Close(); err != nil {
		return 0, err
	}
	if err := rows.Err(); err != nil {
		return 0, err
	}
	if len(candidates) == 0 {
		return 0, nil
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return 0, err
	}
	defer func() { _ = tx.Rollback() }()
	for _, candidate := range candidates {
		var systemID *string
		if parsed, ok := systemIDFromSeriesKey(candidate.seriesKey); ok {
			systemID = &parsed
			if err := ensureDeviceSourceTx(ctx, tx, candidate.edgeNodeID, parsed); err != nil {
				return 0, err
			}
		}
		if err := ensureSignalSourceTx(
			ctx,
			tx,
			candidate.edgeNodeID,
			candidate.seriesKey,
			systemID,
		); err != nil {
			return 0, err
		}
	}
	if err := tx.Commit(); err != nil {
		return 0, err
	}
	return len(candidates), nil
}

func systemIDFromSeriesKey(seriesKey string) (string, bool) {
	systemID, _, found := strings.Cut(seriesKey, ":")
	if !found || len(systemID) != 36 || systemID[8] != '-' || systemID[13] != '-' ||
		systemID[18] != '-' || systemID[23] != '-' {
		return "", false
	}
	decoded, err := hex.DecodeString(strings.ReplaceAll(systemID, "-", ""))
	return systemID, err == nil && len(decoded) == 16
}
