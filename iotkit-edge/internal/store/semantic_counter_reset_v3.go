package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"strings"
	"time"
	"unicode"

	"github.com/google/uuid"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
)

func (store *Store) RequestSemanticCounterReset(
	ctx context.Context,
	actor edgeapp.Actor,
	ruleID string,
	resetID string,
) (semantics.CounterReset, error) {
	var noReset semantics.CounterReset
	if err := actor.Validate(); err != nil {
		return noReset, err
	}
	if len(resetID) < 1 || len(resetID) > 128 ||
		strings.IndexFunc(resetID, unicode.IsControl) >= 0 {
		return noReset, errors.New("invalid semantic counter reset id")
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noReset, err
	}
	defer func() { _ = tx.Rollback() }()
	var existing semantics.CounterReset
	var appliedAt sql.NullInt64
	err = tx.QueryRowContext(ctx, `
		SELECT reset_id, rule_id, ledger_epoch, apply_after_pub_seq,
			requested_at, applied_at
		FROM semantic_counter_resets_v3 WHERE reset_id = ?
	`, resetID).Scan(
		&existing.ID, &existing.RuleID, &existing.LedgerEpoch,
		&existing.ApplyAfterPubSeq, &existing.RequestedAt, &appliedAt,
	)
	if err == nil {
		if existing.RuleID != ruleID {
			return noReset, errors.New("semantic counter reset id is already used")
		}
		if appliedAt.Valid {
			existing.AppliedAt = &appliedAt.Int64
		}
		if err := tx.Commit(); err != nil {
			return noReset, err
		}
		return existing, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return noReset, err
	}
	var edgeNodeID string
	var kind semantics.Kind
	if err := tx.QueryRowContext(ctx, `
		SELECT signal.edge_node_id, rule.kind
		FROM semantic_rules_v3 AS rule
		JOIN edge_signals AS signal ON signal.signal_ref = rule.signal_ref
		WHERE rule.rule_id = ? AND rule.retired_at IS NULL
	`, ruleID).Scan(&edgeNodeID, &kind); errors.Is(err, sql.ErrNoRows) {
		return noReset, edgeapp.ErrNotFound
	} else if err != nil {
		return noReset, err
	}
	if kind != semantics.KindCumulativeCounter {
		return noReset, errors.New("semantic rule is not a cumulative counter")
	}
	reset := semantics.CounterReset{ID: resetID, RuleID: ruleID}
	if err := tx.QueryRowContext(ctx, `
		SELECT ledger_epoch, accepted_through
		FROM accepted_cursors WHERE edge_node_id = ?
		ORDER BY updated_at DESC, ledger_epoch DESC LIMIT 1
	`, edgeNodeID).Scan(
		&reset.LedgerEpoch, &reset.ApplyAfterPubSeq,
	); errors.Is(err, sql.ErrNoRows) {
		return noReset, errors.New("semantic counter has no accepted cursor")
	} else if err != nil {
		return noReset, err
	}
	reset.RequestedAt = time.Now().UnixMilli()
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_counter_resets_v3(
			reset_id, rule_id, ledger_epoch, apply_after_pub_seq,
			requested_at, actor_ref
		) VALUES (?, ?, ?, ?, ?, ?)
	`, reset.ID, reset.RuleID, reset.LedgerEpoch, reset.ApplyAfterPubSeq,
		reset.RequestedAt, actor.Ref); err != nil {
		return noReset, err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_counter_reset_boundaries_v3(
			reset_id, ledger_epoch, apply_after_pub_seq
		)
		SELECT ?, ledger_epoch, accepted_through
		FROM accepted_cursors WHERE edge_node_id = ?
	`, reset.ID, edgeNodeID); err != nil {
		return noReset, err
	}
	summary, _ := json.Marshal(struct {
		LedgerEpoch      string `json:"ledger_epoch"`
		ApplyAfterPubSeq int64  `json:"apply_after_pub_seq"`
	}{
		LedgerEpoch:      reset.LedgerEpoch,
		ApplyAfterPubSeq: reset.ApplyAfterPubSeq,
	})
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt: reset.RequestedAt, ActorClass: actor.Class,
		ActorRef: actor.Ref, Operation: "semantic_counter_reset.request",
		ResourceRef: reset.ID, Outcome: auditOutcomeSuccess, Summary: summary,
	}); err != nil {
		return noReset, err
	}
	if err := tx.Commit(); err != nil {
		return noReset, err
	}
	return reset, nil
}

