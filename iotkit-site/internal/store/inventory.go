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

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

type pendingInventoryEpoch struct {
	edgeNodeID      string
	ledgerEpoch     string
	lastPubSeq      int64
	acceptedThrough int64
}

type pendingInventoryRecord struct {
	edgeNodeID  string
	ledgerEpoch string
	pubSeq      int64
	record      []byte
	receivedAt  int64
}

type inventoryEpochKey struct {
	edgeNodeID  string
	ledgerEpoch string
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
			signal_ref, edge_node_id, series_key, system_id, last_received_at, created_at
		) VALUES (?, ?, ?, ?, NULL, ?)
		ON CONFLICT(edge_node_id, series_key) DO UPDATE SET
			system_id = COALESCE(excluded.system_id, site_signals.system_id)
	`, signalRef, edgeNodeID, seriesKey, systemID, time.Now().UnixMilli())
	return err
}

func (store *Store) ReconcileInventorySources(ctx context.Context, limit int) (int, error) {
	if limit < 1 || limit > 1000 {
		return 0, errors.New("inventory reconciliation limit must be between 1 and 1000")
	}
	records, err := store.listPendingInventoryRecords(ctx, limit)
	if err != nil {
		return 0, err
	}
	if len(records) == 0 {
		return 0, nil
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return 0, err
	}
	defer func() { _ = tx.Rollback() }()
	cursorUpdates := make(map[inventoryEpochKey]pendingInventoryRecord)
	for _, record := range records {
		cursorUpdates[inventoryEpochKey{
			edgeNodeID:  record.edgeNodeID,
			ledgerEpoch: record.ledgerEpoch,
		}] = record
		seriesKey, systemID, measurement := inventoryMeasurementIdentity(record.record)
		if !measurement {
			continue
		}
		if err := ensureDeviceSourceTx(ctx, tx, record.edgeNodeID, systemID); err != nil {
			return 0, err
		}
		if err := ensureSignalSourceTx(
			ctx,
			tx,
			record.edgeNodeID,
			seriesKey,
			&systemID,
		); err != nil {
			return 0, err
		}
		if _, err := tx.ExecContext(ctx, `
			UPDATE site_signals
			SET last_received_at = CASE
				WHEN last_received_at IS NULL OR last_received_at < ? THEN ?
				ELSE last_received_at
			END
			WHERE edge_node_id = ? AND series_key = ?
		`, record.receivedAt, record.receivedAt, record.edgeNodeID, seriesKey); err != nil {
			return 0, err
		}
		if _, err := tx.ExecContext(ctx, `
			UPDATE site_devices
			SET last_received_at = CASE
				WHEN last_received_at IS NULL OR last_received_at < ? THEN ?
				ELSE last_received_at
			END
			WHERE edge_node_id = ? AND system_id = ?
		`, record.receivedAt, record.receivedAt, record.edgeNodeID, systemID); err != nil {
			return 0, err
		}
		current, valid := decodeInventoryMeasurement(record.record, record.receivedAt)
		if !valid {
			continue
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO signal_current_values (
				edge_node_id, series_key, values_json, event_time,
				site_received_at, updated_at
			) VALUES (?, ?, ?, ?, ?, ?)
			ON CONFLICT(edge_node_id, series_key) DO UPDATE SET
				values_json = excluded.values_json,
				event_time = excluded.event_time,
				site_received_at = excluded.site_received_at,
				updated_at = excluded.updated_at
			WHERE excluded.site_received_at >= signal_current_values.site_received_at
		`, record.edgeNodeID, seriesKey, []byte(current.Values), current.EventTime,
			current.SiteReceivedAt, time.Now().UnixMilli()); err != nil {
			return 0, err
		}
	}
	for _, record := range cursorUpdates {
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO inventory_projection_cursors (
				edge_node_id, ledger_epoch, last_pub_seq, updated_at
			) VALUES (?, ?, ?, ?)
			ON CONFLICT(edge_node_id, ledger_epoch) DO UPDATE SET
				last_pub_seq = excluded.last_pub_seq,
				updated_at = excluded.updated_at
		`, record.edgeNodeID, record.ledgerEpoch, record.pubSeq, time.Now().UnixMilli()); err != nil {
			return 0, err
		}
	}
	if err := tx.Commit(); err != nil {
		return 0, err
	}
	return len(records), nil
}

func (store *Store) listPendingInventoryRecords(
	ctx context.Context,
	limit int,
) ([]pendingInventoryRecord, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT accepted.edge_node_id, accepted.ledger_epoch,
			COALESCE(projected.last_pub_seq, 0), accepted.accepted_through
		FROM accepted_cursors AS accepted
		LEFT JOIN inventory_projection_cursors AS projected
			ON projected.edge_node_id = accepted.edge_node_id
			AND projected.ledger_epoch = accepted.ledger_epoch
		WHERE accepted.accepted_through > COALESCE(projected.last_pub_seq, 0)
		ORDER BY accepted.updated_at, accepted.edge_node_id, accepted.ledger_epoch
		LIMIT ?
	`, limit)
	if err != nil {
		return nil, err
	}
	epochs := make([]pendingInventoryEpoch, 0)
	for rows.Next() {
		var epoch pendingInventoryEpoch
		if err := rows.Scan(
			&epoch.edgeNodeID,
			&epoch.ledgerEpoch,
			&epoch.lastPubSeq,
			&epoch.acceptedThrough,
		); err != nil {
			_ = rows.Close()
			return nil, err
		}
		epochs = append(epochs, epoch)
	}
	if err := rows.Close(); err != nil {
		return nil, err
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}

	records := make([]pendingInventoryRecord, 0, limit)
	for _, epoch := range epochs {
		remaining := limit - len(records)
		if remaining == 0 {
			break
		}
		rows, err := store.db.QueryContext(ctx, `
			SELECT pub_seq, record_json, received_at
			FROM raw_records
			WHERE edge_node_id = ? AND ledger_epoch = ?
				AND pub_seq > ? AND pub_seq <= ?
			ORDER BY pub_seq
			LIMIT ?
		`, epoch.edgeNodeID, epoch.ledgerEpoch, epoch.lastPubSeq,
			epoch.acceptedThrough, remaining)
		if err != nil {
			return nil, err
		}
		before := len(records)
		for rows.Next() {
			record := pendingInventoryRecord{
				edgeNodeID:  epoch.edgeNodeID,
				ledgerEpoch: epoch.ledgerEpoch,
			}
			if err := rows.Scan(&record.pubSeq, &record.record, &record.receivedAt); err != nil {
				_ = rows.Close()
				return nil, err
			}
			records = append(records, record)
		}
		if err := rows.Close(); err != nil {
			return nil, err
		}
		if err := rows.Err(); err != nil {
			return nil, err
		}
		if len(records) == before {
			return nil, fmt.Errorf(
				"inventory projection raw gap for edge %q epoch %q after %d",
				epoch.edgeNodeID,
				epoch.ledgerEpoch,
				epoch.lastPubSeq,
			)
		}
	}
	return records, nil
}

