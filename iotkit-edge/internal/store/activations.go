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

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
)

var (
	ErrEdgeNodeNotActive  = errors.New("Edge Node is not active for IoTKit Edge custody")
	ErrActivationConflict = errors.New("Edge Node activation result conflict")
)

type EdgeNodeActivationState = edgeapp.EdgeNodeState

const (
	EdgeNodeDiscovered   = edgeapp.EdgeNodeDiscovered
	EdgeNodeActivating   = edgeapp.EdgeNodeActivating
	EdgeNodeActive       = edgeapp.EdgeNodeActive
	EdgeNodeRecoveryHold = edgeapp.EdgeNodeRecoveryHold
)

type EdgeNodeActivation = edgeapp.EdgeNode

type ActivationCommand struct {
	ActivationID string          `json:"activation_id"`
	Topic        string          `json:"topic"`
	PayloadJSON  json.RawMessage `json:"payload_json"`
	Attempts     int64           `json:"attempts"`
	LastAttempt  *int64          `json:"last_attempt_at,omitempty"`
	CreatedAt    int64           `json:"created_at"`
}

func (store *Store) ListEdgeNodes(ctx context.Context) ([]EdgeNodeActivation, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT edge_node_ref, edge_node_id, ledger_epoch, state,
			COALESCE(activation_id, ''), grant_revision,
			display_name, location, revision,
			last_descriptor_at, last_result_at,
			(SELECT COUNT(*) FROM descriptor_devices d
				WHERE d.edge_node_id = edge_node_activations.edge_node_id
					AND d.presence = 'current'),
			(SELECT COUNT(*) FROM descriptor_signals s
				WHERE s.edge_node_id = edge_node_activations.edge_node_id
					AND s.presence = 'current')
		FROM edge_node_activations
		ORDER BY created_at, edge_node_ref
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	edgeNodes := make([]EdgeNodeActivation, 0)
	for rows.Next() {
		edgeNode, err := scanEdgeNode(rows)
		if err != nil {
			return nil, err
		}
		edgeNodes = append(edgeNodes, edgeNode)
	}
	return edgeNodes, rows.Err()
}

