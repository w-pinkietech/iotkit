package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/google/uuid"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
)

const maxActiveSemanticRulesV3 = 16

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
