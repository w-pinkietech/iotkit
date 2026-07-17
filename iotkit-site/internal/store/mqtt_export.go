package store

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/applicationcontract"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

const mqttExportQoS = 1

type MQTTRoute struct {
	RouteID              string `json:"route_id"`
	MappingID            string `json:"mapping_id"`
	Topic                string `json:"topic"`
	QoS                  byte   `json:"qos"`
	StartAfterEventRowID int64  `json:"start_after_event_row_id"`
	Active               bool   `json:"active"`
	CreatedAt            int64  `json:"created_at"`
}

type MQTTRouteSpec struct {
	MappingID string
	Topic     string
}

type MQTTRouteStatus struct {
	MQTTRoute
	PendingCount    int64  `json:"pending_count"`
	PublishedCount  int64  `json:"published_count"`
	OldestPendingAt *int64 `json:"oldest_pending_at"`
}

func (spec MQTTRouteSpec) Validate() error {
	if strings.TrimSpace(spec.MappingID) == "" {
		return errors.New("mapping ID must be non-empty")
	}
	return validateMQTTTopic(spec.Topic)
}

type PendingMQTTExport struct {
	ExportID    string          `json:"export_id"`
	RouteID     string          `json:"route_id"`
	EventID     string          `json:"event_id"`
	Topic       string          `json:"topic"`
	QoS         byte            `json:"qos"`
	PayloadJSON json.RawMessage `json:"payload_json"`
	Attempts    int64           `json:"attempts"`
	CreatedAt   int64           `json:"created_at"`
}

