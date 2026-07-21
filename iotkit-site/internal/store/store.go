package store

import (
	"bytes"
	"context"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantic"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
	_ "modernc.org/sqlite"
)

var (
	ErrConflict                = errors.New("raw record content conflict")
	ErrGap                     = errors.New("batch starts after the contiguous cursor")
	ErrArchiveRecoveryRequired = errors.New("restored Site is missing Edge records and requires an explicit recovery decision")
)

type Store struct {
	db *sql.DB
}

type RawRecord struct {
	EdgeNodeID    string          `json:"edge_node_id"`
	LedgerEpoch   string          `json:"ledger_epoch"`
	PubSeq        int64           `json:"pub_seq"`
	PublicationID string          `json:"publication_id"`
	Record        json.RawMessage `json:"record"`
	ReceivedAt    int64           `json:"received_at"`
}

type SemanticEvent struct {
	EventID         string           `json:"event_id"`
	MappingID       string           `json:"mapping_id"`
	MappingRevision int64            `json:"mapping_revision"`
	EventSequence   int64            `json:"event_sequence"`
	Meaning         semantic.Meaning `json:"meaning"`
	EdgeNodeID      string           `json:"edge_node_id"`
	LedgerEpoch     string           `json:"ledger_epoch"`
	SourcePubSeq    int64            `json:"source_pub_seq"`
	SourceSeriesKey string           `json:"source_series_key"`
	OccurredAt      int64            `json:"occurred_at"`
	CreatedAt       int64            `json:"created_at"`
}

func Open(path string) (*Store, error) {
	return open(path, "")
}

func OpenWithSiteID(path string, siteID string) (*Store, error) {
	if !siteIDPattern.MatchString(siteID) {
		return nil, errors.New("configured Site ID is invalid")
	}
	return open(path, siteID)
}

func open(path string, configuredSiteID string) (*Store, error) {
	db, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	store := &Store{db: db}
	if err := store.initialize(configuredSiteID); err != nil {
		_ = db.Close()
		return nil, err
	}
	return store, nil
}

func (store *Store) Close() error {
	return store.db.Close()
}

func (store *Store) initialize(configuredSiteID string) error {
	if err := store.rejectLegacyGatewaySchema(); err != nil {
		return err
	}
	if _, err := store.db.Exec(`
		PRAGMA journal_mode = WAL;
		PRAGMA synchronous = FULL;
		PRAGMA foreign_keys = ON;
	`); err != nil {
		return err
	}
	if err := applyMigrations(
		context.Background(),
		store.db,
		configuredSiteID,
	); err != nil {
		return err
	}
	if configuredSiteID != "" {
		var storedSiteID string
		if err := store.db.QueryRow(
			"SELECT site_id FROM site_meta WHERE singleton = 1",
		).Scan(&storedSiteID); err != nil {
			return err
		}
		if storedSiteID != configuredSiteID {
			return errors.New("configured Site ID does not match the existing database")
		}
	}
	return store.validateSiteIdentity(context.Background())
}

func (store *Store) rejectLegacyGatewaySchema() error {
	for _, table := range []string{"raw_records", "accepted_cursors"} {
		var legacyColumns int
		if err := store.db.QueryRow(
			"SELECT count(*) FROM pragma_table_info(?) WHERE name = 'gateway_identity'",
			table,
		).Scan(&legacyColumns); err != nil {
			return err
		}
		if legacyColumns > 0 {
			return errors.New("unsupported pre-release Site database; recreate it")
		}
	}
	return nil
}

