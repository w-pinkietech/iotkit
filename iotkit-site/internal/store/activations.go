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
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

var (
	ErrEdgeNotActive      = errors.New("Edge is not active for Site custody")
	ErrActivationConflict = errors.New("Edge activation result conflict")
)

type EdgeActivationState string

const (
	EdgeDiscovered   EdgeActivationState = "discovered"
	EdgeActivating   EdgeActivationState = "activating"
	EdgeActive       EdgeActivationState = "active"
	EdgeRecoveryHold EdgeActivationState = "recovery_hold"
)

type EdgeActivation struct {
	EdgeRef          string              `json:"edge_ref"`
	EdgeNodeID       string              `json:"edge_node_id"`
	LedgerEpoch      string              `json:"ledger_epoch"`
	State            EdgeActivationState `json:"state"`
	ActivationID     string              `json:"activation_id,omitempty"`
	GrantRevision    uint64              `json:"grant_revision"`
	DisplayName      string              `json:"display_name"`
	Location         string              `json:"location"`
	Revision         int64               `json:"revision"`
	LastDescriptorAt *int64              `json:"last_descriptor_at,omitempty"`
	LastResultAt     *int64              `json:"last_result_at,omitempty"`
}

type ActivationCommand struct {
	ActivationID string          `json:"activation_id"`
	Topic        string          `json:"topic"`
	PayloadJSON  json.RawMessage `json:"payload_json"`
	Attempts     int64           `json:"attempts"`
	LastAttempt  *int64          `json:"last_attempt_at,omitempty"`
	CreatedAt    int64           `json:"created_at"`
}

func (store *Store) ListEdges(ctx context.Context) ([]EdgeActivation, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT edge_ref, edge_node_id, ledger_epoch, state,
			COALESCE(activation_id, ''), grant_revision,
			display_name, location, revision,
			last_descriptor_at, last_result_at
		FROM edge_activations
		ORDER BY created_at, edge_ref
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	edges := make([]EdgeActivation, 0)
	for rows.Next() {
		edge, err := scanEdge(rows)
		if err != nil {
			return nil, err
		}
		edges = append(edges, edge)
	}
	return edges, rows.Err()
}

