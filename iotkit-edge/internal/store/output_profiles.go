package store

import (
	"context"
	"crypto/rand"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
)

var edgeIDPattern = regexp.MustCompile(`^edge-[0-9a-f]{32}$`)
var outputRandomRead = rand.Read

func (store *Store) validateEdgeIdentity(ctx context.Context) error {
	var count int
	var edgeID string
	if err := store.db.QueryRowContext(ctx, `
		SELECT count(*), COALESCE(MAX(edge_id), '') FROM edge_meta
	`).Scan(&count, &edgeID); err != nil {
		return err
	}
	if count != 1 || !edgeIDPattern.MatchString(edgeID) {
		return errors.New("Edge identity metadata is missing or invalid")
	}
	return nil
}

func (store *Store) ActivateExportProfile(
	ctx context.Context,
	actor edgeapp.Actor,
	displayName string,
	adapterID string,
) (edgeapp.ExportProfile, error) {
	var noProfile edgeapp.ExportProfile
	if err := authorizeOutputMutation(actor); err != nil {
		return noProfile, err
	}
	if displayName == "" || len(displayName) > 128 {
		return noProfile, errors.New("external destination name is required")
	}
	_, descriptor, err := resolveOutputAdapter(adapterID)
	if err != nil {
		return noProfile, err
	}
	switch adapterID {
	case "iotkit.mqtt-json.v1", "yokakit.mqtt.v1":
	default:
		return noProfile, outputadapter.ErrInvalidConfiguration
	}
	profileID, err := newResourceRef("exp_")
	if err != nil {
		return noProfile, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noProfile, err
	}
	defer func() { _ = tx.Rollback() }()
	sourceID, err := validatedEdgeIDTx(ctx, tx)
	if err != nil {
		return noProfile, err
	}
	now := time.Now().UnixMilli()
	profileState := edgeapp.ExportProfileActive
	if adapterID == "yokakit.mqtt.v1" {
		profileState = edgeapp.ExportProfilePreparing
	}
	profile := edgeapp.ExportProfile{
		ProfileID:            profileID,
		DisplayName:          displayName,
		AdapterID:            adapterID,
		AdapterSchemaVersion: descriptor.ConfigSchemaVersion,
		State:                profileState,
		AutoBindFutureRules:  true,
		Revision:             1,
		CreatedAt:            now,
	}
	profileConfig := json.RawMessage(`{"schema_version":1}`)
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO export_profiles(
			profile_id, display_name, adapter_id, adapter_schema_version,
			profile_config_json, state, auto_bind_future_rules,
			revision, created_at
		) VALUES (?, ?, ?, ?, ?, ?, 1, 1, ?)
	`, profile.ProfileID, profile.DisplayName, profile.AdapterID,
		profile.AdapterSchemaVersion, []byte(profileConfig), profile.State,
		profile.CreatedAt); err != nil {
		return noProfile, err
	}
	rows, err := tx.QueryContext(ctx, `
		SELECT rule.rule_id, rule.signal_ref, rule.display_name, rule.kind,
			signal.edge_node_id
		FROM semantic_rules_v3 AS rule
		JOIN edge_signals AS signal ON signal.signal_ref = rule.signal_ref
		WHERE rule.retired_at IS NULL
		ORDER BY rule.created_at, rule.rule_id
	`)
	if err != nil {
		return noProfile, err
	}
	type ruleDescriptor struct {
		id, signalRef, displayName, edgeNodeID string
		kind                                   semantics.Kind
	}
	var rules []ruleDescriptor
	for rows.Next() {
		var rule ruleDescriptor
		if err := rows.Scan(
			&rule.id, &rule.signalRef, &rule.displayName, &rule.kind,
			&rule.edgeNodeID,
		); err != nil {
			_ = rows.Close()
			return noProfile, err
		}
		rules = append(rules, rule)
	}
	if err := rows.Close(); err != nil {
		return noProfile, err
	}
	for _, rule := range rules {
		binding, err := createProfileBindingTx(
			ctx, tx, profile, sourceID, rule.id, rule.signalRef,
			rule.displayName, rule.kind, rule.edgeNodeID,
		)
		if err != nil {
			return noProfile, err
		}
		profile.Bindings = append(profile.Bindings, binding)
	}
	summary, _ := json.Marshal(map[string]any{
		"adapter_id":             adapterID,
		"rule_count":             len(profile.Bindings),
		"auto_bind_future_rules": true,
	})
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt:  now,
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "export_profile.activate",
		ResourceRef: profile.ProfileID,
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

func (store *Store) ListExportProfiles(
	ctx context.Context,
) ([]edgeapp.ExportProfile, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT profile_id, display_name, adapter_id, adapter_schema_version,
			state, auto_bind_future_rules, revision, created_at,
			drain_requested_at, stopped_at
		FROM export_profiles
		ORDER BY created_at, profile_id
	`)
	if err != nil {
		return nil, err
	}
	var profiles []edgeapp.ExportProfile
	for rows.Next() {
		var profile edgeapp.ExportProfile
		var drainAt, stoppedAt sql.NullInt64
		if err := rows.Scan(
			&profile.ProfileID,
			&profile.DisplayName,
			&profile.AdapterID,
			&profile.AdapterSchemaVersion,
			&profile.State,
			&profile.AutoBindFutureRules,
			&profile.Revision,
			&profile.CreatedAt,
			&drainAt,
			&stoppedAt,
		); err != nil {
			_ = rows.Close()
			return nil, err
		}
		profile.DrainRequestedAt = nullableInt64Pointer(drainAt)
		profile.StoppedAt = nullableInt64Pointer(stoppedAt)
		profiles = append(profiles, profile)
	}
	if err := rows.Close(); err != nil {
		return nil, err
	}
	for index := range profiles {
		bindings, err := store.listProfileBindings(ctx, profiles[index].ProfileID)
		if err != nil {
			return nil, err
		}
		profiles[index].Bindings = bindings
	}
	return profiles, nil
}