func (store *Store) applyReadySemanticCounterResetsV3(
	ctx context.Context,
) error {
	rows, err := store.db.QueryContext(ctx, `
		SELECT reset_id, rule_id, ledger_epoch, apply_after_pub_seq,
			requested_at, actor_ref
		FROM semantic_counter_resets_v3
		WHERE applied_at IS NULL
		ORDER BY requested_at, reset_id
	`)
	if err != nil {
		return err
	}
	type pendingReset struct {
		reset    semantics.CounterReset
		actorRef string
	}
	pending := make([]pendingReset, 0)
	for rows.Next() {
		var item pendingReset
		if err := rows.Scan(
			&item.reset.ID, &item.reset.RuleID, &item.reset.LedgerEpoch,
			&item.reset.ApplyAfterPubSeq, &item.reset.RequestedAt,
			&item.actorRef,
		); err != nil {
			_ = rows.Close()
			return err
		}
		pending = append(pending, item)
	}
	if err := rows.Close(); err != nil {
		return err
	}
	for _, item := range pending {
		ready, err := store.semanticCounterResetReadyV3(ctx, item.reset)
		if err != nil {
			return err
		}
		if !ready {
			continue
		}
		if err := store.applySemanticCounterResetV3(
			ctx, item.reset, item.actorRef,
		); err != nil {
			return err
		}
	}
	return nil
}

func (store *Store) semanticCounterResetReadyV3(
	ctx context.Context,
	reset semantics.CounterReset,
) (bool, error) {
	var missing bool
	err := store.db.QueryRowContext(ctx, `
		SELECT EXISTS(
			SELECT 1
			FROM semantic_counter_reset_boundaries_v3 AS boundary
			JOIN semantic_rules_v3 AS rule
				ON rule.rule_id = ?
			JOIN edge_signals AS signal ON signal.signal_ref = rule.signal_ref
			JOIN semantic_rule_revisions_v3 AS revision
				ON revision.rule_id = rule.rule_id
			JOIN semantic_rule_starts_v3 AS starts
				ON starts.rule_id = rule.rule_id
				AND starts.rule_revision = revision.revision
				AND starts.ledger_epoch = boundary.ledger_epoch
			LEFT JOIN semantic_rule_ends_v3 AS ends
				ON ends.rule_id = rule.rule_id
				AND ends.rule_revision = revision.revision
				AND ends.ledger_epoch = boundary.ledger_epoch
			JOIN raw_records AS raw
				ON raw.edge_node_id = signal.edge_node_id
				AND raw.ledger_epoch = boundary.ledger_epoch
				AND json_extract(raw.record_json, '$.series_key') = signal.series_key
			WHERE boundary.reset_id = ?
				AND raw.pub_seq <= boundary.apply_after_pub_seq
				AND raw.pub_seq > starts.start_after_pub_seq
				AND (ends.end_at_pub_seq IS NULL OR raw.pub_seq <= ends.end_at_pub_seq)
				AND NOT EXISTS(
					SELECT 1 FROM semantic_projection_receipts_v3 AS receipt
					WHERE receipt.rule_id = rule.rule_id
						AND receipt.ledger_epoch = raw.ledger_epoch
						AND receipt.pub_seq = raw.pub_seq
				)
		)
	`, reset.RuleID, reset.ID).Scan(&missing)
	return !missing, err
}

