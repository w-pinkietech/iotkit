package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"github.com/google/uuid"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

type semanticV2Candidate struct {
	DefinitionID       string
	DefinitionRevision int64
	SignalRef          string
	EdgeNodeID         string
	SeriesKey          string
	SeriesID           string
	SpecJSON           []byte
	LedgerEpoch        string
	PubSeq             int64
	RecordJSON         []byte
}

func (store *Store) ApplySemanticDefinition(
	ctx context.Context,
	actor siteapp.Actor,
	signalRef string,
	spec semantics.DefinitionSpec,
	precondition siteapp.RevisionPrecondition,
) (semantics.Definition, error) {
	var noDefinition semantics.Definition
	if err := actor.Validate(); err != nil {
		return noDefinition, err
	}
	if err := spec.Validate(); err != nil {
		return noDefinition, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noDefinition, err
	}
	defer func() { _ = tx.Rollback() }()

	var edgeNodeID string
	var seriesKey string
	if err := tx.QueryRowContext(ctx, `
		SELECT edge_node_id, series_key
		FROM site_signals
		WHERE signal_ref = ?
	`, signalRef).Scan(&edgeNodeID, &seriesKey); errors.Is(err, sql.ErrNoRows) {
		return noDefinition, siteapp.ErrNotFound
	} else if err != nil {
		return noDefinition, err
	}

	var definitionID string
	var currentRevision int64
	err = tx.QueryRowContext(ctx, `
		SELECT definition_id, revision
		FROM semantic_definitions_v2
		WHERE signal_ref = ? AND active = 1
	`, signalRef).Scan(&definitionID, &currentRevision)
	switch {
	case errors.Is(err, sql.ErrNoRows):
		if err := checkRevisionPrecondition(precondition, false, 0); err != nil {
			return noDefinition, err
		}
		definitionID, err = newResourceRef("sem_")
		if err != nil {
			return noDefinition, err
		}
	case err != nil:
		return noDefinition, err
	default:
		if err := checkRevisionPrecondition(precondition, true, currentRevision); err != nil {
			return noDefinition, err
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO semantic_definition_ends_v2 (
				definition_id, definition_revision, ledger_epoch, end_at_pub_seq
			)
			SELECT ?, ?, ledger_epoch, accepted_through
			FROM accepted_cursors
			WHERE edge_node_id = ?
		`, definitionID, currentRevision, edgeNodeID); err != nil {
			return noDefinition, err
		}
		if _, err := tx.ExecContext(ctx, `
			UPDATE semantic_definitions_v2 SET active = 0
			WHERE definition_id = ? AND revision = ? AND active = 1
		`, definitionID, currentRevision); err != nil {
			return noDefinition, err
		}
	}
	revision := currentRevision + 1
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_definition_starts_v2 (
			definition_id, definition_revision, ledger_epoch, start_after_pub_seq
		)
		SELECT ?, ?, ledger_epoch, accepted_through
		FROM accepted_cursors
		WHERE edge_node_id = ?
	`, definitionID, revision, edgeNodeID); err != nil {
		return noDefinition, err
	}
	specJSON, err := json.Marshal(spec)
	if err != nil {
		return noDefinition, err
	}
	definition := semantics.Definition{
		ID:             definitionID,
		Revision:       revision,
		SignalRef:      signalRef,
		EdgeNodeID:     edgeNodeID,
		SeriesKey:      seriesKey,
		SeriesID:       uuid.NewString(),
		DefinitionSpec: spec,
		Active:         true,
		CreatedAt:      time.Now().UnixMilli(),
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_definitions_v2 (
			definition_id, revision, signal_ref, edge_node_id, series_key,
			series_id, spec_json, active, created_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?)
	`, definition.ID, definition.Revision, definition.SignalRef,
		definition.EdgeNodeID, definition.SeriesKey, definition.SeriesID,
		specJSON, definition.CreatedAt); err != nil {
		return noDefinition, err
	}
	summary, _ := json.Marshal(struct {
		Kind     semantics.Kind `json:"kind"`
		Revision int64          `json:"revision"`
	}{
		Kind:     definition.Kind,
		Revision: definition.Revision,
	})
	if err := insertAuditEventTx(ctx, tx, siteapp.AuditEvent{
		OccurredAt:  definition.CreatedAt,
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "semantic_definition.put",
		ResourceRef: definition.ID,
		Outcome:     auditOutcomeSuccess,
		Summary:     summary,
	}); err != nil {
		return noDefinition, err
	}
	if err := tx.Commit(); err != nil {
		return noDefinition, err
	}
	return definition, nil
}

func (store *Store) ListSemanticDefinitions(
	ctx context.Context,
) ([]semantics.Definition, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT definition_id, revision, signal_ref, edge_node_id, series_key,
			series_id, spec_json, active, created_at
		FROM semantic_definitions_v2
		ORDER BY definition_id, revision
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	definitions := make([]semantics.Definition, 0)
	for rows.Next() {
		definition, err := scanSemanticDefinition(rows)
		if err != nil {
			return nil, err
		}
		definitions = append(definitions, definition)
	}
	return definitions, rows.Err()
}

func (store *Store) DeactivateSemanticDefinition(
	ctx context.Context,
	actor siteapp.Actor,
	signalRef string,
	precondition siteapp.RevisionPrecondition,
) (semantics.Definition, error) {
	var noDefinition semantics.Definition
	if err := actor.Validate(); err != nil {
		return noDefinition, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noDefinition, err
	}
	defer func() { _ = tx.Rollback() }()
	var definition semantics.Definition
	var specJSON []byte
	if err := tx.QueryRowContext(ctx, `
		SELECT definition_id, revision, signal_ref, edge_node_id, series_key,
			series_id, spec_json, active, created_at
		FROM semantic_definitions_v2
		WHERE signal_ref = ? AND active = 1
	`, signalRef).Scan(
		&definition.ID, &definition.Revision, &definition.SignalRef,
		&definition.EdgeNodeID, &definition.SeriesKey, &definition.SeriesID,
		&specJSON, &definition.Active, &definition.CreatedAt,
	); errors.Is(err, sql.ErrNoRows) {
		return noDefinition, siteapp.ErrNotFound
	} else if err != nil {
		return noDefinition, err
	}
	if err := json.Unmarshal(specJSON, &definition.DefinitionSpec); err != nil {
		return noDefinition, err
	}
	if err := checkRevisionPrecondition(precondition, true, definition.Revision); err != nil {
		return noDefinition, err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_definition_ends_v2 (
			definition_id, definition_revision, ledger_epoch, end_at_pub_seq
		)
		SELECT ?, ?, ledger_epoch, accepted_through
		FROM accepted_cursors WHERE edge_node_id = ?
	`, definition.ID, definition.Revision, definition.EdgeNodeID); err != nil {
		return noDefinition, err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE semantic_definitions_v2 SET active = 0
		WHERE definition_id = ? AND revision = ? AND active = 1
	`, definition.ID, definition.Revision); err != nil {
		return noDefinition, err
	}
	if err := insertAuditEventTx(ctx, tx, siteapp.AuditEvent{
		OccurredAt:  time.Now().UnixMilli(),
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "semantic_definition.deactivate",
		ResourceRef: definition.ID,
		Outcome:     auditOutcomeSuccess,
		Summary:     json.RawMessage(`{"active":false}`),
	}); err != nil {
		return noDefinition, err
	}
	if err := tx.Commit(); err != nil {
		return noDefinition, err
	}
	definition.Active = false
	return definition, nil
}

func scanSemanticDefinition(row rowScanner) (semantics.Definition, error) {
	var definition semantics.Definition
	var specJSON []byte
	if err := row.Scan(
		&definition.ID,
		&definition.Revision,
		&definition.SignalRef,
		&definition.EdgeNodeID,
		&definition.SeriesKey,
		&definition.SeriesID,
		&specJSON,
		&definition.Active,
		&definition.CreatedAt,
	); err != nil {
		return semantics.Definition{}, err
	}
	if err := json.Unmarshal(specJSON, &definition.DefinitionSpec); err != nil {
		return semantics.Definition{}, err
	}
	return definition, nil
}

func (store *Store) ProjectSemanticObservations(
	ctx context.Context,
	limit int,
) (int, error) {
	if limit < 1 || limit > 10_000 {
		return 0, errors.New("semantic observation projection limit must be between 1 and 10000")
	}
	candidates, err := store.listSemanticV2Candidates(ctx, limit)
	if err != nil {
		return 0, err
	}
	processed := 0
	failedDefinitions := make(map[string]struct{})
	var projectionErrors []error
	for _, candidate := range candidates {
		key := fmt.Sprintf("%s:%d", candidate.DefinitionID, candidate.DefinitionRevision)
		if _, failed := failedDefinitions[key]; failed {
			continue
		}
		if err := store.projectSemanticV2Candidate(ctx, candidate); err != nil {
			failedDefinitions[key] = struct{}{}
			store.recordSemanticV2Failure(ctx, candidate, err)
			projectionErrors = append(projectionErrors, err)
			continue
		}
		store.clearSemanticV2Failure(ctx, candidate)
		processed++
	}
	return processed, errors.Join(projectionErrors...)
}

func (store *Store) listSemanticV2Candidates(
	ctx context.Context,
	limit int,
) ([]semanticV2Candidate, error) {
	rows, err := store.db.QueryContext(ctx, `
		WITH candidates AS (
		SELECT definition.definition_id, definition.revision,
			definition.signal_ref, definition.edge_node_id, definition.series_key,
			definition.series_id, definition.spec_json,
			raw.ledger_epoch, raw.pub_seq, raw.record_json, raw.received_at,
			ROW_NUMBER() OVER (
				PARTITION BY definition.definition_id, definition.revision
				ORDER BY raw.received_at, raw.ledger_epoch, raw.pub_seq
			) AS definition_rank
		FROM semantic_definitions_v2 AS definition
		JOIN raw_records AS raw
			ON raw.edge_node_id = definition.edge_node_id
			AND json_extract(raw.record_json, '$.series_key') = definition.series_key
		JOIN accepted_cursors AS accepted
			ON accepted.edge_node_id = raw.edge_node_id
			AND accepted.ledger_epoch = raw.ledger_epoch
		LEFT JOIN semantic_definition_starts_v2 AS starts
			ON starts.definition_id = definition.definition_id
			AND starts.definition_revision = definition.revision
			AND starts.ledger_epoch = raw.ledger_epoch
		LEFT JOIN semantic_definition_ends_v2 AS ends
			ON ends.definition_id = definition.definition_id
			AND ends.definition_revision = definition.revision
			AND ends.ledger_epoch = raw.ledger_epoch
		WHERE raw.pub_seq <= accepted.accepted_through
			AND raw.pub_seq > COALESCE(starts.start_after_pub_seq, 0)
			AND (definition.active = 1 OR ends.ledger_epoch IS NOT NULL)
			AND (ends.end_at_pub_seq IS NULL OR raw.pub_seq <= ends.end_at_pub_seq)
			AND NOT EXISTS (
				SELECT 1 FROM semantic_results_v2 AS result
				WHERE result.definition_id = definition.definition_id
					AND result.definition_revision = definition.revision
					AND result.ledger_epoch = raw.ledger_epoch
					AND result.pub_seq = raw.pub_seq
			)
		)
		SELECT definition_id, revision, signal_ref, edge_node_id, series_key,
			series_id, spec_json, ledger_epoch, pub_seq, record_json
		FROM candidates
		ORDER BY definition_rank, received_at, ledger_epoch, pub_seq,
			definition_id, revision
		LIMIT ?
	`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	candidates := make([]semanticV2Candidate, 0)
	for rows.Next() {
		var candidate semanticV2Candidate
		if err := rows.Scan(
			&candidate.DefinitionID,
			&candidate.DefinitionRevision,
			&candidate.SignalRef,
			&candidate.EdgeNodeID,
			&candidate.SeriesKey,
			&candidate.SeriesID,
			&candidate.SpecJSON,
			&candidate.LedgerEpoch,
			&candidate.PubSeq,
			&candidate.RecordJSON,
		); err != nil {
			return nil, err
		}
		candidates = append(candidates, candidate)
	}
	return candidates, rows.Err()
}

func (store *Store) recordSemanticV2Failure(
	ctx context.Context,
	candidate semanticV2Candidate,
	projectionErr error,
) {
	message := projectionErr.Error()
	if len(message) > 256 {
		message = message[:256]
	}
	_, _ = store.db.ExecContext(ctx, `
		INSERT INTO semantic_projection_failures_v2 (
			definition_id, definition_revision, ledger_epoch, pub_seq,
			error_text, attempts, last_failed_at
		) VALUES (?, ?, ?, ?, ?, 1, ?)
		ON CONFLICT(definition_id, definition_revision, ledger_epoch, pub_seq)
		DO UPDATE SET error_text = excluded.error_text,
			attempts = attempts + 1, last_failed_at = excluded.last_failed_at
	`, candidate.DefinitionID, candidate.DefinitionRevision,
		candidate.LedgerEpoch, candidate.PubSeq, message, time.Now().UnixMilli())
}

func (store *Store) clearSemanticV2Failure(
	ctx context.Context,
	candidate semanticV2Candidate,
) {
	_, _ = store.db.ExecContext(ctx, `
		DELETE FROM semantic_projection_failures_v2
		WHERE definition_id = ? AND definition_revision = ?
			AND ledger_epoch = ? AND pub_seq = ?
	`, candidate.DefinitionID, candidate.DefinitionRevision,
		candidate.LedgerEpoch, candidate.PubSeq)
}

func (store *Store) SemanticProjectionFailureCount(ctx context.Context) (int64, error) {
	var count int64
	err := store.db.QueryRowContext(
		ctx, `SELECT count(*) FROM semantic_projection_failures_v2`,
	).Scan(&count)
	return count, err
}

func (store *Store) projectSemanticV2Candidate(
	ctx context.Context,
	candidate semanticV2Candidate,
) error {
	var spec semantics.DefinitionSpec
	if err := json.Unmarshal(candidate.SpecJSON, &spec); err != nil {
		return err
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
		return errors.New("invalid scalar measurement for semantic definition")
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer func() { _ = tx.Rollback() }()
	state := semantics.State{}
	nextSequence := int64(1)
	var initialized bool
	var active bool
	err = tx.QueryRowContext(ctx, `
		SELECT initialized, active, counter, next_sequence
		FROM semantic_definition_state_v2
		WHERE definition_id = ? AND definition_revision = ?
	`, candidate.DefinitionID, candidate.DefinitionRevision).Scan(
		&initialized,
		&active,
		&state.Counter,
		&nextSequence,
	)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return err
	}
	state.Initialized = initialized
	state.Active = active
	result, nextState, err := semantics.Evaluate(spec, state, record.Values[0])
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
				"%s:%d:%s:%d",
				candidate.DefinitionID,
				candidate.DefinitionRevision,
				candidate.LedgerEpoch,
				candidate.PubSeq,
			)),
		).String()
		observationID = id
		now := time.Now().UnixMilli()
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO semantic_observations_v2 (
				observation_id, series_id, sequence, definition_id,
				definition_revision, kind, value_json, signal_ref,
				edge_node_id, ledger_epoch, source_pub_seq, observed_at, created_at
			) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
		`, id, candidate.SeriesID, nextSequence, candidate.DefinitionID,
			candidate.DefinitionRevision, spec.Kind, valueJSON, candidate.SignalRef,
			candidate.EdgeNodeID, candidate.LedgerEpoch, candidate.PubSeq,
			*record.EventTime, now); err != nil {
			return err
		}
		nextSequence++
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_results_v2 (
			definition_id, definition_revision, ledger_epoch, pub_seq, observation_id
		) VALUES (?, ?, ?, ?, ?)
	`, candidate.DefinitionID, candidate.DefinitionRevision,
		candidate.LedgerEpoch, candidate.PubSeq, observationID); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_definition_state_v2 (
			definition_id, definition_revision, initialized, active,
			counter, next_sequence
		) VALUES (?, ?, ?, ?, ?, ?)
		ON CONFLICT(definition_id, definition_revision) DO UPDATE SET
			initialized = excluded.initialized,
			active = excluded.active,
			counter = excluded.counter,
			next_sequence = excluded.next_sequence
	`, candidate.DefinitionID, candidate.DefinitionRevision,
		nextState.Initialized, nextState.Active, nextState.Counter, nextSequence); err != nil {
		return err
	}
	return tx.Commit()
}

