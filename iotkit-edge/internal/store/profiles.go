package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"strings"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
)

func (store *Store) UpdateDeviceProfile(
	ctx context.Context,
	actor edgeapp.Actor,
	deviceRef string,
	input edgeapp.DeviceProfileInput,
	precondition edgeapp.RevisionPrecondition,
) (edgeapp.DeviceProfile, error) {
	var noProfile edgeapp.DeviceProfile
	if err := actor.Validate(); err != nil {
		return noProfile, err
	}
	if err := input.Validate(); err != nil {
		return noProfile, err
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noProfile, err
	}
	defer func() { _ = tx.Rollback() }()

	var edgeNodeID string
	var systemID string
	if err := tx.QueryRowContext(ctx, `
		SELECT edge_node_id, system_id FROM edge_devices WHERE device_ref = ?
	`, deviceRef).Scan(&edgeNodeID, &systemID); errors.Is(err, sql.ErrNoRows) {
		return noProfile, edgeapp.ErrNotFound
	} else if err != nil {
		return noProfile, err
	}

	currentRevision, exists, err := currentDeviceProfileRevision(ctx, tx, edgeNodeID, systemID)
	if err != nil {
		return noProfile, err
	}
	if err := checkRevisionPrecondition(precondition, exists, currentRevision); err != nil {
		return noProfile, err
	}

	profile := edgeapp.DeviceProfile{
		DeviceRef:   deviceRef,
		DisplayName: strings.TrimSpace(input.DisplayName),
		Location:    strings.TrimSpace(input.Location),
		Revision:    currentRevision + 1,
		UpdatedAt:   time.Now().UnixMilli(),
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO device_profiles (
			edge_node_id, system_id, display_name, location, revision, updated_at
		) VALUES (?, ?, ?, ?, ?, ?)
		ON CONFLICT(edge_node_id, system_id) DO UPDATE SET
			display_name = excluded.display_name,
			location = excluded.location,
			revision = excluded.revision,
			updated_at = excluded.updated_at
	`, edgeNodeID, systemID, profile.DisplayName, profile.Location,
		profile.Revision, profile.UpdatedAt); err != nil {
		return noProfile, err
	}
	summary, err := json.Marshal(struct {
		DisplayName string `json:"display_name"`
		Location    string `json:"location"`
		Revision    int64  `json:"revision"`
	}{profile.DisplayName, profile.Location, profile.Revision})
	if err != nil {
		return noProfile, err
	}
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt:  profile.UpdatedAt,
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "device_profile.update",
		ResourceRef: deviceRef,
		Outcome:     auditOutcomeSuccess,
		Summary:     summary,
	}); err != nil {
		return noProfile, err
	}
	if err := tx.Commit(); err != nil {
		return noProfile, err
	}
	return profile, nil
}

func currentDeviceProfileRevision(
	ctx context.Context,
	tx *sqlTx,
	edgeNodeID string,
	systemID string,
) (int64, bool, error) {
	var revision int64
	err := tx.QueryRowContext(ctx, `
		SELECT revision FROM device_profiles WHERE edge_node_id = ? AND system_id = ?
	`, edgeNodeID, systemID).Scan(&revision)
	if errors.Is(err, sql.ErrNoRows) {
		return 0, false, nil
	}
	return revision, err == nil, err
}

func (store *Store) UpdateSignalProfile(
	ctx context.Context,
	actor edgeapp.Actor,
	signalRef string,
	input edgeapp.SignalProfileInput,
	precondition edgeapp.RevisionPrecondition,
) (edgeapp.SignalProfile, error) {
	var noProfile edgeapp.SignalProfile
	if err := actor.Validate(); err != nil {
		return noProfile, err
	}
	if err := input.Validate(); err != nil {
		return noProfile, err
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noProfile, err
	}
	defer func() { _ = tx.Rollback() }()

	var edgeNodeID string
	var seriesKey string
	if err := tx.QueryRowContext(ctx, `
		SELECT edge_node_id, series_key FROM edge_signals WHERE signal_ref = ?
	`, signalRef).Scan(&edgeNodeID, &seriesKey); errors.Is(err, sql.ErrNoRows) {
		return noProfile, edgeapp.ErrNotFound
	} else if err != nil {
		return noProfile, err
	}

	currentRevision, exists, err := currentSignalProfileRevision(ctx, tx, edgeNodeID, seriesKey)
	if err != nil {
		return noProfile, err
	}
	if err := checkRevisionPrecondition(precondition, exists, currentRevision); err != nil {
		return noProfile, err
	}

	sensorTypeLabel := strings.TrimSpace(input.DisplaySensorTypeLabel)
	if input.DisplaySensorType != "custom" {
		sensorTypeLabel = ""
	}
	displayUnit := strings.TrimSpace(input.DisplayUnit)
	if input.DisplayUnitMode == "dimensionless" {
		displayUnit = ""
	}
	profile := edgeapp.SignalProfile{
		SignalRef:              signalRef,
		DisplayName:            strings.TrimSpace(input.DisplayName),
		DisplaySensorType:      input.DisplaySensorType,
		DisplaySensorTypeLabel: sensorTypeLabel,
		DisplayValueKind:       input.DisplayValueKind,
		DisplayUnitMode:        input.DisplayUnitMode,
		DisplayUnit:            displayUnit,
		DecimalPlaces:          input.DecimalPlaces,
		Revision:               currentRevision + 1,
		UpdatedAt:              time.Now().UnixMilli(),
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO signal_profiles (
			edge_node_id, series_key, display_name, display_sensor_type,
			display_sensor_type_label, display_value_kind, display_unit_mode,
			display_unit, decimal_places, revision, updated_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
		ON CONFLICT(edge_node_id, series_key) DO UPDATE SET
			display_name = excluded.display_name,
			display_sensor_type = excluded.display_sensor_type,
			display_sensor_type_label = excluded.display_sensor_type_label,
			display_value_kind = excluded.display_value_kind,
			display_unit_mode = excluded.display_unit_mode,
			display_unit = excluded.display_unit,
			decimal_places = excluded.decimal_places,
			revision = excluded.revision,
			updated_at = excluded.updated_at
	`, edgeNodeID, seriesKey, profile.DisplayName, profile.DisplaySensorType,
		profile.DisplaySensorTypeLabel, profile.DisplayValueKind,
		profile.DisplayUnitMode, profile.DisplayUnit, profile.DecimalPlaces,
		profile.Revision, profile.UpdatedAt); err != nil {
		return noProfile, err
	}
	summary, err := json.Marshal(struct {
		DisplayName            string `json:"display_name"`
		DisplaySensorType      string `json:"display_sensor_type"`
		DisplaySensorTypeLabel string `json:"display_sensor_type_label"`
		DisplayValueKind       string `json:"display_value_kind"`
		DisplayUnitMode        string `json:"display_unit_mode"`
		DisplayUnit            string `json:"display_unit"`
		DecimalPlaces          int    `json:"decimal_places"`
		Revision               int64  `json:"revision"`
	}{
		profile.DisplayName,
		profile.DisplaySensorType,
		profile.DisplaySensorTypeLabel,
		profile.DisplayValueKind,
		profile.DisplayUnitMode,
		profile.DisplayUnit,
		profile.DecimalPlaces,
		profile.Revision,
	})
	if err != nil {
		return noProfile, err
	}
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt:  profile.UpdatedAt,
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "signal_profile.update",
		ResourceRef: signalRef,
		Outcome:     auditOutcomeSuccess,
		Summary:     summary,
	}); err != nil {
		return noProfile, err
	}
	if err := tx.Commit(); err != nil {
		return noProfile, err
	}
	return profile, nil
}

func currentSignalProfileRevision(
	ctx context.Context,
	tx *sqlTx,
	edgeNodeID string,
	seriesKey string,
) (int64, bool, error) {
	var revision int64
	err := tx.QueryRowContext(ctx, `
		SELECT revision FROM signal_profiles WHERE edge_node_id = ? AND series_key = ?
	`, edgeNodeID, seriesKey).Scan(&revision)
	if errors.Is(err, sql.ErrNoRows) {
		return 0, false, nil
	}
	return revision, err == nil, err
}
