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

type YokaKitRuleRoute = siteapp.YokaKitRuleRoute

func (store *Store) ApplyYokaKitRuleRoute(
	ctx context.Context,
	actor siteapp.Actor,
	ruleID string,
	adapter outputadapter.YokaKit,
) (YokaKitRuleRoute, error) {
	var noRoute YokaKitRuleRoute
	if err := actor.Validate(); err != nil {
		return noRoute, err
	}
	if err := adapter.Validate(); err != nil {
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
		return noRoute, siteapp.ErrNotFound
	} else if err != nil {
		return noRoute, err
	}
	probe := semantics.Observation{Kind: semanticKind, Value: json.RawMessage(`0`)}
	if semanticKind == semantics.KindBoolean || semanticKind == semantics.KindAlarm {
		probe.Value = json.RawMessage(`false`)
	}
	if _, err := adapter.Transform(probe); err != nil {
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
	route := YokaKitRuleRoute{
		RouteID: routeID, RuleID: ruleID,
		SourceID: adapter.SourceID, SignalID: adapter.SignalID,
		Kind: adapter.Kind, Reason: adapter.Reason,
		StartAfterObservationRowID: start, Active: true,
		CreatedAt: time.Now().UnixMilli(),
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO yokakit_routes_v3(
			route_id, rule_id, source_id, signal_id, kind, reason,
			start_after_observation_row_id, active, created_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?)
	`, route.RouteID, route.RuleID, route.SourceID, route.SignalID,
		route.Kind, route.Reason, route.StartAfterObservationRowID,
		route.CreatedAt); err != nil {
		return noRoute, err
	}
	summary, _ := json.Marshal(struct {
		RuleID   string                    `json:"rule_id"`
		SourceID string                    `json:"source_id"`
		SignalID string                    `json:"signal_id"`
		Kind     outputadapter.YokaKitKind `json:"kind"`
	}{route.RuleID, route.SourceID, route.SignalID, route.Kind})
	if err := insertAuditEventTx(ctx, tx, siteapp.AuditEvent{
		OccurredAt: route.CreatedAt, ActorClass: actor.Class, ActorRef: actor.Ref,
		Operation: "yokakit_rule_route.create", ResourceRef: route.RouteID,
		Outcome: auditOutcomeSuccess, Summary: summary,
	}); err != nil {
		return noRoute, err
	}
	if err := tx.Commit(); err != nil {
		return noRoute, err
	}
	return route, nil
}

func (store *Store) ListYokaKitRuleRoutes(
	ctx context.Context,
) ([]YokaKitRuleRoute, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT route.route_id, route.rule_id, route.source_id,
			route.signal_id, route.kind, route.reason,
			route.start_after_observation_row_id, route.active, route.created_at,
			COALESCE(SUM(CASE WHEN outbox.export_id IS NOT NULL
				AND outbox.published_at IS NULL THEN 1 ELSE 0 END), 0),
			COALESCE(SUM(CASE WHEN outbox.published_at IS NOT NULL
				THEN 1 ELSE 0 END), 0)
		FROM yokakit_routes_v3 AS route
		LEFT JOIN output_outbox_v3 AS outbox ON outbox.route_id = route.route_id
		GROUP BY route.route_id
		ORDER BY route.created_at, route.route_id
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var routes []YokaKitRuleRoute
	for rows.Next() {
		var route YokaKitRuleRoute
		if err := rows.Scan(
			&route.RouteID, &route.RuleID, &route.SourceID, &route.SignalID,
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

func (store *Store) EnqueueMultipleRuleOutputExports(
	ctx context.Context,
	limit int,
) (int, error) {
	if limit < 1 || limit > 10_000 {
		return 0, errors.New("output enqueue limit must be between 1 and 10000")
	}
	rows, err := store.db.QueryContext(ctx, `
		SELECT route.route_id, route.source_id, route.signal_id, route.kind,
			route.reason, observation.observation_row_id,
			observation.observation_id, observation.series_id, observation.sequence,
			observation.rule_id, observation.rule_revision, observation.kind,
			observation.value_json, observation.signal_ref,
			observation.edge_node_id, observation.ledger_epoch,
			observation.source_pub_seq, observation.observed_at,
			observation.created_at
		FROM yokakit_routes_v3 AS route
		JOIN semantic_observations_v3 AS observation
			ON observation.rule_id = route.rule_id
			AND observation.observation_row_id > route.start_after_observation_row_id
		WHERE route.active = 1 AND NOT EXISTS (
			SELECT 1 FROM output_outbox_v3 AS outbox
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
		observation                         semantics.Observation
	}
	var candidates []candidate
	for rows.Next() {
		var item candidate
		var rowID int64
		var value []byte
		if err := rows.Scan(
			&item.routeID, &item.sourceID, &item.signalID, &item.kind,
			&item.reason, &rowID, &item.observation.ObservationID,
			&item.observation.SeriesID, &item.observation.Sequence,
			&item.observation.DefinitionID,
			&item.observation.DefinitionRevision,
			&item.observation.Kind, &value, &item.observation.SignalRef,
			&item.observation.EdgeNodeID, &item.observation.LedgerEpoch,
			&item.observation.SourcePubSeq, &item.observation.ObservedAt,
			&item.observation.CreatedAt,
		); err != nil {
			_ = rows.Close()
			return 0, err
		}
		item.observation.RowID = rowID
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
		message, err := (outputadapter.YokaKit{
			SourceID: item.sourceID, SignalID: item.signalID,
			Kind: item.kind, Reason: item.reason,
		}).Transform(item.observation)
		if err != nil {
			return 0, err
		}
		result, err := tx.ExecContext(ctx, `
			INSERT OR IGNORE INTO output_outbox_v3(
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