func (store *Store) PreviewExportProfileActivation(
	ctx context.Context,
	adapterID string,
) (edgeapp.ExportProfileActivationPreview, error) {
	var preview edgeapp.ExportProfileActivationPreview
	preview.AdapterID = adapterID
	switch adapterID {
	case "iotkit.mqtt-json.v1", "yokakit.mqtt.v1":
	default:
		return preview, outputadapter.ErrInvalidConfiguration
	}
	rows, err := store.db.QueryContext(ctx, `
		SELECT rule.rule_id, rule.display_name,
			COALESCE(NULLIF(profile.display_name, ''), '名前未設定のセンサー'),
			rule.kind
		FROM semantic_rules_v3 AS rule
		JOIN edge_signals AS signal ON signal.signal_ref = rule.signal_ref
		LEFT JOIN signal_profiles AS profile
			ON profile.edge_node_id = signal.edge_node_id
			AND profile.series_key = signal.series_key
		WHERE rule.retired_at IS NULL
		ORDER BY rule.signal_ref, rule.display_order
	`)
	if err != nil {
		return preview, err
	}
	defer rows.Close()
	for rows.Next() {
		var kind semantics.Kind
		var rule edgeapp.OutputActivationRulePreview
		if err := rows.Scan(
			&rule.RuleID, &rule.DisplayName, &rule.SensorName, &kind,
		); err != nil {
			return preview, err
		}
		rule.Kind = string(kind)
		if adapterID == "iotkit.mqtt-json.v1" {
			rule.Disposition = "automatic"
			preview.AutomaticCount++
			preview.Rules = append(preview.Rules, rule)
			continue
		}
		switch kind {
		case semantics.KindCumulativeCounter, semantics.KindAlarm:
			rule.Disposition = "automatic"
			preview.AutomaticCount++
		case semantics.KindBoolean:
			rule.Disposition = "needs_configuration"
			preview.NeedsConfigurationCount++
		case semantics.KindNumeric:
			rule.Disposition = "ineligible"
			preview.IneligibleCount++
		}
		preview.Rules = append(preview.Rules, rule)
	}
	return preview, rows.Err()
}