func putMQTTRouteTx(ctx context.Context, tx *sql.Tx, mappingID, topic string) (MQTTRoute, bool, error) {
	var noRoute MQTTRoute
	existing, err := readMQTTRoute(ctx, tx, mappingID, topic)
	if err == nil {
		return existing, false, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return noRoute, false, err
	}

	var mappingExists int
	if err := tx.QueryRowContext(ctx, `
		SELECT EXISTS (SELECT 1 FROM semantic_mappings WHERE mapping_id = ?)
	`, mappingID).Scan(&mappingExists); err != nil {
		return noRoute, false, err
	}
	if mappingExists != 1 {
		return noRoute, false, fmt.Errorf("semantic mapping %q does not exist", mappingID)
	}

	var startAfterEventRowID int64
	if err := tx.QueryRowContext(ctx, `
		SELECT COALESCE(MAX(event_row_id), 0)
		FROM semantic_events
		WHERE mapping_id = ?
	`, mappingID).Scan(&startAfterEventRowID); err != nil {
		return noRoute, false, err
	}
	routeID, err := generateMQTTRouteID()
	if err != nil {
		return noRoute, false, err
	}
	createdAt := time.Now().UnixMilli()
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO mqtt_routes (
			route_id, mapping_id, topic, qos, start_after_event_row_id, active, created_at
		) VALUES (?, ?, ?, ?, ?, 1, ?)
	`, routeID, mappingID, topic, mqttExportQoS, startAfterEventRowID, createdAt); err != nil {
		return noRoute, false, err
	}
	return MQTTRoute{
		RouteID:              routeID,
		MappingID:            mappingID,
		Topic:                topic,
		QoS:                  mqttExportQoS,
		StartAfterEventRowID: startAfterEventRowID,
		Active:               true,
		CreatedAt:            createdAt,
	}, true, nil
}

func (store *Store) ApplyLegacyMQTTRoute(
	ctx context.Context,
	actor siteapp.Actor,
	mappingID string,
	topic string,
) (siteapp.LegacyMQTTRoute, error) {
	var noRoute siteapp.LegacyMQTTRoute
	if err := actor.Validate(); err != nil {
		return noRoute, err
	}
	if err := (MQTTRouteSpec{MappingID: mappingID, Topic: topic}).Validate(); err != nil {
		return noRoute, err
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noRoute, err
	}
	defer func() { _ = tx.Rollback() }()
	route, created, err := putMQTTRouteTx(ctx, tx, mappingID, topic)
	if err != nil {
		return noRoute, err
	}
	summary, err := json.Marshal(struct {
		MappingID string `json:"mapping_id"`
		Topic     string `json:"topic"`
		QoS       byte   `json:"qos"`
		Created   bool   `json:"created"`
	}{
		MappingID: route.MappingID,
		Topic:     route.Topic,
		QoS:       route.QoS,
		Created:   created,
	})
	if err != nil {
		return noRoute, err
	}
	if err := insertAuditEventTx(ctx, tx, siteapp.AuditEvent{
		OccurredAt:  time.Now().UnixMilli(),
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "legacy_mqtt_route.put",
		ResourceRef: route.RouteID,
		Outcome:     auditOutcomeSuccess,
		Summary:     summary,
	}); err != nil {
		return noRoute, err
	}
	if err := tx.Commit(); err != nil {
		return noRoute, err
	}
	return siteapp.LegacyMQTTRoute{
		RouteID:              route.RouteID,
		MappingID:            route.MappingID,
		Topic:                route.Topic,
		QoS:                  int(route.QoS),
		StartAfterEventRowID: route.StartAfterEventRowID,
		Active:               route.Active,
		CreatedAt:            route.CreatedAt,
	}, nil
}

func readMQTTRoute(ctx context.Context, tx *sql.Tx, mappingID, topic string) (MQTTRoute, error) {
	var route MQTTRoute
	var qos int
	err := tx.QueryRowContext(ctx, `
		SELECT route_id, mapping_id, topic, qos, start_after_event_row_id, active, created_at
		FROM mqtt_routes
		WHERE mapping_id = ? AND topic = ?
	`, mappingID, topic).Scan(
		&route.RouteID,
		&route.MappingID,
		&route.Topic,
		&qos,
		&route.StartAfterEventRowID,
		&route.Active,
		&route.CreatedAt,
	)
	route.QoS = byte(qos)
	return route, err
}

func (store *Store) ListMQTTRouteStatuses(ctx context.Context) ([]MQTTRouteStatus, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT routes.route_id, routes.mapping_id, routes.topic, routes.qos,
			routes.start_after_event_row_id, routes.active, routes.created_at,
			COALESCE(SUM(CASE
				WHEN outbox.export_id IS NOT NULL AND outbox.published_at IS NULL THEN 1
				ELSE 0
			END), 0) AS pending_count,
			COALESCE(SUM(CASE WHEN outbox.published_at IS NOT NULL THEN 1 ELSE 0 END), 0)
				AS published_count,
			MIN(CASE WHEN outbox.published_at IS NULL THEN outbox.created_at END)
				AS oldest_pending_at
		FROM mqtt_routes AS routes
		LEFT JOIN mqtt_export_outbox AS outbox ON outbox.route_id = routes.route_id
		GROUP BY routes.route_id
		ORDER BY routes.created_at, routes.route_id
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	statuses := make([]MQTTRouteStatus, 0)
	for rows.Next() {
		var status MQTTRouteStatus
		var qos int
		var oldestPendingAt sql.NullInt64
		if err := rows.Scan(
			&status.RouteID,
			&status.MappingID,
			&status.Topic,
			&qos,
			&status.StartAfterEventRowID,
			&status.Active,
			&status.CreatedAt,
			&status.PendingCount,
			&status.PublishedCount,
			&oldestPendingAt,
		); err != nil {
			return nil, err
		}
		status.QoS = byte(qos)
		if oldestPendingAt.Valid {
			status.OldestPendingAt = &oldestPendingAt.Int64
		}
		statuses = append(statuses, status)
	}
	return statuses, rows.Err()
}

func validateMQTTTopic(topic string) error {
	if strings.TrimSpace(topic) == "" {
		return errors.New("MQTT topic must be non-empty")
	}
	if strings.HasPrefix(topic, "/") || strings.HasSuffix(topic, "/") {
		return errors.New("MQTT topic must not start or end with a slash")
	}
	if strings.ContainsAny(topic, "+#") {
		return errors.New("MQTT topic must not contain wildcards")
	}
	return nil
}

type mqttExportCandidate struct {
	RouteID         string
	EventID         string
	Topic           string
	QoS             int
	MappingID       string
	MappingRevision int64
	EventSequence   int64
	Meaning         string
	EdgeNodeID      string
	SourceSeriesKey string
	SourcePubSeq    int64
	OccurredAt      int64
}

func (store *Store) EnqueueMQTTExports(ctx context.Context, limit int) (int, error) {
	if limit < 1 || limit > 10_000 {
		return 0, errors.New("MQTT export enqueue limit must be between 1 and 10000")
	}
	candidates, err := store.listMQTTExportCandidates(ctx, limit)
	if err != nil {
		return 0, err
	}
	if len(candidates) == 0 {
		return 0, nil
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return 0, err
	}
	defer func() { _ = tx.Rollback() }()

	createdAt := time.Now().UnixMilli()
	inserted := 0
	for _, candidate := range candidates {
		payload := applicationcontract.ProductionPulseV1{
			SchemaVersion:   applicationcontract.ProductionPulseSchemaVersion,
			EventID:         candidate.EventID,
			MappingID:       candidate.MappingID,
			MappingRevision: candidate.MappingRevision,
			EventSequence:   candidate.EventSequence,
			Meaning:         candidate.Meaning,
			EdgeNodeID:      candidate.EdgeNodeID,
			SourceSeriesKey: candidate.SourceSeriesKey,
			SourcePubSeq:    candidate.SourcePubSeq,
			OccurredAt:      candidate.OccurredAt,
			Count:           candidate.EventSequence,
		}
		if err := payload.Validate(); err != nil {
			return 0, fmt.Errorf("build MQTT export for event %s: %w", candidate.EventID, err)
		}
		payloadJSON, err := json.Marshal(payload)
		if err != nil {
			return 0, err
		}
		result, err := tx.ExecContext(ctx, `
			INSERT OR IGNORE INTO mqtt_export_outbox (
				export_id, route_id, event_id, topic, qos, payload_json, created_at
			) VALUES (?, ?, ?, ?, ?, ?, ?)
		`, mqttExportID(candidate.RouteID, candidate.EventID), candidate.RouteID,
			candidate.EventID, candidate.Topic, candidate.QoS, payloadJSON, createdAt)
		if err != nil {
			return 0, err
		}
		rowsAffected, err := result.RowsAffected()
		if err != nil {
			return 0, err
		}
		inserted += int(rowsAffected)
	}
	if err := tx.Commit(); err != nil {
		return 0, err
	}
	return inserted, nil
}

func (store *Store) listMQTTExportCandidates(ctx context.Context, limit int) ([]mqttExportCandidate, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT routes.route_id, events.event_id, routes.topic, routes.qos,
			events.mapping_id, events.mapping_revision, events.event_sequence,
			events.meaning, events.edge_node_id, events.source_series_key,
			events.source_pub_seq, events.occurred_at
		FROM mqtt_routes AS routes
		JOIN semantic_events AS events
			ON events.mapping_id = routes.mapping_id
			AND events.event_row_id > routes.start_after_event_row_id
		WHERE routes.active = 1
			AND NOT EXISTS (
				SELECT 1 FROM mqtt_export_outbox AS outbox
				WHERE outbox.route_id = routes.route_id
					AND outbox.event_id = events.event_id
			)
		ORDER BY events.event_row_id, routes.topic, routes.route_id
		LIMIT ?
	`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	candidates := make([]mqttExportCandidate, 0)
	for rows.Next() {
		var candidate mqttExportCandidate
		if err := rows.Scan(
			&candidate.RouteID,
			&candidate.EventID,
			&candidate.Topic,
			&candidate.QoS,
			&candidate.MappingID,
			&candidate.MappingRevision,
			&candidate.EventSequence,
			&candidate.Meaning,
			&candidate.EdgeNodeID,
			&candidate.SourceSeriesKey,
			&candidate.SourcePubSeq,
			&candidate.OccurredAt,
		); err != nil {
			return nil, err
		}
		candidates = append(candidates, candidate)
	}
	return candidates, rows.Err()
}

