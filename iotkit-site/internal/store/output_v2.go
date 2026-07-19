package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

type YokaKitRoute struct {
	RouteID                    string                    `json:"route_id"`
	DefinitionID               string                    `json:"definition_id"`
	SourceID                   string                    `json:"source_id"`
	SignalID                   string                    `json:"signal_id"`
	Kind                       outputadapter.YokaKitKind `json:"kind"`
	Reason                     string                    `json:"reason,omitempty"`
	StartAfterObservationRowID int64                     `json:"start_after_observation_row_id"`
	Active                     bool                      `json:"active"`
	CreatedAt                  int64                     `json:"created_at"`
	PendingCount               int64                     `json:"pending_count"`
	PublishedCount             int64                     `json:"published_count"`
}

func (store *Store) ApplyYokaKitRoute(
	ctx context.Context,
	actor siteapp.Actor,
	definitionID string,
	config outputadapter.YokaKitConfig,
) (YokaKitRoute, error) {
	var noRoute YokaKitRoute
	if err := actor.Validate(); err != nil {
		return noRoute, err
	}
	encodedConfig, err := outputadapter.EncodeYokaKitConfig(config)
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
		SELECT kind FROM semantic_observations_v2
		WHERE definition_id = ? ORDER BY observation_row_id DESC LIMIT 1
	`, definitionID).Scan(&semanticKind); errors.Is(err, sql.ErrNoRows) {
		var specJSON []byte
		if err := tx.QueryRowContext(ctx, `
			SELECT spec_json FROM semantic_definitions_v2
			WHERE definition_id = ? AND active = 1
		`, definitionID).Scan(&specJSON); errors.Is(err, sql.ErrNoRows) {
			return noRoute, siteapp.ErrNotFound
		} else if err != nil {
			return noRoute, err
		}
		var spec semantics.DefinitionSpec
		if err := json.Unmarshal(specJSON, &spec); err != nil {
			return noRoute, err
		}
		semanticKind = spec.Kind
	} else if err != nil {
		return noRoute, err
	}
	outputKind, err := outputObservationKind(semanticKind)
	if err != nil {
		return noRoute, err
	}
	if err := (outputadapter.YokaKitAdapter{}).ValidateConfig(
		encodedConfig,
		outputKind,
	); err != nil {
		return noRoute, err
	}
	var start int64
	if err := tx.QueryRowContext(ctx,
		`SELECT COALESCE(MAX(observation_row_id), 0) FROM semantic_observations_v2`,
	).Scan(&start); err != nil {
		return noRoute, err
	}
	routeID, err := newResourceRef("out_")
	if err != nil {
		return noRoute, err
	}
	route := YokaKitRoute{
		RouteID: routeID, DefinitionID: definitionID,
		SourceID: config.SourceID, SignalID: config.SignalID,
		Kind: config.Kind, Reason: config.Reason,
		StartAfterObservationRowID: start, Active: true,
		CreatedAt: time.Now().UnixMilli(),
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO yokakit_routes (
			route_id, definition_id, source_id, signal_id, kind, reason,
			start_after_observation_row_id, active, created_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?)
	`, route.RouteID, route.DefinitionID, route.SourceID, route.SignalID,
		route.Kind, route.Reason, route.StartAfterObservationRowID,
		route.CreatedAt); err != nil {
		return noRoute, err
	}
	summary, _ := json.Marshal(struct {
		SourceID string                    `json:"source_id"`
		SignalID string                    `json:"signal_id"`
		Kind     outputadapter.YokaKitKind `json:"kind"`
	}{route.SourceID, route.SignalID, route.Kind})
	if err := insertAuditEventTx(ctx, tx, siteapp.AuditEvent{
		OccurredAt: route.CreatedAt, ActorClass: actor.Class, ActorRef: actor.Ref,
		Operation: "yokakit_route.create", ResourceRef: route.RouteID,
		Outcome: auditOutcomeSuccess, Summary: summary,
	}); err != nil {
		return noRoute, err
	}
	if err := tx.Commit(); err != nil {
		return noRoute, err
	}
	return route, nil
}

func (store *Store) ListYokaKitRoutes(ctx context.Context) ([]YokaKitRoute, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT route.route_id, route.definition_id, route.source_id,
			route.signal_id, route.kind, route.reason,
			route.start_after_observation_row_id, route.active, route.created_at,
			COALESCE(SUM(CASE WHEN outbox.export_id IS NOT NULL
				AND outbox.published_at IS NULL THEN 1 ELSE 0 END), 0),
			COALESCE(SUM(CASE WHEN outbox.published_at IS NOT NULL
				THEN 1 ELSE 0 END), 0)
		FROM yokakit_routes AS route
		LEFT JOIN output_outbox_v2 AS outbox ON outbox.route_id = route.route_id
		GROUP BY route.route_id
		ORDER BY route.created_at, route.route_id
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var routes []YokaKitRoute
	for rows.Next() {
		var route YokaKitRoute
		if err := rows.Scan(
			&route.RouteID, &route.DefinitionID, &route.SourceID, &route.SignalID,
			&route.Kind, &route.Reason, &route.StartAfterObservationRowID,
			&route.Active, &route.CreatedAt, &route.PendingCount,
			&route.PublishedCount,
		); err != nil {
			return nil, err
		}
		routes = append(routes, route)
	}
	return routes, rows.Err()
}