func (store *Store) listProfileBindings(
	ctx context.Context,
	profileID string,
) ([]edgeapp.OutputProfileRuleBinding, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT binding.binding_id, binding.profile_id, binding.rule_id,
			COALESCE(binding.output_identity_id, ''),
			COALESCE(rule.display_name, ''), COALESCE(rule.kind, ''),
			COALESCE(rule.signal_ref, ''),
			COALESCE(NULLIF(signal_profile.display_name, ''),
				'名前未設定のセンサー'),
			COALESCE(identity.source_id, ''),
			COALESCE(identity.signal_id, ''), COALESCE(identity.mode, ''),
			binding.reason, binding.state, binding.ineligible_reason,
			binding.revision, binding.created_at, binding.activated_at,
			binding.stopped_at
		FROM output_profile_rule_bindings AS binding
		LEFT JOIN output_signal_identities AS identity
			ON identity.output_identity_id = binding.output_identity_id
		LEFT JOIN semantic_rules_v3 AS rule ON rule.rule_id = binding.rule_id
		LEFT JOIN edge_signals AS signal ON signal.signal_ref = rule.signal_ref
		LEFT JOIN signal_profiles AS signal_profile
			ON signal_profile.edge_node_id = signal.edge_node_id
			AND signal_profile.series_key = signal.series_key
		WHERE binding.profile_id = ?
		ORDER BY binding.created_at, binding.binding_id
	`, profileID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var bindings []edgeapp.OutputProfileRuleBinding
	for rows.Next() {
		var binding edgeapp.OutputProfileRuleBinding
		var activatedAt, stoppedAt sql.NullInt64
		if err := rows.Scan(
			&binding.BindingID,
			&binding.ProfileID,
			&binding.RuleID,
			&binding.OutputIdentityID,
			&binding.RuleDisplayName,
			&binding.RuleKind,
			&binding.SignalRef,
			&binding.SensorName,
			&binding.SourceID,
			&binding.SignalID,
			&binding.Mode,
			&binding.Reason,
			&binding.State,
			&binding.IneligibleReason,
			&binding.Revision,
			&binding.CreatedAt,
			&activatedAt,
			&stoppedAt,
		); err != nil {
			return nil, err
		}
		binding.ActivatedAt = nullableInt64Pointer(activatedAt)
		binding.StoppedAt = nullableInt64Pointer(stoppedAt)
		bindings = append(bindings, binding)
	}
	return bindings, rows.Err()
}

func (store *Store) ConfigureYokaKitBooleanBinding(
	ctx context.Context,
	actor edgeapp.Actor,
	bindingID string,
	mode string,
	expectedRevision int64,
) (edgeapp.OutputProfileRuleBinding, error) {
	var noBinding edgeapp.OutputProfileRuleBinding
	if err := authorizeOutputMutation(actor); err != nil {
		return noBinding, err
	}
	if mode != string(outputadapter.YokaKitOnOff) &&
		mode != string(outputadapter.YokaKitGanttChart) {
		return noBinding, outputadapter.ErrInvalidConfiguration
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noBinding, err
	}
	defer func() { _ = tx.Rollback() }()
	var profile edgeapp.ExportProfile
	var binding edgeapp.OutputProfileRuleBinding
	var semanticKind semantics.Kind
	err = tx.QueryRowContext(ctx, `
		SELECT profile.profile_id, profile.adapter_id,
			profile.adapter_schema_version, profile.state,
			binding.binding_id, binding.rule_id,
			binding.revision, binding.created_at, binding.state,
			rule.signal_ref, rule.display_name, rule.kind
		FROM output_profile_rule_bindings AS binding
		JOIN export_profiles AS profile ON profile.profile_id = binding.profile_id
		JOIN semantic_rules_v3 AS rule ON rule.rule_id = binding.rule_id
		JOIN edge_signals AS signal ON signal.signal_ref = rule.signal_ref
		WHERE binding.binding_id = ? AND rule.retired_at IS NULL
	`, bindingID).Scan(
		&profile.ProfileID, &profile.AdapterID,
		&profile.AdapterSchemaVersion, &profile.State,
		&binding.BindingID, &binding.RuleID,
		&binding.Revision, &binding.CreatedAt, &binding.State,
		&binding.SignalRef, &binding.RuleDisplayName, &semanticKind,
	)
	if errors.Is(err, sql.ErrNoRows) {
		return noBinding, edgeapp.ErrNotFound
	}
	if err != nil {
		return noBinding, err
	}
	if profile.AdapterID != "yokakit.mqtt.v1" ||
		(profile.State != edgeapp.ExportProfileActive &&
			profile.State != edgeapp.ExportProfilePreparing) ||
		binding.State != edgeapp.OutputBindingNeedsConfiguration ||
		semanticKind != semantics.KindBoolean {
		return noBinding, errors.New("output binding is not configurable")
	}
	if binding.Revision != expectedRevision {
		return noBinding, edgeapp.ErrRevisionMismatch
	}
	sourceID, err := validatedEdgeIDTx(ctx, tx)
	if err != nil {
		return noBinding, err
	}
	outputIdentityID, signalID, err := findOrCreateOutputSignalIdentityTx(
		ctx, tx, profile.AdapterID, binding.RuleID, mode, sourceID,
	)
	if err != nil {
		return noBinding, err
	}
	now := time.Now().UnixMilli()
	result, err := tx.ExecContext(ctx, `
		UPDATE output_profile_rule_bindings
		SET output_identity_id = ?, state = 'prepared',
			revision = revision + 1
		WHERE binding_id = ? AND revision = ?
			AND state = 'needs_configuration'
	`, outputIdentityID, bindingID, expectedRevision)
	if err != nil {
		return noBinding, err
	}
	changed, _ := result.RowsAffected()
	if changed != 1 {
		return noBinding, edgeapp.ErrRevisionMismatch
	}
	binding.ProfileID = profile.ProfileID
	binding.OutputIdentityID = outputIdentityID
	binding.SourceID = sourceID
	binding.SignalID = signalID
	binding.Mode = mode
	binding.State = edgeapp.OutputBindingPrepared
	binding.RuleKind = string(semanticKind)
	binding.Revision++
	config, err := bindingRouteConfig(profile.AdapterID, binding, semanticKind)
	if err != nil {
		return noBinding, err
	}
	routeID, err := newResourceRef("out_")
	if err != nil {
		return noBinding, err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO output_routes(
			route_id, rule_id, adapter_id, config_schema_version,
			config_json, start_after_observation_row_id, active, created_at,
			binding_id, lifecycle_state
		) VALUES (?, ?, ?, ?, ?, 0, 0, ?, ?, 'active')
	`, routeID, binding.RuleID, profile.AdapterID,
		profile.AdapterSchemaVersion, []byte(config), now, bindingID); err != nil {
		return noBinding, err
	}
	summary, _ := json.Marshal(map[string]any{
		"adapter_id": profile.AdapterID,
		"mode":       mode,
	})
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt:  now,
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "output_binding.configure",
		ResourceRef: bindingID,
		Outcome:     auditOutcomeSuccess,
		Summary:     summary,
	}); err != nil {
		return noBinding, err
	}
	if err := tx.Commit(); err != nil {
		return noBinding, err
	}
	return binding, nil
}

