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

func (store *Store) PutMQTTRoute(ctx context.Context, mappingID, topic string) (MQTTRoute, error) {
	var noRoute MQTTRoute
	if err := (MQTTRouteSpec{MappingID: mappingID, Topic: topic}).Validate(); err != nil {
		return noRoute, err
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noRoute, err
	}
	defer func() { _ = tx.Rollback() }()

	existing, err := readMQTTRoute(ctx, tx, mappingID, topic)
	if err == nil {
		if err := tx.Commit(); err != nil {
			return noRoute, err
		}
		return existing, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return noRoute, err
	}

	var mappingExists int
	if err := tx.QueryRowContext(ctx, `
		SELECT EXISTS (SELECT 1 FROM semantic_mappings WHERE mapping_id = ?)
	`, mappingID).Scan(&mappingExists); err != nil {
		return noRoute, err
	}
	if mappingExists != 1 {
		return noRoute, fmt.Errorf("semantic mapping %q does not exist", mappingID)
	}

	var startAfterEventRowID int64
	if err := tx.QueryRowContext(ctx, `
		SELECT COALESCE(MAX(event_row_id), 0)
		FROM semantic_events
		WHERE mapping_id = ?
	`, mappingID).Scan(&startAfterEventRowID); err != nil {
		return noRoute, err
	}
	routeID, err := generateMQTTRouteID()
	if err != nil {
		return noRoute, err
	}
	createdAt := time.Now().UnixMilli()
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO mqtt_routes (
			route_id, mapping_id, topic, qos, start_after_event_row_id, active, created_at
		) VALUES (?, ?, ?, ?, ?, 1, ?)
	`, routeID, mappingID, topic, mqttExportQoS, startAfterEventRowID, createdAt); err != nil {
		return noRoute, err
	}
	if err := tx.Commit(); err != nil {
		return noRoute, err
	}
	return MQTTRoute{
		RouteID:              routeID,
		MappingID:            mappingID,
		Topic:                topic,
		QoS:                  mqttExportQoS,
		StartAfterEventRowID: startAfterEventRowID,
		Active:               true,
		CreatedAt:            createdAt,
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
		)
		SELECT export_id, route_id, event_id, topic, qos, payload_json, attempts, created_at
		FROM ranked_pending
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
	result, err := store.db.ExecContext(ctx, `
		UPDATE mqtt_export_outbox
		SET published_at = COALESCE(published_at, ?)
		WHERE export_id = ?
	`, time.Now().UnixMilli(), exportID)
	if err != nil {
		return err
	}
	rowsAffected, err := result.RowsAffected()
	if err != nil {
		return err
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
