package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantic"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

const (
	auditOutcomeSuccess = "success"
	auditOutcomeFailure = "failure"
)

func (store *Store) RecordAuditEvent(ctx context.Context, event siteapp.AuditEvent) error {
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer func() { _ = tx.Rollback() }()
	if err := insertAuditEventTx(ctx, tx, event); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) ApplySemanticMapping(
	ctx context.Context,
	actor siteapp.Actor,
	spec semantic.MappingSpec,
	precondition siteapp.RevisionPrecondition,
) (semantic.Mapping, error) {
	var noMapping semantic.Mapping
	if err := actor.Validate(); err != nil {
		return noMapping, err
	}
	if err := spec.Validate(); err != nil {
		return noMapping, err
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noMapping, err
	}
	defer func() { _ = tx.Rollback() }()

	mapping, err := putSemanticMappingTx(ctx, tx, spec, precondition)
	if err != nil {
		return noMapping, err
	}
	summary, err := json.Marshal(struct {
		Meaning     semantic.Meaning     `json:"meaning"`
		TriggerMode semantic.TriggerMode `json:"trigger_mode"`
		ActiveValue int                  `json:"active_value"`
		Revision    int64                `json:"revision"`
	}{
		Meaning:     mapping.Meaning,
		TriggerMode: mapping.TriggerMode,
		ActiveValue: mapping.ActiveValue,
		Revision:    mapping.Revision,
	})
	if err != nil {
		return noMapping, err
	}
	if err := insertAuditEventTx(ctx, tx, siteapp.AuditEvent{
		OccurredAt:  time.Now().UnixMilli(),
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "semantic_mapping.put",
		ResourceRef: mapping.ID,
		Outcome:     auditOutcomeSuccess,
		Summary:     summary,
	}); err != nil {
		return noMapping, err
	}
	if err := tx.Commit(); err != nil {
		return noMapping, err
	}
	return mapping, nil
}

func (store *Store) DeactivateSemanticMapping(
	ctx context.Context,
	actor siteapp.Actor,
	edgeNodeID string,
	seriesKey string,
	precondition siteapp.RevisionPrecondition,
) (semantic.Mapping, error) {
	var noMapping semantic.Mapping
	if err := actor.Validate(); err != nil {
		return noMapping, err
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noMapping, err
	}
	defer func() { _ = tx.Rollback() }()

	var mapping semantic.Mapping
	err = tx.QueryRowContext(ctx, `
		SELECT mapping_id, revision, edge_node_id, series_key, meaning,
			trigger_mode, active_value, active, created_at
		FROM semantic_mappings
		WHERE edge_node_id = ? AND series_key = ? AND active = 1
	`, edgeNodeID, seriesKey).Scan(
		&mapping.ID,
		&mapping.Revision,
		&mapping.EdgeNodeID,
		&mapping.SeriesKey,
		&mapping.Meaning,
		&mapping.TriggerMode,
		&mapping.ActiveValue,
		&mapping.Active,
		&mapping.CreatedAt,
	)
	if errors.Is(err, sql.ErrNoRows) {
		return noMapping, siteapp.ErrNotFound
	}
	if err != nil {
		return noMapping, err
	}
	if err := checkRevisionPrecondition(precondition, true, mapping.Revision); err != nil {
		return noMapping, err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_mapping_ends (
			mapping_id, mapping_revision, ledger_epoch, end_at_pub_seq
		)
		SELECT ?, ?, ledger_epoch, accepted_through
		FROM accepted_cursors
		WHERE edge_node_id = ?
	`, mapping.ID, mapping.Revision, mapping.EdgeNodeID); err != nil {
		return noMapping, err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE semantic_mappings
		SET active = 0
		WHERE mapping_id = ? AND revision = ? AND active = 1
	`, mapping.ID, mapping.Revision); err != nil {
		return noMapping, err
	}
	mapping.Active = false
	summary, err := json.Marshal(struct {
		Revision int64 `json:"revision"`
	}{Revision: mapping.Revision})
	if err != nil {
		return noMapping, err
	}
	if err := insertAuditEventTx(ctx, tx, siteapp.AuditEvent{
		OccurredAt:  time.Now().UnixMilli(),
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "semantic_mapping.deactivate",
		ResourceRef: mapping.ID,
		Outcome:     auditOutcomeSuccess,
		Summary:     summary,
	}); err != nil {
		return noMapping, err
	}
	if err := tx.Commit(); err != nil {
		return noMapping, err
	}
	return mapping, nil
}

func insertAuditEventTx(ctx context.Context, tx *sql.Tx, event siteapp.AuditEvent) error {
	var actorLoginID any
	var actorDisplayName any
	if event.ActorClass == siteapp.ActorAccount {
		var loginID string
		var displayName string
		if err := tx.QueryRowContext(ctx, `
			SELECT login_id, display_name
			FROM site_accounts
			WHERE account_ref = ?
		`, event.ActorRef).Scan(&loginID, &displayName); err != nil {
			return err
		}
		actorLoginID = loginID
		actorDisplayName = displayName
	}
	_, err := tx.ExecContext(ctx, `
		INSERT INTO audit_events (
			occurred_at, actor_class, actor_ref, actor_login_id, actor_display_name,
			operation, resource_ref, outcome, summary_json
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, event.OccurredAt, event.ActorClass, event.ActorRef,
		actorLoginID, actorDisplayName, event.Operation,
		event.ResourceRef, event.Outcome, []byte(event.Summary))
	return err
}

func (store *Store) ListAuditEvents(ctx context.Context, limit int) ([]siteapp.AuditEvent, error) {
	if limit < 1 || limit > 100 {
		return nil, errors.New("audit event limit must be between 1 and 100")
	}
	rows, err := store.db.QueryContext(ctx, `
		SELECT audit_row_id, occurred_at, actor_class, actor_ref,
			actor_login_id, actor_display_name, operation,
			resource_ref, outcome, summary_json
		FROM audit_events
		ORDER BY audit_row_id DESC
		LIMIT ?
	`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	events := make([]siteapp.AuditEvent, 0)
	for rows.Next() {
		var event siteapp.AuditEvent
		var actorLoginID sql.NullString
		var actorDisplayName sql.NullString
		if err := rows.Scan(
			&event.AuditRowID,
			&event.OccurredAt,
			&event.ActorClass,
			&event.ActorRef,
			&actorLoginID,
			&actorDisplayName,
			&event.Operation,
			&event.ResourceRef,
			&event.Outcome,
			&event.Summary,
		); err != nil {
			return nil, err
		}
		event.ActorLoginID = nullableString(actorLoginID)
		event.ActorDisplayName = nullableString(actorDisplayName)
		events = append(events, event)
	}
	return events, rows.Err()
}

func nullableString(value sql.NullString) *string {
	if !value.Valid {
		return nil
	}
	return &value.String
}
