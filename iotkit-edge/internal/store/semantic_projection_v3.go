package store

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"github.com/google/uuid"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
)

type semanticV3Candidate struct {
	RuleID              string
	RuleRevision        int64
	SignalRef           string
	EdgeNodeID          string
	SeriesKey           string
	SeriesID            string
	Kind                semantics.Kind
	SpecJSON            []byte
	CalibrationRevision int64
	Scale               float64
	Offset              float64
	LedgerEpoch         string
	PubSeq              int64
	RecordJSON          []byte
	ReceivedAt          int64
}

func (store *Store) ProjectSemanticRules(
	ctx context.Context,
	limit int,
) (int, error) {
	if limit < 1 || limit > 10_000 {
		return 0, errors.New("semantic rule projection limit must be between 1 and 10000")
	}
	if err := store.ensureSemanticEpochStartsV3(ctx); err != nil {
		return 0, err
	}
	failedRules := make(map[string]struct{})
	processed := 0
	var projectionErrors []error
	for processed < limit {
		if err := store.applyReadySemanticCounterResetsV3(ctx); err != nil {
			projectionErrors = append(projectionErrors, err)
			break
		}
		candidates, err := store.listSemanticV3Candidates(ctx, limit-processed)
		if err != nil {
			projectionErrors = append(projectionErrors, err)
			break
		}
		if len(candidates) == 0 {
			break
		}
		progress := false
		for _, candidate := range candidates {
			if _, failed := failedRules[candidate.RuleID]; failed {
				continue
			}
			if err := store.projectSemanticV3Candidate(ctx, candidate); err != nil {
				failedRules[candidate.RuleID] = struct{}{}
				store.recordSemanticV3Failure(ctx, candidate, err)
				projectionErrors = append(projectionErrors, fmt.Errorf(
					"project semantic rule %s at %s/%d: %w",
					candidate.RuleID, candidate.LedgerEpoch, candidate.PubSeq, err,
				))
				continue
			}
			store.clearSemanticV3Failure(ctx, candidate)
			processed++
			progress = true
			if processed >= limit {
				break
			}
		}
		if !progress {
			break
		}
	}
	if err := store.applyReadySemanticCounterResetsV3(ctx); err != nil {
		projectionErrors = append(projectionErrors, err)
	}
	return processed, errors.Join(projectionErrors...)
}