func (store *Store) RequestExportProfileStop(
	ctx context.Context,
	actor edgeapp.Actor,
	profileID string,
	expectedRevision int64,
) (edgeapp.ExportProfile, error) {
	var noProfile edgeapp.ExportProfile
	if err := authorizeOutputMutation(actor); err != nil {
		return noProfile, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noProfile, err
	}
	defer func() { _ = tx.Rollback() }()
	now := time.Now().UnixMilli()
	result, err := tx.ExecContext(ctx, `
		UPDATE export_profiles
		SET state = 'draining', revision = revision + 1,
			drain_requested_at = ?
		WHERE profile_id = ? AND revision = ?
			AND state IN ('preparing', 'active')
	`, now, profileID, expectedRevision)
	if err != nil {
		return noProfile, err
	}
	changed, _ := result.RowsAffected()
	if changed != 1 {
		return noProfile, edgeapp.ErrRevisionMismatch
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
		WHERE binding.profile_id = ? AND binding.state = 'active'
	`, profileID); err != nil {
		return noProfile, err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO output_binding_ends(
			binding_id, ledger_epoch, end_at_pub_seq
		)
		SELECT binding.binding_id, cursor.ledger_epoch, cursor.accepted_through
		FROM output_profile_rule_bindings AS binding
		JOIN semantic_rules_v3 AS rule ON rule.rule_id = binding.rule_id
		JOIN edge_signals AS signal ON signal.signal_ref = rule.signal_ref
		JOIN accepted_cursors AS cursor
			ON cursor.edge_node_id = signal.edge_node_id
		WHERE binding.profile_id = ? AND binding.state = 'active'
	`, profileID); err != nil {
		return noProfile, err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE output_profile_rule_bindings
		SET state = CASE
				WHEN state = 'active' THEN 'draining'
				ELSE 'stopped'
			END,
			revision = revision + 1,
			stopped_at = CASE
				WHEN state = 'active' THEN NULL
				ELSE ?
			END
		WHERE profile_id = ? AND state IN (
			'active', 'prepared', 'needs_configuration', 'ineligible'
		)
	`, now, profileID); err != nil {
		return noProfile, err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE output_routes
		SET lifecycle_state = 'draining'
		WHERE binding_id IN (
			SELECT binding_id FROM output_profile_rule_bindings
			WHERE profile_id = ? AND state = 'draining'
		)
	`, profileID); err != nil {
		return noProfile, err
	}
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt:  now,
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "export_profile.stop_requested",
		ResourceRef: profileID,
		Outcome:     auditOutcomeSuccess,
		Summary:     json.RawMessage(`{"delivery":"drain"}`),
	}); err != nil {
		return noProfile, err
	}
	if err := tx.Commit(); err != nil {
		return noProfile, err
	}
	profiles, err := store.ListExportProfiles(ctx)
	if err != nil {
		return noProfile, err
	}
	for _, profile := range profiles {
		if profile.ProfileID == profileID {
			return profile, nil
		}
	}
	return noProfile, edgeapp.ErrNotFound
}

