package store

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
)

var ErrDescriptorConflict = errors.New("descriptor revision content conflict")

type DescriptorApplyStatus string

const (
	DescriptorApplied      DescriptorApplyStatus = "applied"
	DescriptorIdempotent   DescriptorApplyStatus = "idempotent"
	DescriptorStaleIgnored DescriptorApplyStatus = "stale_ignored"
)

type DescriptorPresence string

const (
	DescriptorCurrent DescriptorPresence = "current"
	DescriptorStale   DescriptorPresence = "stale"
)

type DescriptorApplyResult struct {
	Status DescriptorApplyStatus
}

type DescriptorDeviceRow struct {
	EdgeNodeID         string
	SystemID           string
	Identifier         *string
	ModelID            *string
	State              string
	Presence           DescriptorPresence
	DescriptorRevision int64
	UpdatedAt          int64
}

type DescriptorSignalRow struct {
	EdgeNodeID         string
	SeriesKey          string
	SystemID           string
	MeasurementKey     string
	ChannelIndex       *int32
	Variant            string
	Unit               *string
	ValueType          string
	Presence           DescriptorPresence
	DescriptorRevision int64
	UpdatedAt          int64
}

func (store *Store) ApplyDescriptorSnapshot(
	ctx context.Context,
	snapshot contract.DescriptorSnapshot,
) (DescriptorApplyResult, error) {
	if err := snapshot.Validate(); err != nil {
		return DescriptorApplyResult{}, err
	}
	hash, err := snapshot.ContentSHA256()
	if err != nil {
		return DescriptorApplyResult{}, err
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return DescriptorApplyResult{}, err
	}
	defer func() { _ = tx.Rollback() }()

	var currentEpoch string
	var currentRevision int64
	var currentHash []byte
	err = tx.QueryRowContext(ctx, `
		SELECT ledger_epoch, descriptor_revision, content_sha256
		FROM edge_descriptor_state
		WHERE edge_node_id = ?
	`, snapshot.EdgeNodeID).Scan(&currentEpoch, &currentRevision, &currentHash)
	switch {
	case errors.Is(err, sql.ErrNoRows):
	case err != nil:
		return DescriptorApplyResult{}, err
	case currentEpoch == snapshot.LedgerEpoch && int64(snapshot.DescriptorRevision) < currentRevision:
		return DescriptorApplyResult{Status: DescriptorStaleIgnored}, nil
	case currentEpoch == snapshot.LedgerEpoch && int64(snapshot.DescriptorRevision) == currentRevision:
		if bytes.Equal(currentHash, hash[:]) {
			return DescriptorApplyResult{Status: DescriptorIdempotent}, nil
		}
		summary, marshalErr := json.Marshal(struct {
			LedgerEpoch        string `json:"ledger_epoch"`
			DescriptorRevision uint64 `json:"descriptor_revision"`
		}{
			LedgerEpoch:        snapshot.LedgerEpoch,
			DescriptorRevision: snapshot.DescriptorRevision,
		})
		if marshalErr != nil {
			return DescriptorApplyResult{}, marshalErr
		}
		if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
			OccurredAt:  time.Now().UnixMilli(),
			ActorClass:  edgeapp.ActorSystem,
			ActorRef:    "descriptor_consumer",
			Operation:   "descriptor_snapshot.conflict",
			ResourceRef: snapshot.EdgeNodeID,
			Outcome:     "failure",
			Summary:     summary,
		}); err != nil {
			return DescriptorApplyResult{}, err
		}
		if err := tx.Commit(); err != nil {
			return DescriptorApplyResult{}, err
		}
		return DescriptorApplyResult{}, ErrDescriptorConflict
	}

	now := time.Now().UnixMilli()
	if err := discoverEdgeNodeTx(
		ctx,
		tx,
		snapshot.EdgeNodeID,
		snapshot.LedgerEpoch,
		now,
	); err != nil {
		return DescriptorApplyResult{}, err
	}
	for _, device := range snapshot.Devices {
		if err := ensureDeviceSourceTx(ctx, tx, snapshot.EdgeNodeID, device.SystemID); err != nil {
			return DescriptorApplyResult{}, err
		}
	}
	for _, signal := range snapshot.Signals {
		if err := ensureSignalSourceTx(
			ctx,
			tx,
			snapshot.EdgeNodeID,
			signal.SeriesKey,
			&signal.SystemID,
		); err != nil {
			return DescriptorApplyResult{}, err
		}
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE descriptor_devices SET presence = 'stale', updated_at = ?
		WHERE edge_node_id = ? AND presence != 'stale'
	`, now, snapshot.EdgeNodeID); err != nil {
		return DescriptorApplyResult{}, err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE descriptor_signals SET presence = 'stale', updated_at = ?
		WHERE edge_node_id = ? AND presence != 'stale'
	`, now, snapshot.EdgeNodeID); err != nil {
		return DescriptorApplyResult{}, err
	}

	for _, device := range snapshot.Devices {
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO descriptor_devices (
				edge_node_id, system_id, identifier, model_id, state, presence,
				descriptor_revision, updated_at
			) VALUES (?, ?, ?, ?, ?, 'current', ?, ?)
			ON CONFLICT(edge_node_id, system_id) DO UPDATE SET
				identifier = excluded.identifier,
				model_id = excluded.model_id,
				state = excluded.state,
				presence = 'current',
				descriptor_revision = excluded.descriptor_revision,
				updated_at = excluded.updated_at
		`, snapshot.EdgeNodeID, device.SystemID, device.Identifier, device.ModelID, device.State,
			int64(snapshot.DescriptorRevision), now); err != nil {
			return DescriptorApplyResult{}, err
		}
	}
	for _, signal := range snapshot.Signals {
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO descriptor_signals (
				edge_node_id, series_key, system_id, measurement_key, channel_index,
				variant, unit, value_type, presence, descriptor_revision, updated_at
			) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'current', ?, ?)
			ON CONFLICT(edge_node_id, series_key) DO UPDATE SET
				system_id = excluded.system_id,
				measurement_key = excluded.measurement_key,
				channel_index = excluded.channel_index,
				variant = excluded.variant,
				unit = excluded.unit,
				value_type = excluded.value_type,
				presence = 'current',
				descriptor_revision = excluded.descriptor_revision,
				updated_at = excluded.updated_at
		`, snapshot.EdgeNodeID, signal.SeriesKey, signal.SystemID, signal.MeasurementKey,
			signal.ChannelIndex, signal.Variant, signal.Unit, signal.ValueType,
			int64(snapshot.DescriptorRevision), now); err != nil {
			return DescriptorApplyResult{}, err
		}
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO edge_descriptor_state (
			edge_node_id, ledger_epoch, descriptor_revision, content_sha256, updated_at
		) VALUES (?, ?, ?, ?, ?)
		ON CONFLICT(edge_node_id) DO UPDATE SET
			ledger_epoch = excluded.ledger_epoch,
			descriptor_revision = excluded.descriptor_revision,
			content_sha256 = excluded.content_sha256,
			updated_at = excluded.updated_at
	`, snapshot.EdgeNodeID, snapshot.LedgerEpoch, int64(snapshot.DescriptorRevision), hash[:], now); err != nil {
		return DescriptorApplyResult{}, err
	}
	if err := tx.Commit(); err != nil {
		return DescriptorApplyResult{}, err
	}
	return DescriptorApplyResult{Status: DescriptorApplied}, nil
}

func (store *Store) ListDescriptorDevices(ctx context.Context, edgeNodeID string) ([]DescriptorDeviceRow, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT edge_node_id, system_id, identifier, model_id, state, presence,
			descriptor_revision, updated_at
		FROM descriptor_devices
		WHERE edge_node_id = ?
		ORDER BY system_id
	`, edgeNodeID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := make([]DescriptorDeviceRow, 0)
	for rows.Next() {
		var row DescriptorDeviceRow
		var identifier sql.NullString
		var modelID sql.NullString
		if err := rows.Scan(&row.EdgeNodeID, &row.SystemID, &identifier, &modelID, &row.State,
			&row.Presence, &row.DescriptorRevision, &row.UpdatedAt); err != nil {
			return nil, err
		}
		if identifier.Valid {
			row.Identifier = &identifier.String
		}
		if modelID.Valid {
			row.ModelID = &modelID.String
		}
		result = append(result, row)
	}
	return result, rows.Err()
}

func (store *Store) ListDescriptorSignals(ctx context.Context, edgeNodeID string) ([]DescriptorSignalRow, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT edge_node_id, series_key, system_id, measurement_key, channel_index,
			variant, unit, value_type, presence, descriptor_revision, updated_at
		FROM descriptor_signals
		WHERE edge_node_id = ?
		ORDER BY series_key
	`, edgeNodeID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := make([]DescriptorSignalRow, 0)
	for rows.Next() {
		var row DescriptorSignalRow
		var channel sql.NullInt64
		var unit sql.NullString
		if err := rows.Scan(&row.EdgeNodeID, &row.SeriesKey, &row.SystemID,
			&row.MeasurementKey, &channel, &row.Variant, &unit, &row.ValueType,
			&row.Presence, &row.DescriptorRevision, &row.UpdatedAt); err != nil {
			return nil, err
		}
		if channel.Valid {
			value := int32(channel.Int64)
			row.ChannelIndex = &value
		}
		if unit.Valid {
			row.Unit = &unit.String
		}
		result = append(result, row)
	}
	return result, rows.Err()
}
