package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/google/uuid"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
)

const maxActiveSemanticRulesV3 = 16

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

func validateSemanticRuleName(name string) error {
	if name != strings.TrimSpace(name) || utf8.RuneCountInString(name) < 1 ||
		utf8.RuneCountInString(name) > 128 ||
		strings.IndexFunc(name, unicode.IsControl) >= 0 {
		return errors.New("semantic rule display name must be 1 to 128 characters without surrounding whitespace")
	}
	return nil
}

func (store *Store) GetSemanticConfiguration(
	ctx context.Context,
	signalRef string,
) (semantics.Configuration, error) {
	var configuration semantics.Configuration
	configuration.SignalRef = signalRef
	if err := store.db.QueryRowContext(ctx, `
		SELECT config.revision, calibration.revision,
			calibration.scale, calibration."offset", calibration.created_at
		FROM semantic_signal_configs_v3 AS config
		JOIN signal_calibration_revisions_v3 AS calibration
			ON calibration.signal_ref = config.signal_ref
			AND calibration.active = 1
		WHERE config.signal_ref = ?
	`, signalRef).Scan(
		&configuration.Revision,
		&configuration.Calibration.Revision,
		&configuration.Calibration.Scale,
		&configuration.Calibration.Offset,
		&configuration.Calibration.CreatedAt,
	); errors.Is(err, sql.ErrNoRows) {
		var exists bool
		if lookupErr := store.db.QueryRowContext(ctx, `
			SELECT EXISTS(SELECT 1 FROM edge_signals WHERE signal_ref = ?)
		`, signalRef).Scan(&exists); lookupErr != nil {
			return semantics.Configuration{}, lookupErr
		}
		if !exists {
			return semantics.Configuration{}, edgeapp.ErrNotFound
		}
		return semantics.Configuration{}, errors.New("semantic signal configuration is not initialized")
	} else if err != nil {
		return semantics.Configuration{}, err
	}
	configuration.Calibration.SignalRef = signalRef
	rules, err := store.listActiveSemanticRulesV3(ctx, signalRef)
	if err != nil {
		return semantics.Configuration{}, err
	}
	configuration.Rules = rules
	return configuration, nil
}