func (store *Store) ensureSemanticEpochStartsV3(ctx context.Context) error {
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer func() { _ = tx.Rollback() }()
	if _, err := tx.ExecContext(ctx, `
		INSERT OR IGNORE INTO signal_calibration_starts_v3(
			signal_ref, calibration_revision, ledger_epoch, start_after_pub_seq
		)
		SELECT signal.signal_ref, calibration.revision,
			cursor.ledger_epoch, 0
		FROM edge_signals AS signal
		JOIN accepted_cursors AS cursor
			ON cursor.edge_node_id = signal.edge_node_id
		JOIN signal_calibration_revisions_v3 AS calibration
			ON calibration.signal_ref = signal.signal_ref
			AND calibration.active = 1
	`); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT OR IGNORE INTO semantic_rule_starts_v3(
			rule_id, rule_revision, ledger_epoch, start_after_pub_seq
		)
		SELECT rule.rule_id, revision.revision, cursor.ledger_epoch, 0
		FROM semantic_rules_v3 AS rule
		JOIN edge_signals AS signal ON signal.signal_ref = rule.signal_ref
		JOIN accepted_cursors AS cursor
			ON cursor.edge_node_id = signal.edge_node_id
		JOIN semantic_rule_revisions_v3 AS revision
			ON revision.rule_id = rule.rule_id AND revision.active = 1
		WHERE rule.retired_at IS NULL
	`); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT OR IGNORE INTO output_binding_starts(
			binding_id, ledger_epoch, start_after_pub_seq
		)
		SELECT binding.binding_id, cursor.ledger_epoch, 0
		FROM output_profile_rule_bindings AS binding
		JOIN semantic_rules_v3 AS rule ON rule.rule_id = binding.rule_id
		JOIN edge_signals AS signal ON signal.signal_ref = rule.signal_ref
		JOIN accepted_cursors AS cursor
			ON cursor.edge_node_id = signal.edge_node_id
		WHERE binding.state = 'active'
	`); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) listSemanticV3Candidates(
	ctx context.Context,
	limit int,
) ([]semanticV3Candidate, error) {
	rows, err := store.db.QueryContext(ctx, `
		WITH candidates AS (
			SELECT rule.rule_id, revision.revision, rule.signal_ref,
				signal.edge_node_id, signal.series_key, revision.series_id,
				rule.kind, revision.spec_json,
				calibration.revision AS calibration_revision,
				calibration.scale, calibration."offset",
				raw.ledger_epoch, raw.pub_seq, raw.record_json, raw.received_at,
				ROW_NUMBER() OVER (
					PARTITION BY rule.rule_id
					ORDER BY raw.received_at, raw.ledger_epoch, raw.pub_seq
				) AS rule_rank
			FROM semantic_rules_v3 AS rule
			JOIN semantic_rule_revisions_v3 AS revision
				ON revision.rule_id = rule.rule_id
			JOIN edge_signals AS signal
				ON signal.signal_ref = rule.signal_ref
			JOIN raw_records AS raw
				ON raw.edge_node_id = signal.edge_node_id
				AND json_extract(raw.record_json, '$.series_key') = signal.series_key
			JOIN accepted_cursors AS accepted
				ON accepted.edge_node_id = raw.edge_node_id
				AND accepted.ledger_epoch = raw.ledger_epoch
			JOIN semantic_rule_starts_v3 AS starts
				ON starts.rule_id = rule.rule_id
				AND starts.rule_revision = revision.revision
				AND starts.ledger_epoch = raw.ledger_epoch
			LEFT JOIN semantic_rule_ends_v3 AS ends
				ON ends.rule_id = rule.rule_id
				AND ends.rule_revision = revision.revision
				AND ends.ledger_epoch = raw.ledger_epoch
			JOIN signal_calibration_starts_v3 AS calibration_start
				ON calibration_start.signal_ref = signal.signal_ref
				AND calibration_start.ledger_epoch = raw.ledger_epoch
				AND calibration_start.start_after_pub_seq < raw.pub_seq
			JOIN signal_calibration_revisions_v3 AS calibration
				ON calibration.signal_ref = calibration_start.signal_ref
				AND calibration.revision = calibration_start.calibration_revision
			WHERE raw.pub_seq <= accepted.accepted_through
				AND raw.pub_seq > starts.start_after_pub_seq
				AND (revision.active = 1 OR ends.rule_id IS NOT NULL)
				AND (ends.end_at_pub_seq IS NULL OR raw.pub_seq <= ends.end_at_pub_seq)
				AND NOT EXISTS (
					SELECT 1 FROM signal_calibration_starts_v3 AS newer
					WHERE newer.signal_ref = calibration_start.signal_ref
						AND newer.ledger_epoch = calibration_start.ledger_epoch
						AND (
							newer.start_after_pub_seq >
								calibration_start.start_after_pub_seq
							OR (
								newer.start_after_pub_seq =
									calibration_start.start_after_pub_seq
								AND newer.calibration_revision >
									calibration_start.calibration_revision
							)
						)
						AND newer.start_after_pub_seq < raw.pub_seq
				)
				AND NOT EXISTS (
					SELECT 1 FROM semantic_projection_receipts_v3 AS receipt
					WHERE receipt.rule_id = rule.rule_id
						AND receipt.ledger_epoch = raw.ledger_epoch
						AND receipt.pub_seq = raw.pub_seq
				)
				AND NOT EXISTS (
					SELECT 1 FROM semantic_counter_resets_v3 AS reset
					WHERE reset.rule_id = rule.rule_id
						AND reset.applied_at IS NULL
						AND (
							NOT EXISTS (
								SELECT 1
								FROM semantic_counter_reset_boundaries_v3 AS boundary
								WHERE boundary.reset_id = reset.reset_id
									AND boundary.ledger_epoch = raw.ledger_epoch
							)
							OR raw.pub_seq > (
								SELECT boundary.apply_after_pub_seq
								FROM semantic_counter_reset_boundaries_v3 AS boundary
								WHERE boundary.reset_id = reset.reset_id
									AND boundary.ledger_epoch = raw.ledger_epoch
							)
						)
				)
		)
		SELECT rule_id, revision, signal_ref, edge_node_id, series_key,
			series_id, kind, spec_json, calibration_revision, scale, "offset",
			ledger_epoch, pub_seq, record_json, received_at
		FROM candidates
		ORDER BY rule_rank, received_at, ledger_epoch, pub_seq, rule_id, revision
		LIMIT ?
	`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	candidates := make([]semanticV3Candidate, 0)
	for rows.Next() {
		var candidate semanticV3Candidate
		if err := rows.Scan(
			&candidate.RuleID, &candidate.RuleRevision,
			&candidate.SignalRef, &candidate.EdgeNodeID, &candidate.SeriesKey,
			&candidate.SeriesID, &candidate.Kind, &candidate.SpecJSON,
			&candidate.CalibrationRevision, &candidate.Scale, &candidate.Offset,
			&candidate.LedgerEpoch, &candidate.PubSeq, &candidate.RecordJSON,
			&candidate.ReceivedAt,
		); err != nil {
			return nil, err
		}
		candidates = append(candidates, candidate)
	}
	return candidates, rows.Err()
}

func (store *Store) projectSemanticV3Candidate(
	ctx context.Context,
	candidate semanticV3Candidate,
) error {
	var spec semantics.RuleSpec
	if err := json.Unmarshal(candidate.SpecJSON, &spec); err != nil {
		return err
	}
	if spec.Kind != candidate.Kind {
		return errors.New("semantic rule kind conflicts with candidate revision")
	}
	var record struct {
		Family    string    `json:"family"`
		SeriesKey string    `json:"series_key"`
		Values    []float64 `json:"values"`
		EventTime *int64    `json:"event_time"`
	}
	if err := json.Unmarshal(candidate.RecordJSON, &record); err != nil {
		return err
	}
	if record.Family != "measurement" || record.SeriesKey != candidate.SeriesKey ||
		len(record.Values) != 1 || record.EventTime == nil || *record.EventTime < 0 {
		return errors.New("invalid scalar measurement for semantic rule")
	}
	calibrated, err := (semantics.Calibration{
		Scale: candidate.Scale, Offset: candidate.Offset,
	}).Apply(record.Values[0])
	if err != nil {
		return err
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer func() { _ = tx.Rollback() }()
	var receiptExists bool
	if err := tx.QueryRowContext(ctx, `
		SELECT EXISTS(
			SELECT 1 FROM semantic_projection_receipts_v3
			WHERE rule_id = ? AND ledger_epoch = ? AND pub_seq = ?
		)
	`, candidate.RuleID, candidate.LedgerEpoch, candidate.PubSeq).
		Scan(&receiptExists); err != nil {
		return err
	}
	if receiptExists {
		return tx.Commit()
	}

	state := semantics.State{}
	var initialized bool
	var detectorActive bool
	var appliedRuleRevision int64
	var appliedCalibrationRevision int64
	var appliedLedgerEpoch string
	var appliedSeriesID string
	nextSequence := int64(1)
	if err := tx.QueryRowContext(ctx, `
		SELECT initialized, detector_active, counter,
			pending, pending_active, pending_since,
			applied_rule_revision, applied_calibration_revision,
			applied_ledger_epoch, next_sequence, applied_series_id
		FROM semantic_rule_runtime_v3 WHERE rule_id = ?
	`, candidate.RuleID).Scan(
		&initialized, &detectorActive, &state.Counter,
		&state.Pending, &state.PendingActive, &state.PendingSince,
		&appliedRuleRevision, &appliedCalibrationRevision,
		&appliedLedgerEpoch, &nextSequence, &appliedSeriesID,
	); err != nil {
		return err
	}
	if appliedSeriesID == candidate.SeriesID &&
		appliedRuleRevision == candidate.RuleRevision &&
		appliedCalibrationRevision == candidate.CalibrationRevision &&
		appliedLedgerEpoch == candidate.LedgerEpoch {
		state.Initialized = initialized
		state.Active = detectorActive
	} else {
		state.Initialized = false
		state.Active = false
		state.Pending = false
		state.PendingActive = false
		state.PendingSince = 0
		if appliedSeriesID != candidate.SeriesID {
			state.Counter = 0
			nextSequence = 1
		}
	}
	result, nextState, err := semantics.EvaluateRule(
		spec, state, calibrated, candidate.ReceivedAt,
	)
	if err != nil {
		return err
	}
	var observationID any
	if result.Emitted {
		valueJSON, err := semanticResultJSON(result)
		if err != nil {
			return err
		}
		id := uuid.NewSHA1(
			uuid.NameSpaceOID,
			[]byte(fmt.Sprintf(
				"semantic-v3:%s:%s:%d",
				candidate.RuleID, candidate.LedgerEpoch, candidate.PubSeq,
			)),
		).String()
		observationID = id
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO semantic_observations_v3(
				observation_id, rule_id, rule_revision, calibration_revision,
				series_id, sequence, kind, value_json, signal_ref, edge_node_id,
				ledger_epoch, source_pub_seq, observed_at, created_at
			) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
		`, id, candidate.RuleID, candidate.RuleRevision,
			candidate.CalibrationRevision, candidate.SeriesID, nextSequence,
			candidate.Kind, valueJSON, candidate.SignalRef, candidate.EdgeNodeID,
			candidate.LedgerEpoch, candidate.PubSeq, *record.EventTime,
			time.Now().UnixMilli()); err != nil {
			return err
		}
		nextSequence++
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_projection_receipts_v3(
			rule_id, ledger_epoch, pub_seq, rule_revision,
			calibration_revision, observation_id
		) VALUES (?, ?, ?, ?, ?, ?)
	`, candidate.RuleID, candidate.LedgerEpoch, candidate.PubSeq,
		candidate.RuleRevision, candidate.CalibrationRevision,
		observationID); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE semantic_rule_runtime_v3
		SET initialized = ?, detector_active = ?, counter = ?,
			pending = ?, pending_active = ?, pending_since = ?,
			applied_rule_revision = ?, applied_calibration_revision = ?,
			applied_ledger_epoch = ?, next_sequence = ?, applied_series_id = ?
		WHERE rule_id = ?
	`, nextState.Initialized, nextState.Active, nextState.Counter,
		nextState.Pending, nextState.PendingActive, nextState.PendingSince,
		candidate.RuleRevision, candidate.CalibrationRevision,
		candidate.LedgerEpoch, nextSequence, candidate.SeriesID,
		candidate.RuleID); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) recordSemanticV3Failure(
	ctx context.Context,
	candidate semanticV3Candidate,
	projectionErr error,
) {
	message := projectionErr.Error()
	if len(message) > 256 {
		message = message[:256]
	}
	_, _ = store.db.ExecContext(ctx, `
		INSERT INTO semantic_projection_failures_v3(
			rule_id, ledger_epoch, pub_seq, error_text, attempts, last_failed_at
		) VALUES (?, ?, ?, ?, 1, ?)
		ON CONFLICT(rule_id, ledger_epoch, pub_seq) DO UPDATE SET
			error_text = excluded.error_text,
			attempts = attempts + 1,
			last_failed_at = excluded.last_failed_at
	`, candidate.RuleID, candidate.LedgerEpoch, candidate.PubSeq,
		message, time.Now().UnixMilli())
}

func (store *Store) clearSemanticV3Failure(
	ctx context.Context,
	candidate semanticV3Candidate,
) {
	_, _ = store.db.ExecContext(ctx, `
		DELETE FROM semantic_projection_failures_v3
		WHERE rule_id = ? AND ledger_epoch = ? AND pub_seq = ?
	`, candidate.RuleID, candidate.LedgerEpoch, candidate.PubSeq)
}

func (store *Store) SemanticRuleProjectionFailureCount(
	ctx context.Context,
) (int64, error) {
	var count int64
	err := store.db.QueryRowContext(
		ctx, `SELECT count(*) FROM semantic_projection_failures_v3`,
	).Scan(&count)
	return count, err
}

func (store *Store) ListSemanticRuleObservations(
	ctx context.Context,
	limit int,
) ([]semantics.RuleObservation, error) {
	if limit < 1 || limit > 10_000 {
		return nil, errors.New("semantic rule observation limit must be between 1 and 10000")
	}
	rows, err := store.db.QueryContext(ctx, `
		SELECT observation_row_id, observation_id, rule_id, rule_revision,
			calibration_revision, series_id, sequence, kind, value_json,
			signal_ref, edge_node_id, ledger_epoch, source_pub_seq,
			observed_at, created_at
		FROM semantic_observations_v3
		ORDER BY observation_row_id
		LIMIT ?
	`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	observations := make([]semantics.RuleObservation, 0)
	for rows.Next() {
		var observation semantics.RuleObservation
		var value []byte
		if err := rows.Scan(
			&observation.RowID, &observation.ObservationID,
			&observation.RuleID, &observation.RuleRevision,
			&observation.CalibrationRevision, &observation.SeriesID,
			&observation.Sequence, &observation.Kind, &value,
			&observation.SignalRef, &observation.EdgeNodeID,
			&observation.LedgerEpoch, &observation.SourcePubSeq,
			&observation.ObservedAt, &observation.CreatedAt,
		); err != nil {
			return nil, err
		}
		observation.Value = value
		observations = append(observations, observation)
	}
	return observations, rows.Err()
}