func (store *Store) RequestEdgeActivation(
	ctx context.Context,
	actor siteapp.Actor,
	edgeRef string,
	precondition siteapp.RevisionPrecondition,
) (EdgeActivation, error) {
	var noEdge EdgeActivation
	if err := actor.Validate(); err != nil {
		return noEdge, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noEdge, err
	}
	defer func() { _ = tx.Rollback() }()

	edge, err := loadEdgeByRefTx(ctx, tx, edgeRef)
	if errors.Is(err, sql.ErrNoRows) {
		return noEdge, siteapp.ErrNotFound
	}
	if err != nil {
		return noEdge, err
	}
	if edge.State == EdgeActivating || edge.State == EdgeActive {
		return edge, nil
	}
	if edge.State != EdgeDiscovered {
		return noEdge, ErrActivationConflict
	}
	if err := checkRevisionPrecondition(precondition, true, edge.Revision); err != nil {
		return noEdge, err
	}

	var siteID string
	if err := tx.QueryRowContext(ctx, `
		SELECT site_id FROM site_meta WHERE singleton = 1
	`).Scan(&siteID); err != nil {
		return noEdge, err
	}
	activationID, err := randomPrefixedID("act-")
	if err != nil {
		return noEdge, err
	}
	now := time.Now().UnixMilli()
	request := contract.ActivationRequest{
		SchemaVersion:       contract.SchemaVersion,
		ActivationID:        activationID,
		SiteID:              siteID,
		EdgeNodeID:          edge.EdgeNodeID,
		ExpectedLedgerEpoch: edge.LedgerEpoch,
		GrantRevision:       1,
		IssuedAt:            now,
	}
	payload, err := request.Encode()
	if err != nil {
		return noEdge, err
	}
	topic := fmt.Sprintf(
		"iotkit/v1/edge-nodes/%s/activation/request",
		edge.EdgeNodeID,
	)
	if _, err := tx.ExecContext(ctx, `
		UPDATE edge_activations
		SET state = 'activating', activation_id = ?, grant_revision = 1,
			request_json = ?, result_json = NULL,
			revision = revision + 1, updated_at = ?
		WHERE edge_ref = ? AND state = 'discovered'
	`, activationID, payload, now, edgeRef); err != nil {
		return noEdge, err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO activation_command_outbox(
			activation_id, topic, payload_json, created_at
		) VALUES(?, ?, ?, ?)
	`, activationID, topic, payload, now); err != nil {
		return noEdge, err
	}
	summary, err := json.Marshal(struct {
		ActivationID string `json:"activation_id"`
		Revision     int64  `json:"revision"`
	}{activationID, edge.Revision + 1})
	if err != nil {
		return noEdge, err
	}
	if err := insertAuditEventTx(ctx, tx, siteapp.AuditEvent{
		OccurredAt:  now,
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "edge.activation.request",
		ResourceRef: edgeRef,
		Outcome:     auditOutcomeSuccess,
		Summary:     summary,
	}); err != nil {
		return noEdge, err
	}
	edge, err = loadEdgeByRefTx(ctx, tx, edgeRef)
	if err != nil {
		return noEdge, err
	}
	if err := tx.Commit(); err != nil {
		return noEdge, err
	}
	return edge, nil
}

func (store *Store) ApplyActivationResult(
	ctx context.Context,
	result contract.ActivationResult,
) (EdgeActivation, error) {
	var noEdge EdgeActivation
	if err := result.Validate(); err != nil {
		return noEdge, err
	}
	encoded, err := result.Encode()
	if err != nil {
		return noEdge, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noEdge, err
	}
	defer func() { _ = tx.Rollback() }()

	var edgeRef string
	var state EdgeActivationState
	var edgeNodeID string
	var ledgerEpoch string
	var activationID string
	var requestPayload []byte
	var storedResult []byte
	err = tx.QueryRowContext(ctx, `
		SELECT edge_ref, state, edge_node_id, ledger_epoch,
			COALESCE(activation_id, ''), request_json, result_json
		FROM edge_activations
		WHERE activation_id = ?
	`, result.ActivationID).Scan(
		&edgeRef, &state, &edgeNodeID, &ledgerEpoch,
		&activationID, &requestPayload, &storedResult,
	)
	if errors.Is(err, sql.ErrNoRows) {
		return noEdge, ErrActivationConflict
	}
	if err != nil {
		return noEdge, err
	}
	if state == EdgeActive && bytes.Equal(storedResult, encoded) {
		edge, err := loadEdgeByRefTx(ctx, tx, edgeRef)
		if err != nil {
			return noEdge, err
		}
		if err := tx.Commit(); err != nil {
			return noEdge, err
		}
		return edge, nil
	}
	request, decodeErr := contract.DecodeActivationRequest(requestPayload)
	exact := decodeErr == nil &&
		state == EdgeActivating &&
		activationID == result.ActivationID &&
		request.SiteID == result.SiteID &&
		request.EdgeNodeID == result.EdgeNodeID &&
		request.ExpectedLedgerEpoch == result.LedgerEpoch &&
		edgeNodeID == result.EdgeNodeID &&
		ledgerEpoch == result.LedgerEpoch
	if !exact {
		now := time.Now().UnixMilli()
		if _, updateErr := tx.ExecContext(ctx, `
			UPDATE edge_activations
			SET state = 'recovery_hold', result_json = ?,
				last_result_at = ?, revision = revision + 1, updated_at = ?
			WHERE edge_ref = ?
		`, encoded, now, now, edgeRef); updateErr != nil {
			return noEdge, updateErr
		}
		if commitErr := tx.Commit(); commitErr != nil {
			return noEdge, commitErr
		}
		return noEdge, ErrActivationConflict
	}

	now := time.Now().UnixMilli()
	if _, err := tx.ExecContext(ctx, `
		UPDATE edge_activations
		SET state = 'active', result_json = ?, last_result_at = ?,
			revision = revision + 1, updated_at = ?
		WHERE edge_ref = ? AND state = 'activating'
	`, encoded, now, now, edgeRef); err != nil {
		return noEdge, err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE activation_command_outbox
		SET completed_at = ?
		WHERE activation_id = ? AND completed_at IS NULL
	`, now, result.ActivationID); err != nil {
		return noEdge, err
	}
	edge, err := loadEdgeByRefTx(ctx, tx, edgeRef)
	if err != nil {
		return noEdge, err
	}
	if err := tx.Commit(); err != nil {
		return noEdge, err
	}
	return edge, nil
}