func (store *Store) listActiveSemanticRulesV3(
	ctx context.Context,
	signalRef string,
) ([]semantics.Rule, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT rule.rule_id, rule.signal_ref, rule.display_name, rule.kind,
			revision.series_id, revision.revision, revision.spec_json,
			rule.created_at, rule.retired_at
		FROM semantic_rules_v3 AS rule
		JOIN semantic_rule_revisions_v3 AS revision
			ON revision.rule_id = rule.rule_id AND revision.active = 1
		WHERE rule.signal_ref = ? AND rule.retired_at IS NULL
		ORDER BY rule.display_order
	`, signalRef)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	rules := make([]semantics.Rule, 0)
	for rows.Next() {
		rule, err := scanSemanticRuleV3(rows)
		if err != nil {
			return nil, err
		}
		rules = append(rules, rule)
	}
	return rules, rows.Err()
}

func scanSemanticRuleV3(row rowScanner) (semantics.Rule, error) {
	var rule semantics.Rule
	var specJSON []byte
	var retiredAt sql.NullInt64
	if err := row.Scan(
		&rule.ID,
		&rule.SignalRef,
		&rule.DisplayName,
		&rule.Kind,
		&rule.SeriesID,
		&rule.Revision,
		&specJSON,
		&rule.CreatedAt,
		&retiredAt,
	); err != nil {
		return semantics.Rule{}, err
	}
	if err := json.Unmarshal(specJSON, &rule.RuleSpec); err != nil {
		return semantics.Rule{}, err
	}
	if rule.RuleSpec.Kind != rule.Kind {
		return semantics.Rule{}, errors.New("semantic rule kind conflicts with revision")
	}
	rule.Active = !retiredAt.Valid
	if retiredAt.Valid {
		rule.RetiredAt = &retiredAt.Int64
	}
	return rule, nil
}

func (store *Store) CreateSemanticRule(
	ctx context.Context,
	actor edgeapp.Actor,
	signalRef string,
	displayName string,
	spec semantics.RuleSpec,
	precondition edgeapp.RevisionPrecondition,
) (semantics.Rule, error) {
	var noRule semantics.Rule
	if err := actor.Validate(); err != nil {
		return noRule, err
	}
	if err := validateSemanticRuleName(displayName); err != nil {
		return noRule, err
	}
	if err := spec.Validate(); err != nil {
		return noRule, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noRule, err
	}
	defer func() { _ = tx.Rollback() }()

	edgeNodeID, configRevision, err := semanticSignalForUpdateV3(
		ctx, tx, signalRef, precondition,
	)
	if err != nil {
		return noRule, err
	}
	var activeCount int
	if err := tx.QueryRowContext(ctx, `
		SELECT count(*) FROM semantic_rules_v3
		WHERE signal_ref = ? AND retired_at IS NULL
	`, signalRef).Scan(&activeCount); err != nil {
		return noRule, err
	}
	if activeCount >= maxActiveSemanticRulesV3 {
		return noRule, errors.New("semantic signal cannot have more than 16 active rules")
	}
	ruleID, err := newResourceRef("rule_")
	if err != nil {
		return noRule, err
	}
	now := time.Now().UnixMilli()
	rule := semantics.Rule{
		ID: ruleID, SignalRef: signalRef, DisplayName: displayName,
		SeriesID: uuid.NewString(), Revision: 1, RuleSpec: spec,
		Active: true, CreatedAt: now,
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_rules_v3(
			rule_id, signal_ref, display_name, kind, series_id,
			display_order, created_at
		) VALUES (?, ?, ?, ?, ?,
			(SELECT COALESCE(MAX(display_order), 0) + 1
				FROM semantic_rules_v3 WHERE signal_ref = ?),
			?
		)
	`, rule.ID, rule.SignalRef, rule.DisplayName, rule.Kind,
		rule.SeriesID, rule.SignalRef, rule.CreatedAt); err != nil {
		return noRule, err
	}
	if err := insertSemanticRuleRevisionV3(
		ctx, tx, rule, edgeNodeID,
	); err != nil {
		return noRule, err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_rule_runtime_v3(
			rule_id, initialized, detector_active, counter,
			pending, pending_active, pending_since,
			applied_rule_revision, applied_calibration_revision,
			applied_ledger_epoch, next_sequence
		) VALUES (?, 0, 0, 0, 0, 0, 0, 0, 0, '', 1)
	`, rule.ID); err != nil {
		return noRule, err
	}
	if err := autoBindSemanticRuleTx(ctx, tx, rule, edgeNodeID); err != nil {
		return noRule, err
	}
	if err := bumpSemanticConfigV3(ctx, tx, signalRef, configRevision); err != nil {
		return noRule, err
	}
	if err := auditSemanticRuleV3(ctx, tx, actor, "semantic_rule.create", rule); err != nil {
		return noRule, err
	}
	if err := tx.Commit(); err != nil {
		return noRule, err
	}
	return rule, nil
}

func (store *Store) UpdateSemanticRule(
	ctx context.Context,
	actor edgeapp.Actor,
	ruleID string,
	displayName string,
	spec semantics.RuleSpec,
	precondition edgeapp.RevisionPrecondition,
) (semantics.Rule, error) {
	var noRule semantics.Rule
	if err := actor.Validate(); err != nil {
		return noRule, err
	}
	if err := validateSemanticRuleName(displayName); err != nil {
		return noRule, err
	}
	if err := spec.Validate(); err != nil {
		return noRule, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noRule, err
	}
	defer func() { _ = tx.Rollback() }()

	current, edgeNodeID, configRevision, err := activeSemanticRuleForUpdateV3(
		ctx, tx, ruleID,
	)
	if err != nil {
		return noRule, err
	}
	if err := checkRevisionPrecondition(precondition, true, current.Revision); err != nil {
		return noRule, err
	}
	if spec.Kind != current.Kind {
		return noRule, errors.New("semantic rule kind is immutable")
	}
	if err := endSemanticRuleRevisionV3(ctx, tx, current, edgeNodeID); err != nil {
		return noRule, err
	}
	updated := current
	updated.DisplayName = displayName
	updated.Revision++
	updated.RuleSpec = spec
	if spec != current.RuleSpec {
		updated.SeriesID = uuid.NewString()
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE semantic_rules_v3 SET display_name = ?, series_id = ?
		WHERE rule_id = ? AND retired_at IS NULL
	`, displayName, updated.SeriesID, ruleID); err != nil {
		return noRule, err
	}
	if err := insertSemanticRuleRevisionV3(
		ctx, tx, updated, edgeNodeID,
	); err != nil {
		return noRule, err
	}
	if err := bumpSemanticConfigV3(
		ctx, tx, current.SignalRef, configRevision,
	); err != nil {
		return noRule, err
	}
	if err := auditSemanticRuleV3(ctx, tx, actor, "semantic_rule.update", updated); err != nil {
		return noRule, err
	}
	if err := tx.Commit(); err != nil {
		return noRule, err
	}
	return updated, nil
}