func putSemanticMappingTx(
	ctx context.Context,
	tx *sql.Tx,
	spec semantic.MappingSpec,
	precondition siteapp.RevisionPrecondition,
) (semantic.Mapping, error) {
	var noMapping semantic.Mapping
	var mappingID string
	var revision int64
	var previousRevision int64
	err := tx.QueryRowContext(ctx, `
		SELECT mapping_id, revision
		FROM semantic_mappings
		WHERE edge_node_id = ? AND series_key = ? AND active = 1
	`, spec.EdgeNodeID, spec.SeriesKey).Scan(&mappingID, &revision)
	if errors.Is(err, sql.ErrNoRows) {
		if err := checkRevisionPrecondition(precondition, false, 0); err != nil {
			return noMapping, err
		}
		mappingID, err = newSemanticMappingID()
		if err != nil {
			return noMapping, err
		}
		revision = 1
	} else if err == nil {
		if err := checkRevisionPrecondition(precondition, true, revision); err != nil {
			return noMapping, err
		}
		previousRevision = revision
		revision++
	} else {
		return noMapping, err
	}

	if previousRevision > 0 {
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO semantic_mapping_ends (
				mapping_id, mapping_revision, ledger_epoch, end_at_pub_seq
			)
			SELECT ?, ?, ledger_epoch, accepted_through
			FROM accepted_cursors
			WHERE edge_node_id = ?
		`, mappingID, previousRevision, spec.EdgeNodeID); err != nil {
			return noMapping, err
		}
	}

	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_mapping_starts (
			mapping_id, mapping_revision, ledger_epoch, start_after_pub_seq
		)
		SELECT ?, ?, ledger_epoch, accepted_through
		FROM accepted_cursors
		WHERE edge_node_id = ?
	`, mappingID, revision, spec.EdgeNodeID); err != nil {
		return noMapping, err
	}

	if previousRevision > 0 {
		if _, err := tx.ExecContext(ctx, `
			UPDATE semantic_mappings
			SET active = 0
			WHERE mapping_id = ? AND revision = ? AND active = 1
		`, mappingID, previousRevision); err != nil {
			return noMapping, err
		}
	}

	createdAt := time.Now().UnixMilli()
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_mappings (
			mapping_id, revision, edge_node_id, series_key, meaning,
			trigger_mode, active_value, active, created_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?)
	`, mappingID, revision, spec.EdgeNodeID, spec.SeriesKey, spec.Meaning,
		spec.TriggerMode, spec.ActiveValue, createdAt); err != nil {
		return noMapping, err
	}

	return semantic.Mapping{
		ID:          mappingID,
		Revision:    revision,
		MappingSpec: spec,
		Active:      true,
		CreatedAt:   createdAt,
	}, nil
}

func checkRevisionPrecondition(precondition siteapp.RevisionPrecondition, exists bool, revision int64) error {
	if precondition.Expected == nil {
		return nil
	}
	if !exists {
		if *precondition.Expected == 0 {
			return nil
		}
		return siteapp.ErrRevisionMismatch
	}
	if *precondition.Expected != revision {
		return siteapp.ErrRevisionMismatch
	}
	return nil
}

func (store *Store) ListSemanticMappings(ctx context.Context) ([]semantic.Mapping, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT mapping_id, revision, edge_node_id, series_key, meaning,
			trigger_mode, active_value, active, created_at
		FROM semantic_mappings
		ORDER BY mapping_id, revision
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	mappings := make([]semantic.Mapping, 0)
	for rows.Next() {
		var mapping semantic.Mapping
		if err := rows.Scan(
			&mapping.ID,
			&mapping.Revision,
			&mapping.EdgeNodeID,
			&mapping.SeriesKey,
			&mapping.Meaning,
			&mapping.TriggerMode,
			&mapping.ActiveValue,
			&mapping.Active,
			&mapping.CreatedAt,
		); err != nil {
			return nil, err
		}
		mappings = append(mappings, mapping)
	}
	return mappings, rows.Err()
}

type semanticCandidate struct {
	MappingID       string
	MappingRevision int64
	Meaning         semantic.Meaning
	TriggerMode     semantic.TriggerMode
	ActiveValue     int
	EdgeNodeID      string
	SeriesKey       string
	LedgerEpoch     string
	PubSeq          int64
	Record          []byte
}

func (store *Store) ProjectSemanticEvents(ctx context.Context, limit int) (int, error) {
	if limit < 1 || limit > 10_000 {
		return 0, errors.New("semantic projection limit must be between 1 and 10000")
	}

	candidates, err := store.listSemanticCandidates(ctx, limit)
	if err != nil {
		return 0, err
	}

	processed := 0
	type mappingRevision struct {
		id       string
		revision int64
	}
	failedMappings := make(map[mappingRevision]struct{})
	var projectionErrors []error
	for _, candidate := range candidates {
		key := mappingRevision{id: candidate.MappingID, revision: candidate.MappingRevision}
		if _, failed := failedMappings[key]; failed {
			continue
		}
		projected, err := store.projectSemanticCandidate(ctx, candidate)
		if err != nil {
			if ctxErr := ctx.Err(); ctxErr != nil {
				return processed, ctxErr
			}
			failedMappings[key] = struct{}{}
			projectionErrors = append(projectionErrors, err)
			continue
		}
		if projected {
			processed++
		}
	}
	return processed, errors.Join(projectionErrors...)
}

func (store *Store) listSemanticCandidates(ctx context.Context, limit int) ([]semanticCandidate, error) {
	rows, err := store.db.QueryContext(ctx, `
		WITH ranked_candidates AS (
			SELECT m.mapping_id, m.revision, m.meaning, m.trigger_mode, m.active_value,
				m.edge_node_id, m.series_key, r.ledger_epoch, r.pub_seq, r.record_json,
				r.received_at,
				ROW_NUMBER() OVER (
					PARTITION BY m.mapping_id, m.revision
					ORDER BY r.received_at, r.ledger_epoch, r.pub_seq
				) AS candidate_rank
			FROM semantic_mappings AS m
			JOIN raw_records AS r
				ON r.edge_node_id = m.edge_node_id
			LEFT JOIN semantic_mapping_starts AS starts
				ON starts.mapping_id = m.mapping_id
				AND starts.mapping_revision = m.revision
				AND starts.ledger_epoch = r.ledger_epoch
			LEFT JOIN semantic_mapping_ends AS ends
				ON ends.mapping_id = m.mapping_id
				AND ends.mapping_revision = m.revision
				AND ends.ledger_epoch = r.ledger_epoch
			WHERE json_extract(CAST(r.record_json AS TEXT), '$.series_key') = m.series_key
				AND r.pub_seq > COALESCE(starts.start_after_pub_seq, 0)
				AND (
					m.active = 1
					OR (ends.end_at_pub_seq IS NOT NULL AND r.pub_seq <= ends.end_at_pub_seq)
				)
				AND NOT EXISTS (
					SELECT 1
					FROM semantic_results AS results
					WHERE results.mapping_id = m.mapping_id
						AND results.mapping_revision = m.revision
						AND results.ledger_epoch = r.ledger_epoch
						AND results.pub_seq = r.pub_seq
				)
		)
		SELECT mapping_id, revision, meaning, trigger_mode, active_value,
			edge_node_id, series_key, ledger_epoch, pub_seq, record_json
		FROM ranked_candidates
		ORDER BY candidate_rank, mapping_id, revision, received_at, ledger_epoch, pub_seq
		LIMIT ?
	`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	candidates := make([]semanticCandidate, 0)
	for rows.Next() {
		var candidate semanticCandidate
		if err := rows.Scan(
			&candidate.MappingID,
			&candidate.MappingRevision,
			&candidate.Meaning,
			&candidate.TriggerMode,
			&candidate.ActiveValue,
			&candidate.EdgeNodeID,
			&candidate.SeriesKey,
			&candidate.LedgerEpoch,
			&candidate.PubSeq,
			&candidate.Record,
		); err != nil {
			return nil, err
		}
		candidates = append(candidates, candidate)
	}
	return candidates, rows.Err()
}

func (store *Store) projectSemanticCandidate(ctx context.Context, candidate semanticCandidate) (bool, error) {
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return false, err
	}
	defer func() { _ = tx.Rollback() }()

	eligible, err := semanticCandidateEligible(ctx, tx, candidate)
	if err != nil {
		return false, err
	}
	if !eligible {
		return false, nil
	}

	current, occurredAt, err := decodeSemanticContact(candidate.Record)
	if err != nil {
		return false, fmt.Errorf(
			"project semantic input %s revision %d %s/%d: %w",
			candidate.MappingID, candidate.MappingRevision, candidate.LedgerEpoch, candidate.PubSeq, err,
		)
	}

	var lastValue sql.NullInt64
	var nextEventSequence int64
	err = tx.QueryRowContext(ctx, `
		SELECT last_value, next_event_sequence
		FROM semantic_mapping_state
		WHERE mapping_id = ? AND mapping_revision = ?
	`, candidate.MappingID, candidate.MappingRevision).Scan(&lastValue, &nextEventSequence)
	if errors.Is(err, sql.ErrNoRows) {
		nextEventSequence = 1
	} else if err != nil {
		return false, err
	}

	var previous *int
	if lastValue.Valid {
		value := int(lastValue.Int64)
		previous = &value
	}
	emit, nextValue, err := semantic.Evaluate(candidate.TriggerMode, candidate.ActiveValue, previous, current)
	if err != nil {
		return false, err
	}

	var emittedEventID any
	if emit {
		eventID := semanticEventID(
			candidate.MappingID,
			candidate.MappingRevision,
			candidate.EdgeNodeID,
			candidate.LedgerEpoch,
			candidate.PubSeq,
		)
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO semantic_events (
				event_id, mapping_id, mapping_revision, event_sequence, meaning,
				edge_node_id, ledger_epoch, source_pub_seq, source_series_key,
				occurred_at, created_at
			) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
		`, eventID, candidate.MappingID, candidate.MappingRevision, nextEventSequence,
			candidate.Meaning, candidate.EdgeNodeID, candidate.LedgerEpoch, candidate.PubSeq,
			candidate.SeriesKey, occurredAt, time.Now().UnixMilli()); err != nil {
			return false, err
		}
		emittedEventID = eventID
		nextEventSequence++
	}

	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_mapping_state (
			mapping_id, mapping_revision, last_value, next_event_sequence
		) VALUES (?, ?, ?, ?)
		ON CONFLICT(mapping_id, mapping_revision) DO UPDATE SET
			last_value = excluded.last_value,
			next_event_sequence = excluded.next_event_sequence
	`, candidate.MappingID, candidate.MappingRevision, nextValue, nextEventSequence); err != nil {
		return false, err
	}

	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_results (
			mapping_id, mapping_revision, ledger_epoch, pub_seq, emitted_event_id
		) VALUES (?, ?, ?, ?, ?)
	`, candidate.MappingID, candidate.MappingRevision, candidate.LedgerEpoch,
		candidate.PubSeq, emittedEventID); err != nil {
		return false, err
	}

	if err := tx.Commit(); err != nil {
		return false, err
	}
	return true, nil
}