func (store *Store) applySemanticCounterResetV3(
	ctx context.Context,
	reset semantics.CounterReset,
	actorRef string,
) error {
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer func() { _ = tx.Rollback() }()
	var alreadyApplied bool
	if err := tx.QueryRowContext(ctx, `
		SELECT EXISTS(
			SELECT 1 FROM semantic_counter_resets_v3
			WHERE reset_id = ? AND applied_at IS NOT NULL
		)
	`, reset.ID).Scan(&alreadyApplied); err != nil {
		return err
	}
	if alreadyApplied {
		return tx.Commit()
	}
	var (
		seriesID            string
		signalRef           string
		edgeNodeID          string
		ruleRevision        int64
		calibrationRevision int64
		nextSequence        int64
		appliedSeriesID     string
	)
	if err := tx.QueryRowContext(ctx, `
		SELECT revision.series_id, rule.signal_ref, signal.edge_node_id,
			revision.revision, calibration.revision,
			runtime.next_sequence, runtime.applied_series_id
		FROM semantic_rules_v3 AS rule
		JOIN semantic_rule_revisions_v3 AS revision
			ON revision.rule_id = rule.rule_id AND revision.active = 1
		JOIN signal_calibration_revisions_v3 AS calibration
			ON calibration.signal_ref = rule.signal_ref
			AND calibration.active = 1
		JOIN edge_signals AS signal ON signal.signal_ref = rule.signal_ref
		JOIN semantic_rule_runtime_v3 AS runtime
			ON runtime.rule_id = rule.rule_id
		WHERE rule.rule_id = ?
	`, reset.RuleID).Scan(
		&seriesID, &signalRef, &edgeNodeID, &ruleRevision,
		&calibrationRevision, &nextSequence, &appliedSeriesID,
	); err != nil {
		return err
	}
	if appliedSeriesID != seriesID {
		nextSequence = 1
	}
	observationID := uuid.NewSHA1(
		uuid.NameSpaceOID,
		[]byte("semantic-v3-reset:"+reset.ID),
	).String()
	now := time.Now().UnixMilli()
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_observations_v3(
			observation_id, rule_id, rule_revision, calibration_revision,
			series_id, sequence, kind, value_json, signal_ref, edge_node_id,
			ledger_epoch, source_pub_seq, observed_at, created_at
		) VALUES (?, ?, ?, ?, ?, ?, 'cumulative_counter', 0, ?, ?, ?, ?, ?, ?)
	`, observationID, reset.RuleID, ruleRevision, calibrationRevision,
		seriesID, nextSequence, signalRef, edgeNodeID, reset.LedgerEpoch,
		reset.ApplyAfterPubSeq, reset.RequestedAt, now); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE semantic_rule_runtime_v3
		SET initialized = 0, detector_active = 0, counter = 0,
			pending = 0, pending_active = 0, pending_since = 0,
			applied_rule_revision = ?,
			applied_calibration_revision = ?,
			applied_ledger_epoch = ?,
			next_sequence = ?,
			applied_series_id = ?
		WHERE rule_id = ?
	`, ruleRevision, calibrationRevision, reset.LedgerEpoch,
		nextSequence+1, seriesID, reset.RuleID); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE semantic_counter_resets_v3
		SET applied_at = ?, zero_observation_id = ?
		WHERE reset_id = ? AND applied_at IS NULL
	`, now, observationID, reset.ID); err != nil {
		return err
	}
	summary, _ := json.Marshal(struct {
		ResetID string `json:"reset_id"`
		Counter int64  `json:"counter"`
	}{
		ResetID: reset.ID, Counter: 0,
	})
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt: now, ActorClass: edgeapp.ActorSystem,
		ActorRef: "semantic_projector", Operation: "semantic_counter_reset.apply",
		ResourceRef: reset.ID, Outcome: auditOutcomeSuccess, Summary: summary,
	}); err != nil {
		return err
	}
	_ = actorRef
	return tx.Commit()
}