func inventoryMeasurementIdentity(record []byte) (string, string, bool) {
	var header struct {
		Family    string `json:"family"`
		SeriesKey string `json:"series_key"`
	}
	if err := json.Unmarshal(record, &header); err != nil || header.Family != "measurement" {
		return "", "", false
	}
	identity, err := contract.ParseSeriesKey(header.SeriesKey)
	if err != nil {
		return "", "", false
	}
	return header.SeriesKey, identity.SystemID, true
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
		SELECT source.device_ref, source.edge_node_id,
			COALESCE(profile.display_name, ''),
			COALESCE(profile.location, ''),
			profile.revision,
			COALESCE(descriptor.presence, 'unknown'),
			COALESCE(descriptor.state, 'unknown'),
			source.last_received_at
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
	result := make([]siteapp.DeviceSummary, 0)
	for rows.Next() {
		var summary siteapp.DeviceSummary
		var revision sql.NullInt64
		var lastReceivedAt sql.NullInt64
		if err := rows.Scan(
			&summary.DeviceRef,
			&summary.Edge,
			&summary.DisplayName,
			&summary.Location,
			&revision,
			&summary.DescriptorPresence,
			&summary.DeviceState,
			&lastReceivedAt,
		); err != nil {
			return nil, err
		}
		if revision.Valid {
			summary.ProfileRevision = &revision.Int64
		}
		if lastReceivedAt.Valid {
			summary.LastReceivedAt = &lastReceivedAt.Int64
		}
		result = append(result, summary)
	}
	return result, rows.Err()
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
		SELECT source.signal_ref, source.edge_node_id,
			device.device_ref,
			COALESCE(profile.display_name, ''),
			profile.revision,
			COALESCE(descriptor.presence, 'unknown'),
			descriptor.unit,
			descriptor.value_type,
			descriptor.measurement_key,
			EXISTS (
				SELECT 1 FROM semantic_mappings AS mapping
				WHERE mapping.edge_node_id = source.edge_node_id
					AND mapping.series_key = source.series_key
					AND mapping.active = 1
			) OR EXISTS (
				SELECT 1 FROM semantic_definitions_v2 AS definition
				WHERE definition.signal_ref = source.signal_ref
					AND definition.active = 1
			),
			source.last_received_at,
			current.values_json,
			current.event_time,
			current.site_received_at
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
		LEFT JOIN signal_current_values AS current
			ON current.edge_node_id = source.edge_node_id
			AND current.series_key = source.series_key
		WHERE source.signal_ref > ?
		ORDER BY source.signal_ref
		LIMIT ?
	`, afterRef, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := make([]siteapp.SignalSummary, 0)
	for rows.Next() {
		var summary siteapp.SignalSummary
		var deviceRef sql.NullString
		var revision sql.NullInt64
		var unit sql.NullString
		var valueType sql.NullString
		var sensorType sql.NullString
		var lastReceivedAt sql.NullInt64
		var values []byte
		var eventTime sql.NullInt64
		var siteReceivedAt sql.NullInt64
		if err := rows.Scan(
			&summary.SignalRef,
			&summary.Edge,
			&deviceRef,
			&summary.DisplayName,
			&revision,
			&summary.DescriptorPresence,
			&unit,
			&valueType,
			&sensorType,
			&summary.HasSemanticMapping,
			&lastReceivedAt,
			&values,
			&eventTime,
			&siteReceivedAt,
		); err != nil {
			return nil, err
		}
		if deviceRef.Valid {
			summary.DeviceRef = &deviceRef.String
		}
		if revision.Valid {
			summary.ProfileRevision = &revision.Int64
		}
		if unit.Valid {
			summary.Unit = &unit.String
		}
		if valueType.Valid {
			summary.ValueType = &valueType.String
		}
		if sensorType.Valid {
			summary.SensorType = &sensorType.String
		}
		if lastReceivedAt.Valid {
			summary.LastReceivedAt = &lastReceivedAt.Int64
			if time.Now().UnixMilli()-lastReceivedAt.Int64 >
				int64(5*time.Minute/time.Millisecond) {
				summary.ReceiptStatus = "stale"
			} else {
				summary.ReceiptStatus = "receiving"
			}
		} else {
			summary.ReceiptStatus = "never_received"
		}
		if eventTime.Valid && siteReceivedAt.Valid && values != nil {
			summary.Latest = &siteapp.LatestMeasurement{
				Values:         append(json.RawMessage(nil), values...),
				EventTime:      eventTime.Int64,
				SiteReceivedAt: siteReceivedAt.Int64,
			}
		}
		result = append(result, summary)
	}
	return result, rows.Err()
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