func (store *Store) ListPendingMQTTExports(ctx context.Context, limit int) ([]PendingMQTTExport, error) {
	if limit < 1 || limit > 10_000 {
		return nil, errors.New("pending MQTT export query limit must be between 1 and 10000")
	}
	rows, err := store.db.QueryContext(ctx, `
		WITH ranked_pending AS (
			SELECT outbox.export_id, outbox.route_id, outbox.event_id, outbox.topic,
				outbox.qos, outbox.payload_json, outbox.attempts, outbox.created_at,
				events.event_row_id,
				ROW_NUMBER() OVER (
					PARTITION BY outbox.route_id
					ORDER BY events.event_row_id, outbox.export_id
				) AS route_rank
			FROM mqtt_export_outbox AS outbox
			JOIN semantic_events AS events ON events.event_id = outbox.event_id
			WHERE outbox.published_at IS NULL
		), combined AS (
			SELECT export_id, route_id, event_id, topic, qos, payload_json,
				attempts, created_at, route_rank, event_row_id
			FROM ranked_pending
			UNION ALL
			SELECT outbox.export_id, outbox.route_id, outbox.observation_id,
				outbox.topic, outbox.qos, outbox.payload_json, outbox.attempts,
				outbox.created_at,
				ROW_NUMBER() OVER (
					PARTITION BY outbox.route_id
					ORDER BY observation.observation_row_id, outbox.export_id
				), observation.observation_row_id
			FROM output_outbox_v2 AS outbox
			JOIN semantic_observations_v2 AS observation
				ON observation.observation_id = outbox.observation_id
			WHERE outbox.published_at IS NULL
		)
		SELECT export_id, route_id, event_id, topic, qos, payload_json, attempts, created_at
		FROM combined
		ORDER BY route_rank, event_row_id, topic, route_id, export_id
		LIMIT ?
	`, limit)
	if err != nil {
		return nil, err
	}
	pending := make([]PendingMQTTExport, 0)
	for rows.Next() {
		var export PendingMQTTExport
		var qos int
		var payload []byte
		if err := rows.Scan(
			&export.ExportID,
			&export.RouteID,
			&export.EventID,
			&export.Topic,
			&qos,
			&payload,
			&export.Attempts,
			&export.CreatedAt,
		); err != nil {
			_ = rows.Close()
			return nil, err
		}
		export.QoS = byte(qos)
		export.PayloadJSON = json.RawMessage(payload)
		pending = append(pending, export)
	}
	if err := rows.Err(); err != nil {
		_ = rows.Close()
		return nil, err
	}
	return pending, rows.Close()
}