func (store *Store) ListYokaKitSourceIDs(ctx context.Context) ([]string, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT source_id FROM yokakit_routes
		WHERE active = 1
		UNION
		SELECT CAST(json_extract(config_json, '$.source_id') AS TEXT)
		FROM output_routes
		WHERE active = 1
			AND adapter_id = 'yokakit.mqtt.v1'
		ORDER BY 1
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var sourceIDs []string
	for rows.Next() {
		var sourceID string
		if err := rows.Scan(&sourceID); err != nil {
			return nil, err
		}
		sourceIDs = append(sourceIDs, sourceID)
	}
	return sourceIDs, rows.Err()
}

func (store *Store) EnqueueOutputExports(ctx context.Context, limit int) (int, error) {
	if limit < 1 || limit > 10_000 {
		return 0, errors.New("output enqueue limit must be between 1 and 10000")
	}
	rows, err := store.db.QueryContext(ctx, `
		SELECT route.route_id, route.source_id, route.signal_id, route.kind,
			route.reason, observation.observation_row_id,
			observation.observation_id, observation.series_id, observation.sequence,
			observation.definition_id, observation.definition_revision,
			observation.kind, observation.value_json, observation.signal_ref,
			observation.edge_node_id, observation.ledger_epoch,
			observation.source_pub_seq, observation.observed_at, observation.created_at
		FROM yokakit_routes AS route
		JOIN semantic_observations_v2 AS observation
			ON observation.definition_id = route.definition_id
			AND observation.observation_row_id > route.start_after_observation_row_id
		WHERE route.active = 1 AND NOT EXISTS (
			SELECT 1 FROM output_outbox_v2 AS outbox
			WHERE outbox.route_id = route.route_id
				AND outbox.observation_id = observation.observation_id
		)
		ORDER BY observation.observation_row_id, route.route_id
		LIMIT ?
	`, limit)
	if err != nil {
		return 0, err
	}
	type candidate struct {
		routeID, sourceID, signalID, reason string
		kind                                outputadapter.YokaKitKind
		rowID                               int64
		observation                         semantics.Observation
	}
	var candidates []candidate
	for rows.Next() {
		var item candidate
		var value []byte
		if err := rows.Scan(
			&item.routeID, &item.sourceID, &item.signalID, &item.kind,
			&item.reason, &item.rowID, &item.observation.ObservationID,
			&item.observation.SeriesID, &item.observation.Sequence,
			&item.observation.DefinitionID, &item.observation.DefinitionRevision,
			&item.observation.Kind, &value, &item.observation.SignalRef,
			&item.observation.EdgeNodeID, &item.observation.LedgerEpoch,
			&item.observation.SourcePubSeq, &item.observation.ObservedAt,
			&item.observation.CreatedAt,
		); err != nil {
			_ = rows.Close()
			return 0, err
		}
		item.observation.RowID = item.rowID
		item.observation.Value = value
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
	for _, item := range candidates {
		config, err := outputadapter.EncodeYokaKitConfig(
			outputadapter.YokaKitConfig{
				SourceID: item.sourceID, SignalID: item.signalID,
				Kind: item.kind, Reason: item.reason,
			},
		)
		if err != nil {
			return 0, err
		}
		observation, err := outputObservation(item.observation)
		if err != nil {
			return 0, err
		}
		message, err := (outputadapter.YokaKitAdapter{}).Transform(
			config,
			observation,
		)
		if err != nil {
			return 0, err
		}
		result, err := tx.ExecContext(ctx, `
			INSERT OR IGNORE INTO output_outbox_v2 (
				export_id, route_id, observation_id, topic, qos,
				payload_json, created_at
			) VALUES (?, ?, ?, ?, ?, ?, ?)
		`, mqttExportID(item.routeID, item.observation.ObservationID),
			item.routeID, item.observation.ObservationID, message.Topic,
			message.QoS, message.Payload, time.Now().UnixMilli())
		if err != nil {
			return 0, err
		}
		count, _ := result.RowsAffected()
		inserted += int(count)
	}
	if err := tx.Commit(); err != nil {
		return 0, err
	}
	return inserted, nil
}