func (store *Store) StartPreparedOutputBinding(
	ctx context.Context,
	actor edgeapp.Actor,
	bindingID string,
	expectedRevision int64,
) (edgeapp.OutputProfileRuleBinding, error) {
	var noBinding edgeapp.OutputProfileRuleBinding
	if err := authorizeOutputMutation(actor); err != nil {
		return noBinding, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noBinding, err
	}
	defer func() { _ = tx.Rollback() }()
	var binding edgeapp.OutputProfileRuleBinding
	var profileState edgeapp.ExportProfileState
	var edgeNodeID string
	err = tx.QueryRowContext(ctx, `
		SELECT binding.binding_id, binding.profile_id, binding.rule_id,
			COALESCE(binding.output_identity_id, ''),
			COALESCE(identity.source_id, ''),
			COALESCE(identity.signal_id, ''),
			COALESCE(identity.mode, ''), binding.reason, binding.state,
			binding.revision, binding.created_at,
			rule.signal_ref, rule.display_name, rule.kind,
			profile.state, signal.edge_node_id
		FROM output_profile_rule_bindings AS binding
		LEFT JOIN output_signal_identities AS identity
			ON identity.output_identity_id = binding.output_identity_id
		JOIN export_profiles AS profile ON profile.profile_id = binding.profile_id
		JOIN semantic_rules_v3 AS rule ON rule.rule_id = binding.rule_id
		JOIN edge_signals AS signal ON signal.signal_ref = rule.signal_ref
		WHERE binding.binding_id = ? AND rule.retired_at IS NULL
	`, bindingID).Scan(
		&binding.BindingID, &binding.ProfileID, &binding.RuleID,
		&binding.OutputIdentityID,
		&binding.SourceID, &binding.SignalID, &binding.Mode,
		&binding.Reason, &binding.State, &binding.Revision,
		&binding.CreatedAt, &binding.SignalRef, &binding.RuleDisplayName,
		&binding.RuleKind, &profileState, &edgeNodeID,
	)
	if errors.Is(err, sql.ErrNoRows) {
		return noBinding, edgeapp.ErrNotFound
	}
	if err != nil {
		return noBinding, err
	}
	if binding.State != edgeapp.OutputBindingPrepared ||
		(profileState != edgeapp.ExportProfilePreparing &&
			profileState != edgeapp.ExportProfileActive) {
		return noBinding, errors.New("output binding is not ready to start")
	}
	if binding.Revision != expectedRevision {
		return noBinding, edgeapp.ErrRevisionMismatch
	}
	now := time.Now().UnixMilli()
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO output_binding_starts(
			binding_id, ledger_epoch, start_after_pub_seq
		)
		SELECT ?, ledger_epoch, accepted_through
		FROM accepted_cursors WHERE edge_node_id = ?
	`, bindingID, edgeNodeID); err != nil {
		return noBinding, err
	}
	result, err := tx.ExecContext(ctx, `
		UPDATE output_profile_rule_bindings
		SET state = 'active', revision = revision + 1, activated_at = ?
		WHERE binding_id = ? AND revision = ? AND state = 'prepared'
	`, now, bindingID, expectedRevision)
	if err != nil {
		return noBinding, err
	}
	changed, _ := result.RowsAffected()
	if changed != 1 {
		return noBinding, edgeapp.ErrRevisionMismatch
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE output_routes SET active = 1, lifecycle_state = 'active'
		WHERE binding_id = ? AND active = 0
	`, bindingID); err != nil {
		return noBinding, err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE export_profiles
		SET state = 'active', revision = revision + 1
		WHERE profile_id = ? AND state = 'preparing'
	`, binding.ProfileID); err != nil {
		return noBinding, err
	}
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt:  now,
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "output_binding.start",
		ResourceRef: bindingID,
		Outcome:     auditOutcomeSuccess,
		Summary:     json.RawMessage(`{"future_only":true}`),
	}); err != nil {
		return noBinding, err
	}
	if err := tx.Commit(); err != nil {
		return noBinding, err
	}
	binding.State = edgeapp.OutputBindingActive
	binding.Revision++
	binding.ActivatedAt = &now
	return binding, nil
}

func (store *Store) GetOutputBindingPublication(
	ctx context.Context,
	bindingID string,
) (edgeapp.OutputPublicationPreview, error) {
	var noPreview edgeapp.OutputPublicationPreview
	var routeID, adapterID string
	var config json.RawMessage
	var configBytes []byte
	var configVersion int
	var semanticKind semantics.Kind
	err := store.db.QueryRowContext(ctx, `
		SELECT route.route_id, route.adapter_id, route.config_schema_version,
			route.config_json, rule.kind
		FROM output_routes AS route
		JOIN semantic_rules_v3 AS rule ON rule.rule_id = route.rule_id
		WHERE route.binding_id = ?
	`, bindingID).Scan(
		&routeID, &adapterID, &configVersion, &configBytes, &semanticKind,
	)
	if errors.Is(err, sql.ErrNoRows) {
		return noPreview, edgeapp.ErrNotFound
	}
	if err != nil {
		return noPreview, err
	}
	config = append(json.RawMessage(nil), configBytes...)
	adapter, descriptor, err := resolveOutputAdapter(adapterID)
	if err != nil {
		return noPreview, err
	}
	if configVersion != descriptor.ConfigSchemaVersion {
		return noPreview, outputadapter.ErrInvalidConfiguration
	}
	preview := edgeapp.OutputPublicationPreview{
		BindingID:  bindingID,
		Provenance: "actual",
		QoS:        1,
	}
	var actualPayload []byte
	err = store.db.QueryRowContext(ctx, `
		SELECT topic, qos, payload_json
		FROM output_outbox_v3
		WHERE route_id = ?
		ORDER BY created_at DESC, export_id DESC
		LIMIT 1
	`, routeID).Scan(&preview.Topic, &preview.QoS, &actualPayload)
	if err == nil {
		preview.Payload = append(json.RawMessage(nil), actualPayload...)
		return preview, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return noPreview, err
	}
	var observation semantics.Observation
	var value []byte
	err = store.db.QueryRowContext(ctx, `
		SELECT observation.observation_id, observation.series_id,
			observation.sequence, observation.kind, observation.value_json,
			observation.observed_at
		FROM semantic_observations_v3 AS observation
		JOIN output_routes AS route ON route.rule_id = observation.rule_id
		WHERE route.route_id = ?
		ORDER BY observation.observation_row_id DESC
		LIMIT 1
	`, routeID).Scan(
		&observation.ObservationID, &observation.SeriesID,
		&observation.Sequence, &observation.Kind, &value,
		&observation.ObservedAt,
	)
	if err == nil {
		observation.Value = append(json.RawMessage(nil), value...)
		adapterObservation, transformErr := outputObservation(observation)
		if transformErr != nil {
			return noPreview, transformErr
		}
		publication, transformErr := adapter.Transform(config, adapterObservation)
		if transformErr != nil {
			return noPreview, transformErr
		}
		return previewFromPublication(
			bindingID, "latest_observation", publication,
		), nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return noPreview, err
	}
	sample, err := sampleOutputObservation(semanticKind)
	if err != nil {
		return noPreview, err
	}
	publication, err := adapter.Transform(config, sample)
	if err != nil {
		return noPreview, err
	}
	return previewFromPublication(bindingID, "sample", publication), nil
}

func (store *Store) ReconcileExportProfileLifecycle(ctx context.Context) error {
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer func() { _ = tx.Rollback() }()
	now := time.Now().UnixMilli()
	if _, err := tx.ExecContext(ctx, `
		UPDATE output_profile_rule_bindings AS binding
		SET state = 'stopped', revision = revision + 1, stopped_at = ?
		WHERE binding.state = 'draining'
			AND NOT EXISTS (
				SELECT 1
				FROM output_binding_ends AS boundary
				JOIN output_binding_starts AS start
					ON start.binding_id = boundary.binding_id
					AND start.ledger_epoch = boundary.ledger_epoch
				JOIN semantic_rules_v3 AS rule
					ON rule.rule_id = binding.rule_id
				JOIN edge_signals AS signal
					ON signal.signal_ref = rule.signal_ref
				JOIN raw_records AS raw
					ON raw.edge_node_id = signal.edge_node_id
					AND raw.ledger_epoch = boundary.ledger_epoch
					AND raw.pub_seq > start.start_after_pub_seq
					AND raw.pub_seq <= boundary.end_at_pub_seq
					AND json_extract(raw.record_json, '$.series_key') =
						signal.series_key
				WHERE boundary.binding_id = binding.binding_id
					AND NOT EXISTS (
						SELECT 1
						FROM semantic_projection_receipts_v3 AS receipt
						WHERE receipt.rule_id = binding.rule_id
							AND receipt.ledger_epoch = raw.ledger_epoch
							AND receipt.pub_seq = raw.pub_seq
					)
			)
			AND NOT EXISTS (
				SELECT 1
				FROM output_routes AS route
				JOIN semantic_observations_v3 AS observation
					ON observation.rule_id = route.rule_id
				JOIN output_binding_starts AS start
					ON start.binding_id = binding.binding_id
					AND start.ledger_epoch = observation.ledger_epoch
				LEFT JOIN output_binding_ends AS finish
					ON finish.binding_id = binding.binding_id
					AND finish.ledger_epoch = observation.ledger_epoch
				WHERE route.binding_id = binding.binding_id
					AND observation.source_pub_seq > start.start_after_pub_seq
					AND (
						finish.binding_id IS NULL OR
						observation.source_pub_seq <= finish.end_at_pub_seq
					)
					AND NOT EXISTS (
						SELECT 1 FROM output_outbox_v3 AS outbox
						WHERE outbox.route_id = route.route_id
							AND outbox.observation_id =
								observation.observation_id
					)
			)
			AND NOT EXISTS (
				SELECT 1
				FROM output_routes AS route
				JOIN output_outbox_v3 AS outbox
					ON outbox.route_id = route.route_id
				WHERE route.binding_id = binding.binding_id
					AND outbox.published_at IS NULL
			)
			AND NOT EXISTS (
				SELECT 1 FROM output_routes AS route
				WHERE route.binding_id = binding.binding_id
					AND route.last_transform_error_code IS NOT NULL
			)
	`, now); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE output_routes
		SET active = 0, lifecycle_state = 'stopped'
		WHERE binding_id IN (
			SELECT binding_id FROM output_profile_rule_bindings
			WHERE state = 'stopped'
		) AND lifecycle_state = 'draining'
	`); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE export_profiles AS profile
		SET state = 'stopped', revision = revision + 1, stopped_at = ?
		WHERE profile.state = 'draining'
			AND NOT EXISTS (
				SELECT 1 FROM output_profile_rule_bindings AS binding
				WHERE binding.profile_id = profile.profile_id
					AND binding.state NOT IN ('stopped', 'ineligible')
			)
	`, now); err != nil {
		return err
	}
	return tx.Commit()
}

func previewFromPublication(
	bindingID string,
	provenance string,
	publication outputadapter.MQTTPublication,
) edgeapp.OutputPublicationPreview {
	return edgeapp.OutputPublicationPreview{
		BindingID:  bindingID,
		Provenance: provenance,
		Topic:      publication.Topic,
		QoS:        publication.QoS,
		Retain:     publication.Retain,
		Payload:    append(json.RawMessage(nil), publication.Payload...),
	}
}

func sampleOutputObservation(
	kind semantics.Kind,
) (outputadapter.Observation, error) {
	outputKind, err := outputObservationKind(kind)
	if err != nil {
		return outputadapter.Observation{}, err
	}
	value := json.RawMessage(`24.8`)
	switch outputKind {
	case outputadapter.KindBoolean, outputadapter.KindAlarm:
		value = json.RawMessage(`true`)
	case outputadapter.KindCumulativeValue:
		value = json.RawMessage(`1`)
	}
	sample := outputadapter.Observation{
		ObservationID: "00000000-0000-4000-8000-000000000001",
		SeriesID:      "00000000-0000-4000-8000-000000000002",
		Sequence:      1,
		ObservedAt:    0,
		Kind:          outputKind,
		Value:         value,
	}
	if outputKind == outputadapter.KindAlarm {
		reading := 80.0
		sample.Reading = &reading
	}
	return sample, sample.Validate()
}

func autoBindSemanticRuleTx(
	ctx context.Context,
	tx *sqlTx,
	rule semantics.Rule,
	edgeNodeID string,
) error {
	rows, err := tx.QueryContext(ctx, `
		SELECT profile_id, display_name, adapter_id,
			adapter_schema_version, state, auto_bind_future_rules,
			revision, created_at
		FROM export_profiles
		WHERE state IN ('preparing', 'active')
			AND auto_bind_future_rules = 1
		ORDER BY created_at, profile_id
	`)
	if err != nil {
		return err
	}
	var profiles []edgeapp.ExportProfile
	for rows.Next() {
		var profile edgeapp.ExportProfile
		if err := rows.Scan(
			&profile.ProfileID, &profile.DisplayName, &profile.AdapterID,
			&profile.AdapterSchemaVersion, &profile.State,
			&profile.AutoBindFutureRules, &profile.Revision,
			&profile.CreatedAt,
		); err != nil {
			_ = rows.Close()
			return err
		}
		profiles = append(profiles, profile)
	}
	if err := rows.Close(); err != nil {
		return err
	}
	if len(profiles) == 0 {
		return nil
	}
	sourceID, err := validatedEdgeIDTx(ctx, tx)
	if err != nil {
		return err
	}
	for _, profile := range profiles {
		if _, err := createProfileBindingTx(
			ctx, tx, profile, sourceID, rule.ID, rule.SignalRef,
			rule.DisplayName, rule.Kind, edgeNodeID,
		); err != nil {
			return err
		}
	}
	return nil
}

func drainOutputBindingsForRuleTx(
	ctx context.Context,
	tx *sqlTx,
	ruleID string,
	edgeNodeID string,
) error {
	now := time.Now().UnixMilli()
	if _, err := tx.ExecContext(ctx, `
		INSERT OR IGNORE INTO output_binding_starts(
			binding_id, ledger_epoch, start_after_pub_seq
		)
		SELECT binding.binding_id, cursor.ledger_epoch, 0
		FROM output_profile_rule_bindings AS binding
		JOIN accepted_cursors AS cursor ON cursor.edge_node_id = ?
		WHERE binding.rule_id = ? AND binding.state = 'active'
	`, edgeNodeID, ruleID); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT OR IGNORE INTO output_binding_ends(
			binding_id, ledger_epoch, end_at_pub_seq
		)
		SELECT binding.binding_id, cursor.ledger_epoch, cursor.accepted_through
		FROM output_profile_rule_bindings AS binding
		JOIN accepted_cursors AS cursor ON cursor.edge_node_id = ?
		WHERE binding.rule_id = ? AND binding.state = 'active'
	`, edgeNodeID, ruleID); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE output_profile_rule_bindings
		SET state = 'draining', revision = revision + 1
		WHERE rule_id = ? AND state = 'active'
	`, ruleID); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE output_profile_rule_bindings
		SET state = 'stopped', revision = revision + 1, stopped_at = ?
		WHERE rule_id = ? AND state IN (
			'needs_configuration', 'prepared', 'ineligible'
		)
	`, now, ruleID); err != nil {
		return err
	}
	_, err := tx.ExecContext(ctx, `
		UPDATE output_routes
		SET lifecycle_state = 'draining'
		WHERE binding_id IN (
			SELECT binding_id FROM output_profile_rule_bindings
			WHERE rule_id = ? AND state = 'draining'
		)
	`, ruleID)
	return err
}