func (store *Store) RequestEdgeNodeActivation(
	ctx context.Context,
	actor edgeapp.Actor,
	edgeNodeRef string,
	precondition edgeapp.RevisionPrecondition,
) (EdgeNodeActivation, error) {
	var noEdgeNode EdgeNodeActivation
	if err := actor.Validate(); err != nil {
		return noEdgeNode, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noEdgeNode, err
	}
	defer func() { _ = tx.Rollback() }()

	edgeNode, err := loadEdgeNodeByRefTx(ctx, tx, edgeNodeRef)
	if errors.Is(err, sql.ErrNoRows) {
		return noEdgeNode, edgeapp.ErrNotFound
	}
	if err != nil {
		return noEdgeNode, err
	}
	if edgeNode.State == EdgeNodeActivating || edgeNode.State == EdgeNodeActive {
		return edgeNode, nil
	}
	if edgeNode.State != EdgeNodeDiscovered {
		return noEdgeNode, ErrActivationConflict
	}
	if err := checkRevisionPrecondition(precondition, true, edgeNode.Revision); err != nil {
		return noEdgeNode, err
	}

	var edgeID string
	if err := tx.QueryRowContext(ctx, `
		SELECT edge_id FROM edge_meta WHERE singleton = 1
	`).Scan(&edgeID); err != nil {
		return noEdgeNode, err
	}
	activationID, err := randomPrefixedID("act-")
	if err != nil {
		return noEdgeNode, err
	}
	now := time.Now().UnixMilli()
	request := contract.ActivationRequest{
		SchemaVersion:       contract.SchemaVersion,
		ActivationID:        activationID,
		EdgeID:              edgeID,
		EdgeNodeID:          edgeNode.EdgeNodeID,
		ExpectedLedgerEpoch: edgeNode.LedgerEpoch,
		GrantRevision:       1,
		IssuedAt:            now,
	}
	payload, err := request.Encode()
	if err != nil {
		return noEdgeNode, err
	}
	topic := fmt.Sprintf(
		"iotkit/v1/edge-nodes/%s/activation/request",
		edgeNode.EdgeNodeID,
	)
	if _, err := tx.ExecContext(ctx, `
		UPDATE edge_node_activations
		SET state = 'activating', activation_id = ?, grant_revision = 1,
			request_json = ?, result_json = NULL,
			revision = revision + 1, updated_at = ?
		WHERE edge_node_ref = ? AND state = 'discovered'
	`, activationID, payload, now, edgeNodeRef); err != nil {
		return noEdgeNode, err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO activation_command_outbox(
			activation_id, topic, payload_json, created_at
		) VALUES(?, ?, ?, ?)
	`, activationID, topic, payload, now); err != nil {
		return noEdgeNode, err
	}
	summary, err := json.Marshal(struct {
		ActivationID string `json:"activation_id"`
		Revision     int64  `json:"revision"`
	}{activationID, edgeNode.Revision + 1})
	if err != nil {
		return noEdgeNode, err
	}
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt:  now,
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "edge_node.activation.request",
		ResourceRef: edgeNodeRef,
		Outcome:     auditOutcomeSuccess,
		Summary:     summary,
	}); err != nil {
		return noEdgeNode, err
	}
	edgeNode, err = loadEdgeNodeByRefTx(ctx, tx, edgeNodeRef)
	if err != nil {
		return noEdgeNode, err
	}
	if err := tx.Commit(); err != nil {
		return noEdgeNode, err
	}
	return edgeNode, nil
}

func (store *Store) ApplyActivationResult(
	ctx context.Context,
	result contract.ActivationResult,
) (EdgeNodeActivation, error) {
	var noEdgeNode EdgeNodeActivation
	if err := result.Validate(); err != nil {
		return noEdgeNode, err
	}
	encoded, err := result.Encode()
	if err != nil {
		return noEdgeNode, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noEdgeNode, err
	}
	defer func() { _ = tx.Rollback() }()

	var edgeNodeRef string
	var state EdgeNodeActivationState
	var edgeNodeID string
	var ledgerEpoch string
	var activationID string
	var requestPayload []byte
	var storedResult []byte
	err = tx.QueryRowContext(ctx, `
		SELECT edge_node_ref, state, edge_node_id, ledger_epoch,
			COALESCE(activation_id, ''), request_json, result_json
		FROM edge_node_activations
		WHERE activation_id = ?
	`, result.ActivationID).Scan(
		&edgeNodeRef, &state, &edgeNodeID, &ledgerEpoch,
		&activationID, &requestPayload, &storedResult,
	)
	if errors.Is(err, sql.ErrNoRows) {
		return noEdgeNode, ErrActivationConflict
	}
	if err != nil {
		return noEdgeNode, err
	}
	if state == EdgeNodeActive && bytes.Equal(storedResult, encoded) {
		edgeNode, err := loadEdgeNodeByRefTx(ctx, tx, edgeNodeRef)
		if err != nil {
			return noEdgeNode, err
		}
		if err := tx.Commit(); err != nil {
			return noEdgeNode, err
		}
		return edgeNode, nil
	}
	request, decodeErr := contract.DecodeActivationRequest(requestPayload)
	exact := decodeErr == nil &&
		state == EdgeNodeActivating &&
		activationID == result.ActivationID &&
		request.EdgeID == result.EdgeID &&
		request.EdgeNodeID == result.EdgeNodeID &&
		request.ExpectedLedgerEpoch == result.LedgerEpoch &&
		edgeNodeID == result.EdgeNodeID &&
		ledgerEpoch == result.LedgerEpoch
	if !exact {
		now := time.Now().UnixMilli()
		if _, updateErr := tx.ExecContext(ctx, `
			UPDATE edge_node_activations
			SET state = 'recovery_hold', result_json = ?,
				last_result_at = ?, revision = revision + 1, updated_at = ?
			WHERE edge_node_ref = ?
		`, encoded, now, now, edgeNodeRef); updateErr != nil {
			return noEdgeNode, updateErr
		}
		if commitErr := tx.Commit(); commitErr != nil {
			return noEdgeNode, commitErr
		}
		return noEdgeNode, ErrActivationConflict
	}

	now := time.Now().UnixMilli()
	if _, err := tx.ExecContext(ctx, `
		UPDATE edge_node_activations
		SET state = 'active', result_json = ?, last_result_at = ?,
			revision = revision + 1, updated_at = ?
		WHERE edge_node_ref = ? AND state = 'activating'
	`, encoded, now, now, edgeNodeRef); err != nil {
		return noEdgeNode, err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE activation_command_outbox
		SET completed_at = ?
		WHERE activation_id = ? AND completed_at IS NULL
	`, now, result.ActivationID); err != nil {
		return noEdgeNode, err
	}
	edgeNode, err := loadEdgeNodeByRefTx(ctx, tx, edgeNodeRef)
	if err != nil {
		return noEdgeNode, err
	}
	if err := tx.Commit(); err != nil {
		return noEdgeNode, err
	}
	return edgeNode, nil
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

func discoverEdgeNodeTx(
	ctx context.Context,
	tx *sql.Tx,
	edgeNodeID string,
	ledgerEpoch string,
	now int64,
) error {
	var currentEpoch string
	var state EdgeNodeActivationState
	err := tx.QueryRowContext(ctx, `
		SELECT ledger_epoch, state FROM edge_node_activations WHERE edge_node_id = ?
	`, edgeNodeID).Scan(&currentEpoch, &state)
	if errors.Is(err, sql.ErrNoRows) {
		edgeNodeRef, idErr := randomPrefixedID("edge_node_")
		if idErr != nil {
			return idErr
		}
		_, err = tx.ExecContext(ctx, `
			INSERT INTO edge_node_activations(
				edge_node_ref, edge_node_id, ledger_epoch, state,
				last_descriptor_at, created_at, updated_at
			) VALUES(?, ?, ?, 'discovered', ?, ?, ?)
		`, edgeNodeRef, edgeNodeID, ledgerEpoch, now, now, now)
		return err
	}
	if err != nil {
		return err
	}
	if currentEpoch != ledgerEpoch {
		_, err = tx.ExecContext(ctx, `
			UPDATE edge_node_activations
			SET state = 'recovery_hold', last_descriptor_at = ?,
				revision = revision + 1, updated_at = ?
			WHERE edge_node_id = ?
		`, now, now, edgeNodeID)
		return err
	}
	_, err = tx.ExecContext(ctx, `
		UPDATE edge_node_activations
		SET last_descriptor_at = ?, updated_at = ?
		WHERE edge_node_id = ?
	`, now, now, edgeNodeID)
	return err
}

func loadEdgeNodeByRefTx(
	ctx context.Context,
	tx *sql.Tx,
	edgeNodeRef string,
) (EdgeNodeActivation, error) {
	row := tx.QueryRowContext(ctx, `
		SELECT edge_node_ref, edge_node_id, ledger_epoch, state,
			COALESCE(activation_id, ''), grant_revision,
			display_name, location, revision,
			last_descriptor_at, last_result_at,
			(SELECT COUNT(*) FROM descriptor_devices d
				WHERE d.edge_node_id = edge_node_activations.edge_node_id
					AND d.presence = 'current'),
			(SELECT COUNT(*) FROM descriptor_signals s
				WHERE s.edge_node_id = edge_node_activations.edge_node_id
					AND s.presence = 'current')
		FROM edge_node_activations
		WHERE edge_node_ref = ?
	`, edgeNodeRef)
	return scanEdgeNode(row)
}

type edgeNodeScanner interface {
	Scan(dest ...any) error
}

func scanEdgeNode(scanner edgeNodeScanner) (EdgeNodeActivation, error) {
	var edgeNode EdgeNodeActivation
	var lastDescriptor sql.NullInt64
	var lastResult sql.NullInt64
	if err := scanner.Scan(
		&edgeNode.EdgeNodeRef, &edgeNode.EdgeNodeID, &edgeNode.LedgerEpoch, &edgeNode.State,
		&edgeNode.ActivationID, &edgeNode.GrantRevision,
		&edgeNode.DisplayName, &edgeNode.Location, &edgeNode.Revision,
		&lastDescriptor, &lastResult, &edgeNode.DeviceCount, &edgeNode.SensorCount,
	); err != nil {
		return edgeNode, err
	}
	if lastDescriptor.Valid {
		edgeNode.LastDescriptorAt = &lastDescriptor.Int64
	}
	if lastResult.Valid {
		edgeNode.LastResultAt = &lastResult.Int64
	}
	return edgeNode, nil
}

func randomPrefixedID(prefix string) (string, error) {
	random := make([]byte, 16)
	if _, err := rand.Read(random); err != nil {
		return "", err
	}
	return prefix + hex.EncodeToString(random), nil
}