func semanticCandidateEligible(ctx context.Context, tx *sql.Tx, candidate semanticCandidate) (bool, error) {
	var eligible int
	err := tx.QueryRowContext(ctx, `
		SELECT EXISTS (
			SELECT 1
			FROM semantic_mappings AS m
			LEFT JOIN semantic_mapping_starts AS starts
				ON starts.mapping_id = m.mapping_id
				AND starts.mapping_revision = m.revision
				AND starts.ledger_epoch = ?
			LEFT JOIN semantic_mapping_ends AS ends
				ON ends.mapping_id = m.mapping_id
				AND ends.mapping_revision = m.revision
				AND ends.ledger_epoch = ?
			WHERE m.mapping_id = ? AND m.revision = ?
				AND ? > COALESCE(starts.start_after_pub_seq, 0)
				AND (
					m.active = 1
					OR (ends.end_at_pub_seq IS NOT NULL AND ? <= ends.end_at_pub_seq)
				)
				AND NOT EXISTS (
					SELECT 1
					FROM semantic_results AS results
					WHERE results.mapping_id = m.mapping_id
						AND results.mapping_revision = m.revision
						AND results.ledger_epoch = ?
						AND results.pub_seq = ?
				)
		)
	`, candidate.LedgerEpoch, candidate.LedgerEpoch, candidate.MappingID,
		candidate.MappingRevision, candidate.PubSeq, candidate.PubSeq,
		candidate.LedgerEpoch, candidate.PubSeq).Scan(&eligible)
	return eligible == 1, err
}