func (store *Store) UpdateSignalCalibration(
	ctx context.Context,
	actor edgeapp.Actor,
	signalRef string,
	scale float64,
	offset float64,
	precondition edgeapp.RevisionPrecondition,
) (semantics.Configuration, error) {
	var noConfiguration semantics.Configuration
	if err := actor.Validate(); err != nil {
		return noConfiguration, err
	}
	calibration := semantics.Calibration{
		SignalRef: signalRef, Scale: scale, Offset: offset,
	}
	if err := calibration.Validate(); err != nil {
		return noConfiguration, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noConfiguration, err
	}
	defer func() { _ = tx.Rollback() }()
	edgeNodeID, configRevision, err := semanticSignalForUpdateV3(
		ctx, tx, signalRef, precondition,
	)
	if err != nil {
		return noConfiguration, err
	}
	var currentRevision int64
	if err := tx.QueryRowContext(ctx, `
		SELECT revision FROM signal_calibration_revisions_v3
		WHERE signal_ref = ? AND active = 1
	`, signalRef).Scan(&currentRevision); err != nil {
		return noConfiguration, err
	}
	calibration.Revision = currentRevision + 1
	calibration.CreatedAt = time.Now().UnixMilli()
	if _, err := tx.ExecContext(ctx, `
		INSERT OR IGNORE INTO signal_calibration_starts_v3(
			signal_ref, calibration_revision, ledger_epoch, start_after_pub_seq
		)
		SELECT ?, ?, ledger_epoch, 0
		FROM accepted_cursors WHERE edge_node_id = ?
	`, signalRef, currentRevision, edgeNodeID); err != nil {
		return noConfiguration, err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE signal_calibration_revisions_v3 SET active = 0
		WHERE signal_ref = ? AND revision = ? AND active = 1
	`, signalRef, currentRevision); err != nil {
		return noConfiguration, err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO signal_calibration_revisions_v3(
			signal_ref, revision, scale, "offset", active, created_at
		) VALUES (?, ?, ?, ?, 1, ?)
	`, signalRef, calibration.Revision, calibration.Scale,
		calibration.Offset, calibration.CreatedAt); err != nil {
		return noConfiguration, err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO signal_calibration_starts_v3(
			signal_ref, calibration_revision, ledger_epoch, start_after_pub_seq
		)
		SELECT ?, ?, ledger_epoch, accepted_through
		FROM accepted_cursors WHERE edge_node_id = ?
	`, signalRef, calibration.Revision, edgeNodeID); err != nil {
		return noConfiguration, err
	}
	if err := rotateSemanticSeriesForCalibrationTx(
		ctx, tx, signalRef, edgeNodeID,
	); err != nil {
		return noConfiguration, err
	}
	if err := bumpSemanticConfigV3(
		ctx, tx, signalRef, configRevision,
	); err != nil {
		return noConfiguration, err
	}
	summary, _ := json.Marshal(struct {
		Revision int64   `json:"revision"`
		Scale    float64 `json:"scale"`
		Offset   float64 `json:"offset"`
	}{
		Revision: calibration.Revision,
		Scale:    calibration.Scale,
		Offset:   calibration.Offset,
	})
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt: calibration.CreatedAt, ActorClass: actor.Class,
		ActorRef: actor.Ref, Operation: "semantic_calibration.update",
		ResourceRef: signalRef, Outcome: auditOutcomeSuccess, Summary: summary,
	}); err != nil {
		return noConfiguration, err
	}
	if err := tx.Commit(); err != nil {
		return noConfiguration, err
	}
	return store.GetSemanticConfiguration(ctx, signalRef)
}

func rotateSemanticSeriesForCalibrationTx(
	ctx context.Context,
	tx *sqlTx,
	signalRef string,
	edgeNodeID string,
) error {
	rows, err := tx.QueryContext(ctx, `
		SELECT rule.rule_id, rule.signal_ref, rule.display_name, rule.kind,
			revision.series_id, revision.revision, revision.spec_json,
			rule.created_at
		FROM semantic_rules_v3 AS rule
		JOIN semantic_rule_revisions_v3 AS revision
			ON revision.rule_id = rule.rule_id AND revision.active = 1
		WHERE rule.signal_ref = ? AND rule.retired_at IS NULL
		ORDER BY rule.display_order
	`, signalRef)
	if err != nil {
		return err
	}
	rules := make([]semantics.Rule, 0)
	for rows.Next() {
		var rule semantics.Rule
		var specJSON []byte
		if err := rows.Scan(
			&rule.ID, &rule.SignalRef, &rule.DisplayName, &rule.Kind,
			&rule.SeriesID, &rule.Revision, &specJSON, &rule.CreatedAt,
		); err != nil {
			_ = rows.Close()
			return err
		}
		if err := json.Unmarshal(specJSON, &rule.RuleSpec); err != nil {
			_ = rows.Close()
			return err
		}
		rule.Active = true
		rules = append(rules, rule)
	}
	if err := rows.Close(); err != nil {
		return err
	}
	for _, current := range rules {
		if err := endSemanticRuleRevisionV3(
			ctx, tx, current, edgeNodeID,
		); err != nil {
			return err
		}
		updated := current
		updated.Revision++
		updated.SeriesID = uuid.NewString()
		if _, err := tx.ExecContext(ctx, `
			UPDATE semantic_rules_v3 SET series_id = ?
			WHERE rule_id = ? AND retired_at IS NULL
		`, updated.SeriesID, updated.ID); err != nil {
			return err
		}
		if err := insertSemanticRuleRevisionV3(
			ctx, tx, updated, edgeNodeID,
		); err != nil {
			return err
		}
	}
	return nil
}

func (store *Store) RetireSemanticRule(
	ctx context.Context,
	actor edgeapp.Actor,
	ruleID string,
	precondition edgeapp.RevisionPrecondition,
) (semantics.Rule, error) {
	var noRule semantics.Rule
	if err := actor.Validate(); err != nil {
		return noRule, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noRule, err
	}
	defer func() { _ = tx.Rollback() }()
	current, edgeNodeID, configRevision, err := activeSemanticRuleForUpdateV3(
		ctx, tx, ruleID,
	)
	if err != nil {
		return noRule, err
	}
	if err := checkRevisionPrecondition(
		precondition, true, current.Revision,
	); err != nil {
		return noRule, err
	}
	if err := endSemanticRuleRevisionV3(
		ctx, tx, current, edgeNodeID,
	); err != nil {
		return noRule, err
	}
	if err := drainOutputBindingsForRuleTx(
		ctx, tx, current.ID, edgeNodeID,
	); err != nil {
		return noRule, err
	}
	now := time.Now().UnixMilli()
	if _, err := tx.ExecContext(ctx, `
		UPDATE semantic_rules_v3 SET retired_at = ?
		WHERE rule_id = ? AND retired_at IS NULL
	`, now, ruleID); err != nil {
		return noRule, err
	}
	if err := bumpSemanticConfigV3(
		ctx, tx, current.SignalRef, configRevision,
	); err != nil {
		return noRule, err
	}
	current.Active = false
	current.RetiredAt = &now
	if err := auditSemanticRuleV3(
		ctx, tx, actor, "semantic_rule.retire", current,
	); err != nil {
		return noRule, err
	}
	if err := tx.Commit(); err != nil {
		return noRule, err
	}
	return current, nil
}

func semanticSignalForUpdateV3(
	ctx context.Context,
	tx *sqlTx,
	signalRef string,
	precondition edgeapp.RevisionPrecondition,
) (string, int64, error) {
	var edgeNodeID string
	var revision int64
	if err := tx.QueryRowContext(ctx, `
		SELECT signal.edge_node_id, config.revision
		FROM edge_signals AS signal
		JOIN semantic_signal_configs_v3 AS config
			ON config.signal_ref = signal.signal_ref
		WHERE signal.signal_ref = ?
	`, signalRef).Scan(&edgeNodeID, &revision); errors.Is(err, sql.ErrNoRows) {
		return "", 0, edgeapp.ErrNotFound
	} else if err != nil {
		return "", 0, err
	}
	if err := checkRevisionPrecondition(precondition, true, revision); err != nil {
		return "", 0, err
	}
	return edgeNodeID, revision, nil
}

func activeSemanticRuleForUpdateV3(
	ctx context.Context,
	tx *sqlTx,
	ruleID string,
) (semantics.Rule, string, int64, error) {
	var rule semantics.Rule
	var specJSON []byte
	var edgeNodeID string
	var configRevision int64
	err := tx.QueryRowContext(ctx, `
		SELECT rule.rule_id, rule.signal_ref, rule.display_name, rule.kind,
			revision.series_id, revision.revision, revision.spec_json,
			rule.created_at, rule.retired_at,
			signal.edge_node_id, config.revision
		FROM semantic_rules_v3 AS rule
		JOIN semantic_rule_revisions_v3 AS revision
			ON revision.rule_id = rule.rule_id AND revision.active = 1
		JOIN edge_signals AS signal ON signal.signal_ref = rule.signal_ref
		JOIN semantic_signal_configs_v3 AS config
			ON config.signal_ref = rule.signal_ref
		WHERE rule.rule_id = ? AND rule.retired_at IS NULL
	`, ruleID).Scan(
		&rule.ID, &rule.SignalRef, &rule.DisplayName, &rule.Kind,
		&rule.SeriesID, &rule.Revision, &specJSON, &rule.CreatedAt,
		new(sql.NullInt64), &edgeNodeID, &configRevision,
	)
	if errors.Is(err, sql.ErrNoRows) {
		return semantics.Rule{}, "", 0, edgeapp.ErrNotFound
	}
	if err != nil {
		return semantics.Rule{}, "", 0, err
	}
	if err := json.Unmarshal(specJSON, &rule.RuleSpec); err != nil {
		return semantics.Rule{}, "", 0, err
	}
	rule.Active = true
	return rule, edgeNodeID, configRevision, nil
}

func insertSemanticRuleRevisionV3(
	ctx context.Context,
	tx *sqlTx,
	rule semantics.Rule,
	edgeNodeID string,
) error {
	specJSON, err := json.Marshal(rule.RuleSpec)
	if err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_rule_revisions_v3(
			rule_id, revision, spec_json, active, created_at, series_id
		) VALUES (?, ?, ?, 1, ?, ?)
	`, rule.ID, rule.Revision, specJSON, time.Now().UnixMilli(),
		rule.SeriesID); err != nil {
		return err
	}
	_, err = tx.ExecContext(ctx, `
		INSERT INTO semantic_rule_starts_v3(
			rule_id, rule_revision, ledger_epoch, start_after_pub_seq
		)
		SELECT ?, ?, ledger_epoch, accepted_through
		FROM accepted_cursors WHERE edge_node_id = ?
	`, rule.ID, rule.Revision, edgeNodeID)
	return err
}