func createProfileBindingTx(
	ctx context.Context,
	tx *sqlTx,
	profile edgeapp.ExportProfile,
	sourceID string,
	ruleID string,
	signalRef string,
	displayName string,
	semanticKind semantics.Kind,
	edgeNodeID string,
) (edgeapp.OutputProfileRuleBinding, error) {
	var noBinding edgeapp.OutputProfileRuleBinding
	bindingID, err := newResourceRef("bind_")
	if err != nil {
		return noBinding, err
	}
	now := time.Now().UnixMilli()
	binding := edgeapp.OutputProfileRuleBinding{
		BindingID:       bindingID,
		ProfileID:       profile.ProfileID,
		RuleID:          ruleID,
		RuleDisplayName: displayName,
		RuleKind:        string(semanticKind),
		SignalRef:       signalRef,
		SourceID:        sourceID,
		Revision:        1,
		CreatedAt:       now,
	}
	switch profile.AdapterID {
	case "iotkit.mqtt-json.v1":
		binding.Mode = "observation"
		binding.State = edgeapp.OutputBindingActive
	case "yokakit.mqtt.v1":
		switch semanticKind {
		case semantics.KindCumulativeCounter:
			binding.Mode = string(outputadapter.YokaKitProduction)
			binding.State = edgeapp.OutputBindingPrepared
		case semantics.KindAlarm:
			binding.Mode = string(outputadapter.YokaKitAlarm)
			binding.State = edgeapp.OutputBindingPrepared
		case semantics.KindBoolean:
			binding.State = edgeapp.OutputBindingNeedsConfiguration
		case semantics.KindNumeric:
			binding.State = edgeapp.OutputBindingIneligible
			binding.IneligibleReason = "YokaKitは連続数値を受け取りません"
		default:
			return noBinding, outputadapter.ErrUnsupportedObservation
		}
	default:
		return noBinding, outputadapter.ErrInvalidConfiguration
	}
	var activatedAt any
	if binding.State == edgeapp.OutputBindingActive ||
		binding.State == edgeapp.OutputBindingPrepared {
		outputIdentityID, signalID, err := findOrCreateOutputSignalIdentityTx(
			ctx,
			tx,
			profile.AdapterID,
			binding.RuleID,
			binding.Mode,
			binding.SourceID,
		)
		if err != nil {
			return noBinding, err
		}
		binding.OutputIdentityID = outputIdentityID
		binding.SignalID = signalID
		if binding.State == edgeapp.OutputBindingActive {
			binding.ActivatedAt = &now
			activatedAt = now
		}
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO output_profile_rule_bindings(
			binding_id, profile_id, rule_id, output_identity_id,
			reason, state, ineligible_reason, revision,
			created_at, activated_at
		) VALUES (?, ?, ?, NULLIF(?, ''), ?, ?, ?, 1, ?, ?)
	`, binding.BindingID, binding.ProfileID, binding.RuleID,
		binding.OutputIdentityID, binding.Reason, binding.State,
		binding.IneligibleReason, binding.CreatedAt, activatedAt); err != nil {
		return noBinding, err
	}
	if binding.State != edgeapp.OutputBindingActive &&
		binding.State != edgeapp.OutputBindingPrepared {
		return binding, nil
	}
	if binding.State == edgeapp.OutputBindingActive {
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO output_binding_starts(
				binding_id, ledger_epoch, start_after_pub_seq
			)
			SELECT ?, ledger_epoch, accepted_through
			FROM accepted_cursors WHERE edge_node_id = ?
		`, binding.BindingID, edgeNodeID); err != nil {
			return noBinding, err
		}
	}
	config, err := bindingRouteConfig(profile.AdapterID, binding, semanticKind)
	if err != nil {
		return noBinding, err
	}
	adapter, descriptor, err := resolveOutputAdapter(profile.AdapterID)
	if err != nil {
		return noBinding, err
	}
	if err := adapter.ValidateConfig(config, consoleOutputKind(semanticKind)); err != nil {
		return noBinding, err
	}
	routeID, err := newResourceRef("out_")
	if err != nil {
		return noBinding, err
	}
	_, err = tx.ExecContext(ctx, `
		INSERT INTO output_routes(
			route_id, rule_id, adapter_id, config_schema_version,
			config_json, start_after_observation_row_id, active, created_at,
			binding_id, lifecycle_state
		) VALUES (?, ?, ?, ?, ?, 0, ?, ?, ?, 'active')
	`, routeID, ruleID, profile.AdapterID, descriptor.ConfigSchemaVersion,
		[]byte(config), binding.State == edgeapp.OutputBindingActive,
		now, binding.BindingID)
	return binding, err
}