func decodeSemanticContact(record []byte) (int, int64, error) {
	var measurement struct {
		Family    json.RawMessage   `json:"family"`
		Values    []json.RawMessage `json:"values"`
		EventTime json.RawMessage   `json:"event_time"`
	}
	if err := json.Unmarshal(record, &measurement); err != nil {
		return 0, 0, err
	}
	var family string
	if len(measurement.Family) == 0 || bytes.Equal(measurement.Family, []byte("null")) {
		return 0, 0, errors.New("semantic input family must be measurement")
	}
	if err := json.Unmarshal(measurement.Family, &family); err != nil || family != "measurement" {
		return 0, 0, errors.New("semantic input family must be measurement")
	}
	if len(measurement.Values) != 1 {
		return 0, 0, errors.New("contact values must contain exactly one scalar")
	}
	value, err := json.Number(measurement.Values[0]).Float64()
	if err != nil {
		return 0, 0, errors.New("contact value must be a number")
	}
	if value != 0 && value != 1 {
		return 0, 0, errors.New("contact value must be 0 or 1")
	}
	if len(measurement.EventTime) == 0 || bytes.Equal(measurement.EventTime, []byte("null")) {
		return 0, 0, errors.New("event_time must be present and non-null")
	}
	var eventTime int64
	if err := json.Unmarshal(measurement.EventTime, &eventTime); err != nil {
		return 0, 0, errors.New("event_time must be an integer")
	}
	if eventTime < 0 {
		return 0, 0, errors.New("event_time must be non-negative")
	}
	return int(value), eventTime, nil
}