func (store *Store) ListPendingActivationCommands(
	ctx context.Context,
	limit int,
) ([]ActivationCommand, error) {
	if limit < 1 || limit > 1_000 {
		return nil, errors.New("activation command limit must be between 1 and 1000")
	}
	rows, err := store.db.QueryContext(ctx, `
		SELECT activation_id, topic, payload_json, attempts,
			last_attempt_at, created_at
		FROM activation_command_outbox
		WHERE completed_at IS NULL
		ORDER BY created_at, activation_id
		LIMIT ?
	`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	commands := make([]ActivationCommand, 0)
	for rows.Next() {
		var command ActivationCommand
		var payload []byte
		var lastAttempt sql.NullInt64
		if err := rows.Scan(
			&command.ActivationID, &command.Topic, &payload,
			&command.Attempts, &lastAttempt, &command.CreatedAt,
		); err != nil {
			return nil, err
		}
		command.PayloadJSON = json.RawMessage(payload)
		if lastAttempt.Valid {
			command.LastAttempt = &lastAttempt.Int64
		}
		commands = append(commands, command)
	}
	return commands, rows.Err()
}

func (store *Store) MarkActivationCommandAttempt(
	ctx context.Context,
	activationID string,
	at int64,
) error {
	if at < 0 {
		return errors.New("activation attempt timestamp must be non-negative")
	}
	_, err := store.db.ExecContext(ctx, `
		UPDATE activation_command_outbox
		SET attempts = attempts + 1, last_attempt_at = ?
		WHERE activation_id = ? AND completed_at IS NULL
	`, at, activationID)
	return err
}

func discoverEdgeTx(
	ctx context.Context,
	tx *sql.Tx,
	edgeNodeID string,
	ledgerEpoch string,
	now int64,
) error {
	var currentEpoch string
	var state EdgeActivationState
	err := tx.QueryRowContext(ctx, `
		SELECT ledger_epoch, state FROM edge_activations WHERE edge_node_id = ?
	`, edgeNodeID).Scan(&currentEpoch, &state)
	if errors.Is(err, sql.ErrNoRows) {
		edgeRef, idErr := randomPrefixedID("edge_")
		if idErr != nil {
			return idErr
		}
		_, err = tx.ExecContext(ctx, `
			INSERT INTO edge_activations(
				edge_ref, edge_node_id, ledger_epoch, state,
				last_descriptor_at, created_at, updated_at
			) VALUES(?, ?, ?, 'discovered', ?, ?, ?)
		`, edgeRef, edgeNodeID, ledgerEpoch, now, now, now)
		return err
	}
	if err != nil {
		return err
	}
	if currentEpoch != ledgerEpoch {
		_, err = tx.ExecContext(ctx, `
			UPDATE edge_activations
			SET state = 'recovery_hold', last_descriptor_at = ?,
				revision = revision + 1, updated_at = ?
			WHERE edge_node_id = ?
		`, now, now, edgeNodeID)
		return err
	}
	_, err = tx.ExecContext(ctx, `
		UPDATE edge_activations
		SET last_descriptor_at = ?, updated_at = ?
		WHERE edge_node_id = ?
	`, now, now, edgeNodeID)
	return err
}

func loadEdgeByRefTx(
	ctx context.Context,
	tx *sql.Tx,
	edgeRef string,
) (EdgeActivation, error) {
	row := tx.QueryRowContext(ctx, `
		SELECT edge_ref, edge_node_id, ledger_epoch, state,
			COALESCE(activation_id, ''), grant_revision,
			display_name, location, revision,
			last_descriptor_at, last_result_at
		FROM edge_activations
		WHERE edge_ref = ?
	`, edgeRef)
	return scanEdge(row)
}

type edgeScanner interface {
	Scan(dest ...any) error
}

func scanEdge(scanner edgeScanner) (EdgeActivation, error) {
	var edge EdgeActivation
	var lastDescriptor sql.NullInt64
	var lastResult sql.NullInt64
	if err := scanner.Scan(
		&edge.EdgeRef, &edge.EdgeNodeID, &edge.LedgerEpoch, &edge.State,
		&edge.ActivationID, &edge.GrantRevision,
		&edge.DisplayName, &edge.Location, &edge.Revision,
		&lastDescriptor, &lastResult,
	); err != nil {
		return edge, err
	}
	if lastDescriptor.Valid {
		edge.LastDescriptorAt = &lastDescriptor.Int64
	}
	if lastResult.Valid {
		edge.LastResultAt = &lastResult.Int64
	}
	return edge, nil
}

func randomPrefixedID(prefix string) (string, error) {
	random := make([]byte, 16)
	if _, err := rand.Read(random); err != nil {
		return "", err
	}
	return prefix + hex.EncodeToString(random), nil
}
