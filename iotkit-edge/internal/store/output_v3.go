package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
)

type OutputRoute = edgeapp.OutputRoute
type YokaKitRuleRoute = edgeapp.YokaKitRuleRoute

func (store *Store) ApplyOutputRoute(
	ctx context.Context,
	actor edgeapp.Actor,
	ruleID string,
	adapterID string,
	config json.RawMessage,
) (OutputRoute, error) {
	var noRoute OutputRoute
	if err := actor.Validate(); err != nil {
		return noRoute, err
	}
	adapter, descriptor, err := resolveOutputAdapter(adapterID)
	if err != nil {
		return noRoute, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noRoute, err
	}
	defer func() { _ = tx.Rollback() }()
	var semanticKind semantics.Kind
	if err := tx.QueryRowContext(ctx, `
		SELECT kind FROM semantic_rules_v3
		WHERE rule_id = ? AND retired_at IS NULL
	`, ruleID).Scan(&semanticKind); errors.Is(err, sql.ErrNoRows) {
		return noRoute, edgeapp.ErrNotFound
	} else if err != nil {
		return noRoute, err
	}
	outputKind, err := outputObservationKind(semanticKind)
	if err != nil {
		return noRoute, err
	}
	if err := adapter.ValidateConfig(config, outputKind); err != nil {
		return noRoute, err
	}
	var start int64
	if err := tx.QueryRowContext(ctx,
		`SELECT COALESCE(MAX(observation_row_id), 0) FROM semantic_observations_v3`,
	).Scan(&start); err != nil {
		return noRoute, err
	}
	routeID, err := newResourceRef("out_")
	if err != nil {
		return noRoute, err
	}
	route := OutputRoute{
		RouteID:                    routeID,
		RuleID:                     ruleID,
		AdapterID:                  descriptor.ID,
		ConfigSchemaVersion:        descriptor.ConfigSchemaVersion,
		Config:                     append(json.RawMessage(nil), config...),
		StartAfterObservationRowID: start,
		Active:                     true,
		LifecycleState:             "active",
		CreatedAt:                  time.Now().UnixMilli(),
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO output_routes(
			route_id, rule_id, adapter_id, config_schema_version,
			config_json, start_after_observation_row_id, active, created_at
		) VALUES (?, ?, ?, ?, ?, ?, 1, ?)
	`, route.RouteID, route.RuleID, route.AdapterID,
		route.ConfigSchemaVersion, []byte(route.Config),
		route.StartAfterObservationRowID, route.CreatedAt); err != nil {
		return noRoute, err
	}
	summary, _ := json.Marshal(struct {
		RuleID              string `json:"rule_id"`
		AdapterID           string `json:"adapter_id"`
		ConfigSchemaVersion int    `json:"config_schema_version"`
	}{
		RuleID:              route.RuleID,
		AdapterID:           route.AdapterID,
		ConfigSchemaVersion: route.ConfigSchemaVersion,
	})
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt:  route.CreatedAt,
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "output_route.create",
		ResourceRef: route.RouteID,
		Outcome:     auditOutcomeSuccess,
		Summary:     summary,
	}); err != nil {
		return noRoute, err
	}
	if err := tx.Commit(); err != nil {
		return noRoute, err
	}
	return route, nil
}

func (store *Store) ApplyYokaKitRuleRoute(
	ctx context.Context,
	actor edgeapp.Actor,
	ruleID string,
	config outputadapter.YokaKitConfig,
) (YokaKitRuleRoute, error) {
	var noRoute YokaKitRuleRoute
	encoded, err := outputadapter.EncodeYokaKitConfig(config)
	if err != nil {
		return noRoute, err
	}
	route, err := store.ApplyOutputRoute(
		ctx,
		actor,
		ruleID,
		"yokakit.mqtt.v1",
		encoded,
	)
	if err != nil {
		return noRoute, err
	}
	return yokaKitRuleRoute(route)
}

func (store *Store) ListOutputRoutes(
	ctx context.Context,
) ([]OutputRoute, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT route.route_id, COALESCE(route.binding_id, ''),
			route.rule_id, route.adapter_id,
			route.config_schema_version, route.config_json,
			route.start_after_observation_row_id, route.active,
			route.lifecycle_state, route.created_at,
			COALESCE(route.last_transform_error_code, ''),
			route.last_transform_error_at, route.last_transform_success_at,
			COALESCE(rule.signal_ref, ''), COALESCE(rule.display_name, ''),
			COALESCE(rule.kind, ''),
			COALESCE(SUM(CASE WHEN outbox.export_id IS NOT NULL
				AND outbox.published_at IS NULL THEN 1 ELSE 0 END), 0),
			COALESCE(SUM(CASE WHEN outbox.published_at IS NOT NULL
				THEN 1 ELSE 0 END), 0),
			MAX(outbox.published_at),
			MIN(CASE WHEN outbox.published_at IS NULL
				THEN outbox.created_at END)
		FROM output_routes AS route
		LEFT JOIN semantic_rules_v3 AS rule ON rule.rule_id = route.rule_id
		LEFT JOIN output_outbox_v3 AS outbox ON outbox.route_id = route.route_id
		GROUP BY route.route_id, rule.signal_ref, rule.display_name, rule.kind
		ORDER BY route.created_at, route.route_id
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var routes []OutputRoute
	for rows.Next() {
		var route OutputRoute
		var config []byte
		var transformErrorAt, transformSuccessAt, lastPublishedAt sql.NullInt64
		var oldestPendingAt sql.NullInt64
		if err := rows.Scan(
			&route.RouteID,
			&route.BindingID,
			&route.RuleID,
			&route.AdapterID,
			&route.ConfigSchemaVersion,
			&config,
			&route.StartAfterObservationRowID,
			&route.Active,
			&route.LifecycleState,
			&route.CreatedAt,
			&route.LastTransformErrorCode,
			&transformErrorAt,
			&transformSuccessAt,
			&route.SignalRef,
			&route.RuleDisplayName,
			&route.RuleKind,
			&route.PendingCount,
			&route.PublishedCount,
			&lastPublishedAt,
			&oldestPendingAt,
		); err != nil {
			return nil, err
		}
		route.Config = append(json.RawMessage(nil), config...)
		route.LastTransformErrorAt = nullableInt64Pointer(transformErrorAt)
		route.LastTransformSuccessAt = nullableInt64Pointer(transformSuccessAt)
		route.LastPublishedAt = nullableInt64Pointer(lastPublishedAt)
		route.OldestPendingAt = nullableInt64Pointer(oldestPendingAt)
		routes = append(routes, route)
	}
	return routes, rows.Err()
}

func (store *Store) ListYokaKitRuleRoutes(
	ctx context.Context,
) ([]YokaKitRuleRoute, error) {
	routes, err := store.ListOutputRoutes(ctx)
	if err != nil {
		return nil, err
	}
	result := make([]YokaKitRuleRoute, 0, len(routes))
	for _, route := range routes {
		if route.AdapterID != "yokakit.mqtt.v1" {
			continue
		}
		converted, err := yokaKitRuleRoute(route)
		if err != nil {
			return nil, err
		}
		result = append(result, converted)
	}
	return result, nil
}

func yokaKitRuleRoute(route OutputRoute) (YokaKitRuleRoute, error) {
	config, err := outputadapter.DecodeYokaKitConfig(route.Config)
	if err != nil {
		return YokaKitRuleRoute{}, err
	}
	return YokaKitRuleRoute{
		RouteID:                    route.RouteID,
		RuleID:                     route.RuleID,
		SourceID:                   config.SourceID,
		SignalID:                   config.SignalID,
		Kind:                       config.Kind,
		Reason:                     config.Reason,
		StartAfterObservationRowID: route.StartAfterObservationRowID,
		Active:                     route.Active,
		CreatedAt:                  route.CreatedAt,
		PendingCount:               route.PendingCount,
		PublishedCount:             route.PublishedCount,
	}, nil
}

func (store *Store) EnqueueMultipleRuleOutputExports(
	ctx context.Context,
	limit int,
) (int, error) {
	if limit < 1 || limit > 10_000 {
		return 0, errors.New("output enqueue limit must be between 1 and 10000")
	}
	rows, err := store.db.QueryContext(ctx, `
		WITH candidate AS (
			SELECT route.route_id, route.adapter_id,
				route.config_schema_version, route.config_json,
				observation.observation_row_id, observation.observation_id,
				observation.series_id, observation.sequence,
				observation.rule_id, observation.rule_revision, observation.kind,
				observation.value_json, observation.signal_ref,
				observation.edge_node_id, observation.ledger_epoch,
				observation.source_pub_seq, observation.observed_at,
				observation.created_at,
				CASE WHEN route.last_transform_error_code IS NULL
					THEN 0 ELSE 1 END AS route_error_rank,
				CASE
					WHEN COALESCE(route.last_transform_error_at, 0) >=
						COALESCE(route.last_transform_success_at, 0)
						AND COALESCE(route.last_transform_error_at, 0) >=
							route.created_at
					THEN COALESCE(route.last_transform_error_at, 0)
					WHEN COALESCE(route.last_transform_success_at, 0) >=
						route.created_at
					THEN COALESCE(route.last_transform_success_at, 0)
					ELSE route.created_at
				END AS route_attempt_at,
				ROW_NUMBER() OVER (
					PARTITION BY route.route_id
					ORDER BY observation.observation_row_id
				) AS route_rank
			FROM output_routes AS route
			JOIN semantic_observations_v3 AS observation
				ON observation.rule_id = route.rule_id
				AND (
					(
						route.binding_id IS NULL AND
						observation.observation_row_id >
							route.start_after_observation_row_id
					) OR (
						route.binding_id IS NOT NULL AND EXISTS (
							SELECT 1 FROM output_binding_starts AS starts
							WHERE starts.binding_id = route.binding_id
								AND starts.ledger_epoch = observation.ledger_epoch
								AND observation.source_pub_seq >
									starts.start_after_pub_seq
						) AND (
							NOT EXISTS (
								SELECT 1 FROM output_binding_ends AS ends
								WHERE ends.binding_id = route.binding_id
									AND ends.ledger_epoch =
										observation.ledger_epoch
							) OR EXISTS (
								SELECT 1 FROM output_binding_ends AS ends
								WHERE ends.binding_id = route.binding_id
									AND ends.ledger_epoch =
										observation.ledger_epoch
									AND observation.source_pub_seq <=
										ends.end_at_pub_seq
							)
						)
					)
				)
			WHERE route.active = 1
				AND route.lifecycle_state IN ('active', 'draining')
				AND NOT EXISTS (
				SELECT 1 FROM output_outbox_v3 AS outbox
				WHERE outbox.route_id = route.route_id
					AND outbox.observation_id = observation.observation_id
			)
		)
		SELECT route_id, adapter_id, config_schema_version, config_json,
			observation_row_id, observation_id, series_id, sequence,
			rule_id, rule_revision, kind, value_json, signal_ref,
			edge_node_id, ledger_epoch, source_pub_seq, observed_at, created_at
		FROM candidate
		WHERE route_error_rank = 0 OR route_rank = 1
		ORDER BY route_rank, route_attempt_at, observation_row_id, route_id
		LIMIT ?
	`, limit)
	if err != nil {
		return 0, err
	}
	type candidate struct {
		routeID, adapterID  string
		configSchemaVersion int
		config              json.RawMessage
		observation         semantics.Observation
	}
	var candidates []candidate
	for rows.Next() {
		var item candidate
		var rowID int64
		var config, value []byte
		if err := rows.Scan(
			&item.routeID,
			&item.adapterID,
			&item.configSchemaVersion,
			&config,
			&rowID,
			&item.observation.ObservationID,
			&item.observation.SeriesID,
			&item.observation.Sequence,
			&item.observation.DefinitionID,
			&item.observation.DefinitionRevision,
			&item.observation.Kind,
			&value,
			&item.observation.SignalRef,
			&item.observation.EdgeNodeID,
			&item.observation.LedgerEpoch,
			&item.observation.SourcePubSeq,
			&item.observation.ObservedAt,
			&item.observation.CreatedAt,
		); err != nil {
			_ = rows.Close()
			return 0, err
		}
		item.observation.RowID = rowID
		item.observation.Value = value
		item.config = append(json.RawMessage(nil), config...)
		candidates = append(candidates, item)
	}
	if err := rows.Close(); err != nil {
		return 0, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return 0, err
	}
	defer func() { _ = tx.Rollback() }()
	inserted := 0
	var transformErrors []error
	failedRoutes := make(map[string]struct{})
	recordTransformFailure := func(
		routeID string,
		code string,
		at int64,
		cause error,
	) error {
		if _, err := tx.ExecContext(ctx, `
			UPDATE output_routes
			SET last_transform_error_code = ?, last_transform_error_at = ?
			WHERE route_id = ?
		`, code, at, routeID); err != nil {
			return err
		}
		transformErrors = append(
			transformErrors,
			fmt.Errorf("output route %s: %w", routeID, cause),
		)
		failedRoutes[routeID] = struct{}{}
		return nil
	}
	for _, item := range candidates {
		if _, failed := failedRoutes[item.routeID]; failed {
			continue
		}
		adapter, descriptor, err := resolveOutputAdapter(item.adapterID)
		if err != nil {
			if recordErr := recordTransformFailure(
				item.routeID,
				"adapter_unavailable",
				time.Now().UnixMilli(),
				err,
			); recordErr != nil {
				return 0, recordErr
			}
			continue
		}
		if item.configSchemaVersion != descriptor.ConfigSchemaVersion {
			mismatch := fmt.Errorf(
				"%w: route %s uses config schema %d, adapter expects %d",
				outputadapter.ErrInvalidConfiguration,
				item.routeID,
				item.configSchemaVersion,
				descriptor.ConfigSchemaVersion,
			)
			if recordErr := recordTransformFailure(
				item.routeID,
				"config_version_mismatch",
				time.Now().UnixMilli(),
				mismatch,
			); recordErr != nil {
				return 0, recordErr
			}
			continue
		}
		observation, err := outputObservation(item.observation)
		if err != nil {
			if recordErr := recordTransformFailure(
				item.routeID,
				"invalid_observation",
				time.Now().UnixMilli(),
				err,
			); recordErr != nil {
				return 0, recordErr
			}
			continue
		}
		message, err := adapter.Transform(item.config, observation)
		if err != nil {
			code := "transform_failed"
			if errors.Is(err, outputadapter.ErrInvalidObservation) {
				code = "invalid_observation"
			}
			if recordErr := recordTransformFailure(
				item.routeID,
				code,
				time.Now().UnixMilli(),
				err,
			); recordErr != nil {
				return 0, recordErr
			}
			continue
		}
		transformedAt := time.Now().UnixMilli()
		result, err := tx.ExecContext(ctx, `
			INSERT OR IGNORE INTO output_outbox_v3(
				export_id, route_id, observation_id, topic, qos,
				payload_json, created_at
			) VALUES (?, ?, ?, ?, ?, ?, ?)
		`, mqttExportID(item.routeID, item.observation.ObservationID),
			item.routeID, item.observation.ObservationID, message.Topic,
			message.QoS, message.Payload, transformedAt)
		if err != nil {
			return 0, err
		}
		if _, err := tx.ExecContext(ctx, `
			UPDATE output_routes
			SET last_transform_error_code = NULL,
				last_transform_error_at = NULL,
				last_transform_success_at = ?
			WHERE route_id = ?
		`, transformedAt, item.routeID); err != nil {
			return 0, err
		}
		count, _ := result.RowsAffected()
		inserted += int(count)
	}
	if err := tx.Commit(); err != nil {
		return 0, err
	}
	lifecycleErr := store.ReconcileExportProfileLifecycle(ctx)
	return inserted, errors.Join(append(transformErrors, lifecycleErr)...)
}

func nullableInt64Pointer(value sql.NullInt64) *int64 {
	if !value.Valid {
		return nil
	}
	result := value.Int64
	return &result
}

func resolveOutputAdapter(
	adapterID string,
) (outputadapter.Adapter, outputadapter.Descriptor, error) {
	registry, err := outputadapter.BuiltInRegistry()
	if err != nil {
		return nil, outputadapter.Descriptor{}, err
	}
	adapter, ok := registry.Resolve(adapterID)
	if !ok {
		return nil, outputadapter.Descriptor{}, fmt.Errorf(
			"%w: unknown output adapter %q",
			outputadapter.ErrInvalidConfiguration,
			adapterID,
		)
	}
	return adapter, adapter.Descriptor(), nil
}