func endSemanticRuleRevisionV3(
	ctx context.Context,
	tx *sqlTx,
	rule semantics.Rule,
	edgeNodeID string,
) error {
	if _, err := tx.ExecContext(ctx, `
		INSERT OR IGNORE INTO semantic_rule_starts_v3(
			rule_id, rule_revision, ledger_epoch, start_after_pub_seq
		)
		SELECT ?, ?, ledger_epoch, 0
		FROM accepted_cursors WHERE edge_node_id = ?
	`, rule.ID, rule.Revision, edgeNodeID); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_rule_ends_v3(
			rule_id, rule_revision, ledger_epoch, end_at_pub_seq
		)
		SELECT ?, ?, ledger_epoch, accepted_through
		FROM accepted_cursors WHERE edge_node_id = ?
	`, rule.ID, rule.Revision, edgeNodeID); err != nil {
		return err
	}
	_, err := tx.ExecContext(ctx, `
		UPDATE semantic_rule_revisions_v3 SET active = 0
		WHERE rule_id = ? AND revision = ? AND active = 1
	`, rule.ID, rule.Revision)
	return err
}

func bumpSemanticConfigV3(
	ctx context.Context,
	tx *sqlTx,
	signalRef string,
	currentRevision int64,
) error {
	result, err := tx.ExecContext(ctx, `
		UPDATE semantic_signal_configs_v3 SET revision = revision + 1
		WHERE signal_ref = ? AND revision = ?
	`, signalRef, currentRevision)
	if err != nil {
		return err
	}
	changed, err := result.RowsAffected()
	if err != nil {
		return err
	}
	if changed != 1 {
		return edgeapp.ErrRevisionMismatch
	}
	return nil
}

func auditSemanticRuleV3(
	ctx context.Context,
	tx *sqlTx,
	actor edgeapp.Actor,
	operation string,
	rule semantics.Rule,
) error {
	summary, _ := json.Marshal(struct {
		SignalRef   string         `json:"signal_ref"`
		DisplayName string         `json:"display_name"`
		Kind        semantics.Kind `json:"kind"`
		Revision    int64          `json:"revision"`
	}{
		SignalRef: rule.SignalRef, DisplayName: rule.DisplayName,
		Kind: rule.Kind, Revision: rule.Revision,
	})
	return insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt: time.Now().UnixMilli(), ActorClass: actor.Class,
		ActorRef: actor.Ref, Operation: operation, ResourceRef: rule.ID,
		Outcome: auditOutcomeSuccess, Summary: summary,
	})
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