func semanticEventID(mappingID string, revision int64, edgeNodeID, ledgerEpoch string, pubSeq int64) string {
	digest := sha256.New()
	_, _ = fmt.Fprintf(digest, "%s\x00%d\x00%s\x00%s\x00%d", mappingID, revision, edgeNodeID, ledgerEpoch, pubSeq)
	return hex.EncodeToString(digest.Sum(nil))
}

func (store *Store) ListSemanticEvents(ctx context.Context, limit int) ([]SemanticEvent, error) {
	if limit < 1 || limit > 10_000 {
		return nil, errors.New("semantic event query limit must be between 1 and 10000")
	}
	rows, err := store.db.QueryContext(ctx, `
		SELECT event_id, mapping_id, mapping_revision, event_sequence, meaning,
			edge_node_id, ledger_epoch, source_pub_seq, source_series_key,
			occurred_at, created_at
		FROM semantic_events
		ORDER BY event_row_id
		LIMIT ?
	`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	events := make([]SemanticEvent, 0)
	for rows.Next() {
		var event SemanticEvent
		if err := rows.Scan(
			&event.EventID,
			&event.MappingID,
			&event.MappingRevision,
			&event.EventSequence,
			&event.Meaning,
			&event.EdgeNodeID,
			&event.LedgerEpoch,
			&event.SourcePubSeq,
			&event.SourceSeriesKey,
			&event.OccurredAt,
			&event.CreatedAt,
		); err != nil {
			return nil, err
		}
		events = append(events, event)
	}
	return events, rows.Err()
}

var newSemanticMappingID = generateSemanticMappingID

func generateSemanticMappingID() (string, error) {
	random := make([]byte, 16)
	if _, err := rand.Read(random); err != nil {
		return "", err
	}
	return "sm-" + hex.EncodeToString(random), nil
}

func (store *Store) AcceptBatch(ctx context.Context, batch contract.RecordBatch) (contract.AcceptedThrough, error) {
	var noAck contract.AcceptedThrough
	if err := batch.Validate(); err != nil {
		return noAck, err
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noAck, err
	}
	defer func() { _ = tx.Rollback() }()

	var active int
	if err := tx.QueryRowContext(ctx, `
		SELECT EXISTS(
			SELECT 1 FROM edge_activations
			WHERE edge_node_id = ? AND ledger_epoch = ? AND state = 'active'
		)
	`, batch.EdgeNodeID, batch.LedgerEpoch).Scan(&active); err != nil {
		return noAck, err
	}
	if active != 1 {
		return noAck, ErrEdgeNotActive
	}

	currentCursor, err := readCursor(ctx, tx, batch.EdgeNodeID, batch.LedgerEpoch)
	if err != nil {
		return noAck, err
	}
	restoreID, restoreCheckPending, err := pendingRestoredCursorCheckTx(
		ctx, tx, batch.EdgeNodeID, batch.LedgerEpoch,
	)
	if err != nil {
		return noAck, err
	}
	if batch.CursorStart > currentCursor+1 {
		if restoreCheckPending {
			now := time.Now().UnixMilli()
			if _, err := tx.ExecContext(ctx, `
				UPDATE site_restore_cursor_checks
				SET state = 'recovery_required', observed_cursor_start = ?, updated_at = ?
				WHERE restore_id = ? AND edge_node_id = ? AND ledger_epoch = ?
					AND state = 'pending'
			`, batch.CursorStart, now, restoreID, batch.EdgeNodeID, batch.LedgerEpoch); err != nil {
				return noAck, err
			}
			if _, err := tx.ExecContext(ctx, `
				UPDATE edge_activations
				SET state = 'recovery_hold', revision = revision + 1, updated_at = ?
				WHERE edge_node_id = ? AND ledger_epoch = ? AND state = 'active'
			`, now, batch.EdgeNodeID, batch.LedgerEpoch); err != nil {
				return noAck, err
			}
			if err := tx.Commit(); err != nil {
				return noAck, err
			}
			return noAck, ErrArchiveRecoveryRequired
		}
		return noAck, ErrGap
	}

	now := time.Now().UnixMilli()
	for index, raw := range batch.Records {
		pubSeq := batch.CursorStart + int64(index)
		compact, fingerprint, err := canonicalRecord(raw)
		if err != nil {
			return noAck, err
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT OR IGNORE INTO raw_records (
				edge_node_id, ledger_epoch, pub_seq, publication_id,
				record_json, record_sha256, received_at
			) VALUES (?, ?, ?, ?, ?, ?, ?)
		`, batch.EdgeNodeID, batch.LedgerEpoch, pubSeq, batch.PublicationID, compact, fingerprint[:], now); err != nil {
			return noAck, err
		}

		var storedFingerprint []byte
		if err := tx.QueryRowContext(ctx, `
			SELECT record_sha256 FROM raw_records
			WHERE edge_node_id = ? AND ledger_epoch = ? AND pub_seq = ?
		`, batch.EdgeNodeID, batch.LedgerEpoch, pubSeq).Scan(&storedFingerprint); err != nil {
			return noAck, err
		}
		if !bytes.Equal(storedFingerprint, fingerprint[:]) {
			return noAck, fmt.Errorf("%w at pub_seq %d", ErrConflict, pubSeq)
		}
	}

	acceptedThrough := currentCursor
	if batch.CursorEnd > acceptedThrough {
		acceptedThrough = batch.CursorEnd
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO accepted_cursors (
				edge_node_id, ledger_epoch, accepted_through, updated_at
			) VALUES (?, ?, ?, ?)
			ON CONFLICT(edge_node_id, ledger_epoch) DO UPDATE SET
				accepted_through = excluded.accepted_through,
				updated_at = excluded.updated_at
		`, batch.EdgeNodeID, batch.LedgerEpoch, acceptedThrough, now); err != nil {
			return noAck, err
		}
	}
	if restoreCheckPending {
		if _, err := tx.ExecContext(ctx, `
			UPDATE site_restore_cursor_checks
			SET state = 'verified', observed_cursor_start = ?, updated_at = ?
			WHERE restore_id = ? AND edge_node_id = ? AND ledger_epoch = ?
				AND state = 'pending'
		`, batch.CursorStart, time.Now().UnixMilli(), restoreID, batch.EdgeNodeID, batch.LedgerEpoch); err != nil {
			return noAck, err
		}
	}

	if err := tx.Commit(); err != nil {
		return noAck, err
	}
	return contract.AcceptedThrough{
		SchemaVersion:   contract.SchemaVersion,
		EdgeNodeID:      batch.EdgeNodeID,
		LedgerEpoch:     batch.LedgerEpoch,
		PublicationID:   batch.PublicationID,
		AcceptedThrough: acceptedThrough,
	}, nil
}

func (store *Store) ListRawRecords(ctx context.Context, limit int) ([]RawRecord, error) {
	if limit < 1 || limit > 10_000 {
		return nil, errors.New("raw record query limit must be between 1 and 10000")
	}
	rows, err := store.db.QueryContext(ctx, `
		SELECT edge_node_id, ledger_epoch, pub_seq, publication_id, record_json, received_at
		FROM raw_records
		ORDER BY received_at DESC, edge_node_id, ledger_epoch, pub_seq DESC
		LIMIT ?
	`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	records := make([]RawRecord, 0)
	for rows.Next() {
		var record RawRecord
		var payload []byte
		if err := rows.Scan(
			&record.EdgeNodeID,
			&record.LedgerEpoch,
			&record.PubSeq,
			&record.PublicationID,
			&payload,
			&record.ReceivedAt,
		); err != nil {
			return nil, err
		}
		record.Record = json.RawMessage(payload)
		records = append(records, record)
	}
	return records, rows.Err()
}

func readCursor(ctx context.Context, tx *sql.Tx, edgeNodeID, ledgerEpoch string) (int64, error) {
	var cursor int64
	err := tx.QueryRowContext(ctx, `
		SELECT accepted_through FROM accepted_cursors
		WHERE edge_node_id = ? AND ledger_epoch = ?
	`, edgeNodeID, ledgerEpoch).Scan(&cursor)
	if errors.Is(err, sql.ErrNoRows) {
		return 0, nil
	}
	return cursor, err
}

func canonicalRecord(raw json.RawMessage) ([]byte, [sha256.Size]byte, error) {
	var compact bytes.Buffer
	if err := json.Compact(&compact, raw); err != nil {
		return nil, [sha256.Size]byte{}, err
	}
	payload := compact.Bytes()
	return payload, sha256.Sum256(payload), nil
}
