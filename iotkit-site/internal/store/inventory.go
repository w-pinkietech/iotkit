package store

import (
	"bytes"
	"context"
	"crypto/rand"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"strings"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
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

func (store *Store) ListInventoryDevices(
	ctx context.Context,
	limit int,
	afterRef string,
) ([]siteapp.DeviceSummary, error) {
	if err := validateInventoryPage(limit, afterRef, "dev_"); err != nil {
		return nil, err
	}
	rows, err := store.db.QueryContext(ctx, `
		SELECT source.device_ref,
			COALESCE(profile.display_name, ''),
			COALESCE(profile.location, ''),
			profile.revision,
			COALESCE(descriptor.presence, 'unknown'),
			COALESCE(descriptor.state, 'unknown'),
			source.edge_node_id,
			source.system_id
		FROM site_devices AS source
		LEFT JOIN device_profiles AS profile
			ON profile.edge_node_id = source.edge_node_id
			AND profile.system_id = source.system_id
		LEFT JOIN descriptor_devices AS descriptor
			ON descriptor.edge_node_id = source.edge_node_id
			AND descriptor.system_id = source.system_id
		WHERE source.device_ref > ?
		ORDER BY source.device_ref
		LIMIT ?
	`, afterRef, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	type deviceRow struct {
		summary    siteapp.DeviceSummary
		edgeNodeID string
		systemID   string
	}
	resultRows := make([]deviceRow, 0)
	for rows.Next() {
		var row deviceRow
		var revision sql.NullInt64
		if err := rows.Scan(
			&row.summary.DeviceRef,
			&row.summary.DisplayName,
			&row.summary.Location,
			&revision,
			&row.summary.DescriptorPresence,
			&row.summary.DeviceState,
			&row.edgeNodeID,
			&row.systemID,
		); err != nil {
			return nil, err
		}
		if revision.Valid {
			row.summary.ProfileRevision = &revision.Int64
		}
		resultRows = append(resultRows, row)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	if err := rows.Close(); err != nil {
		return nil, err
	}
	result := make([]siteapp.DeviceSummary, 0, len(resultRows))
	for _, row := range resultRows {
		lastReceivedAt, err := store.latestDeviceReceivedAt(ctx, row.edgeNodeID, row.systemID)
		if err != nil {
			return nil, err
		}
		row.summary.LastReceivedAt = lastReceivedAt
		result = append(result, row.summary)
	}
	return result, nil
}

func (store *Store) ListInventorySignals(
	ctx context.Context,
	limit int,
	afterRef string,
) ([]siteapp.SignalSummary, error) {
	if err := validateInventoryPage(limit, afterRef, "sig_"); err != nil {
		return nil, err
	}
	rows, err := store.db.QueryContext(ctx, `
		SELECT source.signal_ref,
			device.device_ref,
			COALESCE(profile.display_name, ''),
			profile.revision,
			COALESCE(descriptor.presence, 'unknown'),
			descriptor.unit,
			descriptor.value_type,
			EXISTS (
				SELECT 1 FROM semantic_mappings AS mapping
				WHERE mapping.edge_node_id = source.edge_node_id
					AND mapping.series_key = source.series_key
					AND mapping.active = 1
			),
			source.edge_node_id,
			source.series_key
		FROM site_signals AS source
		LEFT JOIN site_devices AS device
			ON device.edge_node_id = source.edge_node_id
			AND device.system_id = source.system_id
		LEFT JOIN signal_profiles AS profile
			ON profile.edge_node_id = source.edge_node_id
			AND profile.series_key = source.series_key
		LEFT JOIN descriptor_signals AS descriptor
			ON descriptor.edge_node_id = source.edge_node_id
			AND descriptor.series_key = source.series_key
		WHERE source.signal_ref > ?
		ORDER BY source.signal_ref
		LIMIT ?
	`, afterRef, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	type signalRow struct {
		summary    siteapp.SignalSummary
		edgeNodeID string
		seriesKey  string
	}
	resultRows := make([]signalRow, 0)
	for rows.Next() {
		var row signalRow
		var deviceRef sql.NullString
		var revision sql.NullInt64
		var unit sql.NullString
		var valueType sql.NullString
		if err := rows.Scan(
			&row.summary.SignalRef,
			&deviceRef,
			&row.summary.DisplayName,
			&revision,
			&row.summary.DescriptorPresence,
			&unit,
			&valueType,
			&row.summary.HasSemanticMapping,
			&row.edgeNodeID,
			&row.seriesKey,
		); err != nil {
			return nil, err
		}
		if deviceRef.Valid {
			row.summary.DeviceRef = &deviceRef.String
		}
		if revision.Valid {
			row.summary.ProfileRevision = &revision.Int64
		}
		if unit.Valid {
			row.summary.Unit = &unit.String
		}
		if valueType.Valid {
			row.summary.ValueType = &valueType.String
		}
		resultRows = append(resultRows, row)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	if err := rows.Close(); err != nil {
		return nil, err
	}
	result := make([]siteapp.SignalSummary, 0, len(resultRows))
	for _, row := range resultRows {
		latest, err := store.latestValidMeasurement(ctx, row.edgeNodeID, row.seriesKey)
		if err != nil {
			return nil, err
		}
		row.summary.Latest = latest
		result = append(result, row.summary)
	}
	return result, nil
}

func (store *Store) latestDeviceReceivedAt(
	ctx context.Context,
	edgeNodeID string,
	systemID string,
) (*int64, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT series_key FROM site_signals
		WHERE edge_node_id = ? AND system_id = ?
		ORDER BY signal_ref
	`, edgeNodeID, systemID)
	if err != nil {
		return nil, err
	}
	seriesKeys := make([]string, 0)
	for rows.Next() {
		var seriesKey string
		if err := rows.Scan(&seriesKey); err != nil {
			_ = rows.Close()
			return nil, err
		}
		seriesKeys = append(seriesKeys, seriesKey)
	}
	if err := rows.Close(); err != nil {
		return nil, err
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	var latestReceivedAt *int64
	for _, seriesKey := range seriesKeys {
		measurement, err := store.latestValidMeasurement(ctx, edgeNodeID, seriesKey)
		if err != nil {
			return nil, err
		}
		if measurement != nil &&
			(latestReceivedAt == nil || measurement.SiteReceivedAt > *latestReceivedAt) {
			value := measurement.SiteReceivedAt
			latestReceivedAt = &value
		}
	}
	return latestReceivedAt, nil
}

func (store *Store) latestValidMeasurement(
	ctx context.Context,
	edgeNodeID string,
	seriesKey string,
) (*siteapp.LatestMeasurement, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT record_json, received_at
		FROM raw_records
		WHERE edge_node_id = ?
			AND json_extract(CAST(record_json AS TEXT), '$.family') = 'measurement'
			AND json_extract(CAST(record_json AS TEXT), '$.series_key') = ?
		ORDER BY received_at DESC, ledger_epoch DESC, pub_seq DESC
		LIMIT 32
	`, edgeNodeID, seriesKey)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	for rows.Next() {
		var record []byte
		var siteReceivedAt int64
		if err := rows.Scan(&record, &siteReceivedAt); err != nil {
			return nil, err
		}
		measurement, valid := decodeInventoryMeasurement(record, siteReceivedAt)
		if valid {
			return &measurement, nil
		}
	}
	return nil, rows.Err()
}

func decodeInventoryMeasurement(
	record []byte,
	siteReceivedAt int64,
) (siteapp.LatestMeasurement, bool) {
	var noMeasurement siteapp.LatestMeasurement
	var payload struct {
		Family    string          `json:"family"`
		Values    json.RawMessage `json:"values"`
		EventTime json.RawMessage `json:"event_time"`
	}
	if err := json.Unmarshal(record, &payload); err != nil || payload.Family != "measurement" {
		return noMeasurement, false
	}
	var values []json.RawMessage
	if err := json.Unmarshal(payload.Values, &values); err != nil || len(values) == 0 {
		return noMeasurement, false
	}
	for _, raw := range values {
		value, err := json.Number(raw).Float64()
		if err != nil || math.IsNaN(value) || math.IsInf(value, 0) {
			return noMeasurement, false
		}
	}
	var eventTime int64
	if err := json.Unmarshal(payload.EventTime, &eventTime); err != nil || eventTime < 0 {
		return noMeasurement, false
	}
	var compact bytes.Buffer
	if err := json.Compact(&compact, payload.Values); err != nil {
		return noMeasurement, false
	}
	return siteapp.LatestMeasurement{
		Values:         append(json.RawMessage(nil), compact.Bytes()...),
		EventTime:      eventTime,
		SiteReceivedAt: siteReceivedAt,
	}, true
}

func validateInventoryPage(limit int, afterRef, prefix string) error {
	if limit < 1 || limit > 100 {
		return errors.New("inventory page limit must be between 1 and 100")
	}
	if afterRef != "" {
		if !strings.HasPrefix(afterRef, prefix) {
			return errors.New("inventory page cursor has wrong resource type")
		}
		randomPart := strings.TrimPrefix(afterRef, prefix)
		if len(randomPart) != 32 {
			return errors.New("invalid inventory page cursor")
		}
		if _, err := hex.DecodeString(randomPart); err != nil {
			return errors.New("invalid inventory page cursor")
		}
	}
	return nil
}