func semanticResultJSON(result semantics.Result) ([]byte, error) {
	switch {
	case result.Number != nil:
		return json.Marshal(*result.Number)
	case result.Boolean != nil:
		return json.Marshal(*result.Boolean)
	case result.Integer != nil:
		return json.Marshal(*result.Integer)
	default:
		return nil, errors.New("emitted semantic result has no value")
	}
}

func (store *Store) ListSemanticObservations(
	ctx context.Context,
	limit int,
) ([]semantics.Observation, error) {
	if limit < 1 || limit > 10_000 {
		return nil, errors.New("semantic observation limit must be between 1 and 10000")
	}
	rows, err := store.db.QueryContext(ctx, `
		SELECT observation_row_id, observation_id, series_id, sequence,
			definition_id, definition_revision, kind, value_json,
			signal_ref, edge_node_id, ledger_epoch, source_pub_seq,
			observed_at, created_at
		FROM semantic_observations_v2
		ORDER BY observation_row_id
		LIMIT ?
	`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	observations := make([]semantics.Observation, 0)
	for rows.Next() {
		var observation semantics.Observation
		var value []byte
		if err := rows.Scan(
			&observation.RowID,
			&observation.ObservationID,
			&observation.SeriesID,
			&observation.Sequence,
			&observation.DefinitionID,
			&observation.DefinitionRevision,
			&observation.Kind,
			&value,
			&observation.SignalRef,
			&observation.EdgeNodeID,
			&observation.LedgerEpoch,
			&observation.SourcePubSeq,
			&observation.ObservedAt,
			&observation.CreatedAt,
		); err != nil {
			return nil, err
		}
		observation.Value = value
		observations = append(observations, observation)
	}
	return observations, rows.Err()
}

const (
	PreviewTruncatedByTime       = "time"
	PreviewTruncatedByInputCount = "input_count"
)

type SemanticPreviewWindow struct {
	Inputs      []semantics.PreviewInput
	WindowStart int64
	WindowEnd   int64
	TruncatedBy string
}

func (store *Store) ListSemanticPreviewWindow(
	ctx context.Context,
	signalRef string,
	sinceReceivedAt int64,
	limit int,
) (SemanticPreviewWindow, error) {
	if limit < 1 || limit > 20_000 {
		return SemanticPreviewWindow{},
			errors.New("semantic preview limit must be between 1 and 20000")
	}
	var exists int
	if err := store.db.QueryRowContext(ctx, `
		SELECT EXISTS(
			SELECT 1 FROM site_signals WHERE signal_ref = ?
		)
	`, signalRef).Scan(&exists); err != nil {
		return SemanticPreviewWindow{}, err
	}
	if exists != 1 {
		return SemanticPreviewWindow{}, siteapp.ErrNotFound
	}

	rows, err := store.db.QueryContext(ctx, `
		SELECT raw.received_at, json_extract(raw.record_json, '$.values[0]')
		FROM site_signals AS signal
		JOIN raw_records AS raw
			ON raw.edge_node_id = signal.edge_node_id
			AND json_extract(raw.record_json, '$.series_key') = signal.series_key
		WHERE signal.signal_ref = ? AND raw.received_at >= ?
			AND json_type(raw.record_json, '$.values[0]') IN ('integer', 'real')
		ORDER BY raw.received_at DESC, raw.ledger_epoch DESC, raw.pub_seq DESC
		LIMIT ?
	`, signalRef, sinceReceivedAt, limit+1)
	if err != nil {
		return SemanticPreviewWindow{}, err
	}
	defer rows.Close()
	descending := make([]semantics.PreviewInput, 0, limit+1)
	for rows.Next() {
		var input semantics.PreviewInput
		if err := rows.Scan(&input.ReceivedAt, &input.Value); err != nil {
			return SemanticPreviewWindow{}, err
		}
		descending = append(descending, input)
	}
	if err := rows.Err(); err != nil {
		return SemanticPreviewWindow{}, err
	}

	window := SemanticPreviewWindow{}
	if len(descending) > limit {
		window.TruncatedBy = PreviewTruncatedByInputCount
		descending = descending[:limit]
	} else {
		var hasOlder int
		if err := store.db.QueryRowContext(ctx, `
			SELECT EXISTS(
				SELECT 1
				FROM site_signals AS signal
				JOIN raw_records AS raw
					ON raw.edge_node_id = signal.edge_node_id
					AND json_extract(raw.record_json, '$.series_key') = signal.series_key
				WHERE signal.signal_ref = ? AND raw.received_at < ?
			)
		`, signalRef, sinceReceivedAt).Scan(&hasOlder); err != nil {
			return SemanticPreviewWindow{}, err
		}
		if hasOlder == 1 {
			window.TruncatedBy = PreviewTruncatedByTime
		}
	}
	window.Inputs = make([]semantics.PreviewInput, len(descending))
	for index := range descending {
		window.Inputs[len(descending)-1-index] = descending[index]
	}
	if len(window.Inputs) > 0 {
		window.WindowStart = window.Inputs[0].ReceivedAt
		window.WindowEnd = window.Inputs[len(window.Inputs)-1].ReceivedAt
	}
	return window, nil
}