func findOrCreateOutputSignalIdentityTx(
	ctx context.Context,
	tx *sqlTx,
	adapterID string,
	ruleID string,
	mode string,
	sourceID string,
) (string, string, error) {
	var outputIdentityID, storedSourceID, signalID string
	err := tx.QueryRowContext(ctx, `
		SELECT output_identity_id, source_id, signal_id
		FROM output_signal_identities
		WHERE adapter_id = ? AND rule_id = ? AND mode = ?
	`, adapterID, ruleID, mode).Scan(
		&outputIdentityID,
		&storedSourceID,
		&signalID,
	)
	if err == nil {
		if storedSourceID != sourceID {
			return "", "", errors.New(
				"output signal identity belongs to a different Edge",
			)
		}
		return outputIdentityID, signalID, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return "", "", err
	}
	signalID, err = newOutputSignalID()
	if err != nil {
		return "", "", err
	}
	outputIdentityID, err = newResourceRef("osi_")
	if err != nil {
		return "", "", err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO output_signal_identities(
			output_identity_id, adapter_id, rule_id, mode,
			source_id, signal_id, created_at
		) VALUES (?, ?, ?, ?, ?, ?, ?)
	`, outputIdentityID, adapterID, ruleID, mode, sourceID, signalID,
		time.Now().UnixMilli()); err != nil {
		return "", "", err
	}
	return outputIdentityID, signalID, nil
}

func bindingRouteConfig(
	adapterID string,
	binding edgeapp.OutputProfileRuleBinding,
	semanticKind semantics.Kind,
) (json.RawMessage, error) {
	switch adapterID {
	case "iotkit.mqtt-json.v1":
		return outputadapter.EncodeGenericMQTTJSONConfig(
			outputadapter.GenericMQTTJSONConfig{
				Topic: "iotkit/v1/sources/" + binding.SourceID +
					"/signals/" + binding.SignalID + "/observations",
			},
		)
	case "yokakit.mqtt.v1":
		return outputadapter.EncodeYokaKitConfig(outputadapter.YokaKitConfig{
			SourceID: binding.SourceID,
			SignalID: binding.SignalID,
			Kind:     outputadapter.YokaKitKind(binding.Mode),
			Reason:   binding.Reason,
		})
	default:
		return nil, fmt.Errorf(
			"%w: unknown profile adapter %q",
			outputadapter.ErrInvalidConfiguration,
			adapterID,
		)
	}
}

func consoleOutputKind(kind semantics.Kind) outputadapter.ObservationKind {
	switch kind {
	case semantics.KindNumeric:
		return outputadapter.KindNumeric
	case semantics.KindBoolean:
		return outputadapter.KindBoolean
	case semantics.KindCumulativeCounter:
		return outputadapter.KindCumulativeValue
	case semantics.KindAlarm:
		return outputadapter.KindAlarm
	default:
		return ""
	}
}

func validatedEdgeIDTx(ctx context.Context, tx *sqlTx) (string, error) {
	var count int
	var edgeID string
	if err := tx.QueryRowContext(ctx, `
		SELECT count(*), COALESCE(MAX(edge_id), '') FROM edge_meta
	`).Scan(&count, &edgeID); err != nil {
		return "", err
	}
	if count != 1 || !edgeIDPattern.MatchString(edgeID) {
		return "", errors.New("Edge identity metadata is missing or invalid")
	}
	return edgeID, nil
}

func newOutputSignalID() (string, error) {
	value := make([]byte, 16)
	if _, err := outputRandomRead(value); err != nil {
		return "", fmt.Errorf("generate output signal ID: %w", err)
	}
	return "sig-" + hex.EncodeToString(value), nil
}

func authorizeOutputMutation(actor edgeapp.Actor) error {
	if err := actor.Validate(); err != nil {
		return err
	}
	if actor.Class == edgeapp.ActorLocalCLI {
		return nil
	}
	if actor.Class == edgeapp.ActorAccount &&
		(actor.Role == edgeapp.AccountRoleAdmin ||
			actor.Role == edgeapp.AccountRoleSystemAdmin) {
		return nil
	}
	return edgeapp.ErrForbidden
}