func (store *Store) MarkMQTTExportPublished(ctx context.Context, exportID string) error {
	if strings.TrimSpace(exportID) == "" {
		return errors.New("export ID must be non-empty")
	}
	now := time.Now().UnixMilli()
	result, err := store.db.ExecContext(ctx, `
		UPDATE mqtt_export_outbox SET published_at = COALESCE(published_at, ?)
		WHERE export_id = ?
	`, now, exportID)
	if err != nil {
		return err
	}
	rowsAffected, err := result.RowsAffected()
	if err != nil {
		return err
	}
	if rowsAffected == 0 {
		result, err = store.db.ExecContext(ctx, `
			UPDATE output_outbox_v2 SET published_at = COALESCE(published_at, ?)
			WHERE export_id = ?
		`, now, exportID)
		if err != nil {
			return err
		}
		rowsAffected, err = result.RowsAffected()
		if err != nil {
			return err
		}
	}
	if rowsAffected != 1 {
		return fmt.Errorf("MQTT export %q does not exist", exportID)
	}
	return nil
}

func generateMQTTRouteID() (string, error) {
	random := make([]byte, 16)
	if _, err := rand.Read(random); err != nil {
		return "", err
	}
	return "mr-" + hex.EncodeToString(random), nil
}

func mqttExportID(routeID, eventID string) string {
	digest := sha256.New()
	_, _ = fmt.Fprintf(digest, "%s\x00%s", routeID, eventID)
	return hex.EncodeToString(digest.Sum(nil))
}
