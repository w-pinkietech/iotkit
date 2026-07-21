package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

const (
	maxHistoryRangeMilliseconds = int64(31 * 24 * 60 * 60 * 1000)
	maxHistoryQueryRows         = 100_000
	maxHistorySeriesBuckets     = 1_000
)

type HistoryCursor struct {
	ReceivedAt  int64  `json:"received_at"`
	EdgeNodeID  string `json:"edge_node_id"`
	LedgerEpoch string `json:"ledger_epoch"`
	PubSeq      int64  `json:"pub_seq"`
}

type HistoryQuery struct {
	SignalRef  string
	EdgeNodeID string
	From       int64
	Until      int64
	Limit      int
	Before     *HistoryCursor
}

type HistoryRecord struct {
	SignalRef        string          `json:"signal_ref"`
	SeriesKey        string          `json:"series_key"`
	EdgeNodeID       string          `json:"edge_node_id"`
	LedgerEpoch      string          `json:"ledger_epoch"`
	PubSeq           int64           `json:"pub_seq"`
	ReceivedAt       int64           `json:"received_at"`
	ObservedAt       int64           `json:"observed_at"`
	Values           json.RawMessage `json:"values"`
	ValueType        string          `json:"value_type"`
	Unit             string          `json:"unit"`
	DisplayName      string          `json:"display_name"`
	DecimalPlaces    int             `json:"decimal_places"`
	DisplayValueKind string          `json:"display_value_kind"`
	Cursor           HistoryCursor   `json:"-"`
}

type HistoryPage struct {
	Records []HistoryRecord `json:"records"`
	HasMore bool            `json:"has_more"`
	Next    *HistoryCursor  `json:"-"`
}

type HistorySeriesQuery struct {
	SignalRef          string
	From               int64
	Until              int64
	BucketMilliseconds int64
}

type HistorySeriesPoint struct {
	BucketStart int64   `json:"bucket_start"`
	Minimum     float64 `json:"minimum"`
	Average     float64 `json:"average"`
	Maximum     float64 `json:"maximum"`
	SampleCount int64   `json:"sample_count"`
}

type HistorySeries struct {
	SignalRef   string               `json:"signal_ref"`
	DisplayName string               `json:"display_name"`
	Unit        string               `json:"unit"`
	ValueType   string               `json:"value_type"`
	SampleCount int64                `json:"sample_count"`
	Points      []HistorySeriesPoint `json:"points"`
}

type SemanticHistoryQuery struct {
	SignalRef  string
	EdgeNodeID string
	From       int64
	Until      int64
	Limit      int
}

type SemanticHistoryRecord struct {
	ObservationID       string          `json:"observation_id"`
	SeriesID            string          `json:"series_id"`
	Sequence            int64           `json:"sequence"`
	RuleID              string          `json:"rule_id"`
	RuleName            string          `json:"rule_name"`
	RuleRevision        int64           `json:"rule_revision"`
	CalibrationRevision int64           `json:"calibration_revision"`
	Kind                string          `json:"kind"`
	Value               json.RawMessage `json:"value"`
	SignalRef           string          `json:"signal_ref"`
	SensorName          string          `json:"sensor_name"`
	Unit                string          `json:"unit"`
	EdgeNodeID          string          `json:"edge_node_id"`
	SourcePubSeq        int64           `json:"source_pub_seq"`
	ObservedAt          int64           `json:"observed_at"`
	ProcessedAt         int64           `json:"processed_at"`
}

type SemanticHistoryPage struct {
	Records []SemanticHistoryRecord
	HasMore bool
}

type historyPayload struct {
	EventTime int64           `json:"event_time"`
	Values    json.RawMessage `json:"values"`
}

func validateHistoryRange(from, until int64) error {
	if from < 0 || until <= from {
		return errors.New("history range must be positive and non-empty")
	}
	if until-from > maxHistoryRangeMilliseconds {
		return errors.New("history range must not exceed 31 days")
	}
	return nil
}

func (store *Store) QueryHistory(ctx context.Context, query HistoryQuery) (HistoryPage, error) {
	if err := validateHistoryRange(query.From, query.Until); err != nil {
		return HistoryPage{}, err
	}
	if query.Limit < 1 || query.Limit > maxHistoryQueryRows {
		return HistoryPage{}, fmt.Errorf("history query limit must be between 1 and %d", maxHistoryQueryRows)
	}

	statement := `
		SELECT signal.signal_ref, signal.series_key,
			raw.edge_node_id, raw.ledger_epoch, raw.pub_seq,
			raw.received_at, raw.record_json,
			COALESCE(descriptor.value_type, ''),
			CASE
				WHEN profile.display_unit_mode IN ('hidden', 'dimensionless') THEN ''
				WHEN profile.display_unit_mode = 'custom' THEN profile.display_unit
				ELSE COALESCE(descriptor.unit, '')
			END,
			COALESCE(NULLIF(profile.display_name, ''), descriptor.measurement_key, signal.series_key),
			CASE WHEN profile.revision IS NULL THEN -1 ELSE profile.decimal_places END,
			COALESCE(profile.display_value_kind, '')
		FROM raw_records AS raw
		JOIN site_signals AS signal
			ON signal.edge_node_id = raw.edge_node_id
			AND signal.series_key = json_extract(raw.record_json, '$.series_key')
		LEFT JOIN descriptor_signals AS descriptor
			ON descriptor.edge_node_id = signal.edge_node_id
			AND descriptor.series_key = signal.series_key
		LEFT JOIN signal_profiles AS profile
			ON profile.edge_node_id = signal.edge_node_id
			AND profile.series_key = signal.series_key
		WHERE raw.received_at >= ? AND raw.received_at < ?`
	arguments := []any{query.From, query.Until}
	if query.SignalRef != "" {
		statement += " AND signal.signal_ref = ?"
		arguments = append(arguments, query.SignalRef)
	}
	if query.EdgeNodeID != "" {
		statement += " AND raw.edge_node_id = ?"
		arguments = append(arguments, query.EdgeNodeID)
	}
	if query.Before != nil {
		statement += ` AND (raw.received_at, raw.edge_node_id, raw.ledger_epoch, raw.pub_seq)
			< (?, ?, ?, ?)`
		arguments = append(arguments, query.Before.ReceivedAt, query.Before.EdgeNodeID,
			query.Before.LedgerEpoch, query.Before.PubSeq)
	}
	statement += ` ORDER BY raw.received_at DESC, raw.edge_node_id DESC,
		raw.ledger_epoch DESC, raw.pub_seq DESC LIMIT ?`
	arguments = append(arguments, query.Limit+1)

	rows, err := store.db.QueryContext(ctx, statement, arguments...)
	if err != nil {
		return HistoryPage{}, err
	}
	defer rows.Close()

	page := HistoryPage{Records: make([]HistoryRecord, 0, query.Limit)}
	for rows.Next() {
		var record HistoryRecord
		var payloadBytes []byte
		if err := rows.Scan(
			&record.SignalRef, &record.SeriesKey, &record.EdgeNodeID,
			&record.LedgerEpoch, &record.PubSeq, &record.ReceivedAt,
			&payloadBytes, &record.ValueType, &record.Unit, &record.DisplayName,
			&record.DecimalPlaces, &record.DisplayValueKind,
		); err != nil {
			return HistoryPage{}, err
		}
		var payload historyPayload
		if err := json.Unmarshal(payloadBytes, &payload); err != nil {
			return HistoryPage{}, fmt.Errorf("decode stored history record: %w", err)
		}
		record.ObservedAt = payload.EventTime
		record.Values = append(json.RawMessage(nil), payload.Values...)
		record.Cursor = HistoryCursor{
			ReceivedAt: record.ReceivedAt, EdgeNodeID: record.EdgeNodeID,
			LedgerEpoch: record.LedgerEpoch, PubSeq: record.PubSeq,
		}
		page.Records = append(page.Records, record)
	}
	if err := rows.Err(); err != nil {
		return HistoryPage{}, err
	}
	if len(page.Records) > query.Limit {
		page.HasMore = true
		page.Records = page.Records[:query.Limit]
	}
	if page.HasMore && len(page.Records) > 0 {
		next := page.Records[len(page.Records)-1].Cursor
		page.Next = &next
	}
	return page, nil
}

func (store *Store) QuerySemanticHistory(
	ctx context.Context,
	query SemanticHistoryQuery,
) (SemanticHistoryPage, error) {
	if err := validateHistoryRange(query.From, query.Until); err != nil {
		return SemanticHistoryPage{}, err
	}
	if query.Limit < 1 || query.Limit > maxHistoryQueryRows {
		return SemanticHistoryPage{}, fmt.Errorf(
			"semantic history query limit must be between 1 and %d",
			maxHistoryQueryRows,
		)
	}

	statement := `
		SELECT observation.observation_id, observation.series_id,
			observation.sequence, observation.rule_id, rule.display_name,
			observation.rule_revision, observation.calibration_revision,
			observation.kind, observation.value_json,
			observation.signal_ref,
			COALESCE(
				NULLIF(profile.display_name, ''),
				descriptor.measurement_key,
				observation.signal_ref
			),
			CASE
				WHEN observation.kind != 'numeric' THEN ''
				WHEN profile.display_unit_mode IN ('hidden', 'dimensionless') THEN ''
				WHEN profile.display_unit_mode = 'custom' THEN profile.display_unit
				ELSE COALESCE(descriptor.unit, '')
			END,
			observation.edge_node_id, observation.source_pub_seq,
			observation.observed_at, observation.created_at
		FROM semantic_observations_v3 AS observation
		JOIN semantic_rules_v3 AS rule ON rule.rule_id = observation.rule_id
		LEFT JOIN site_signals AS signal
			ON signal.signal_ref = observation.signal_ref
		LEFT JOIN descriptor_signals AS descriptor
			ON descriptor.edge_node_id = signal.edge_node_id
			AND descriptor.series_key = signal.series_key
		LEFT JOIN signal_profiles AS profile
			ON profile.edge_node_id = signal.edge_node_id
			AND profile.series_key = signal.series_key
		WHERE observation.observed_at >= ? AND observation.observed_at < ?`
	arguments := []any{query.From, query.Until}
	if query.SignalRef != "" {
		statement += " AND observation.signal_ref = ?"
		arguments = append(arguments, query.SignalRef)
	}
	if query.EdgeNodeID != "" {
		statement += " AND observation.edge_node_id = ?"
		arguments = append(arguments, query.EdgeNodeID)
	}
	statement += `
		ORDER BY observation.observed_at DESC,
			observation.observation_row_id DESC
		LIMIT ?`
	arguments = append(arguments, query.Limit+1)

	rows, err := store.db.QueryContext(ctx, statement, arguments...)
	if err != nil {
		return SemanticHistoryPage{}, err
	}
	defer rows.Close()

	page := SemanticHistoryPage{
		Records: make([]SemanticHistoryRecord, 0, query.Limit),
	}
	for rows.Next() {
		var record SemanticHistoryRecord
		var value []byte
		if err := rows.Scan(
			&record.ObservationID, &record.SeriesID, &record.Sequence,
			&record.RuleID, &record.RuleName, &record.RuleRevision,
			&record.CalibrationRevision, &record.Kind, &value,
			&record.SignalRef, &record.SensorName, &record.Unit,
			&record.EdgeNodeID, &record.SourcePubSeq,
			&record.ObservedAt, &record.ProcessedAt,
		); err != nil {
			return SemanticHistoryPage{}, err
		}
		record.Value = append(json.RawMessage(nil), value...)
		page.Records = append(page.Records, record)
	}
	if err := rows.Err(); err != nil {
		return SemanticHistoryPage{}, err
	}
	if len(page.Records) > query.Limit {
		page.HasMore = true
		page.Records = page.Records[:query.Limit]
	}
	return page, nil
}

func (store *Store) QueryHistorySeries(
	ctx context.Context,
	query HistorySeriesQuery,
) (HistorySeries, error) {
	if query.SignalRef == "" {
		return HistorySeries{}, errors.New("history series requires signal_ref")
	}
	if err := validateHistoryRange(query.From, query.Until); err != nil {
		return HistorySeries{}, err
	}
	if query.BucketMilliseconds < 1 ||
		(query.Until-query.From+query.BucketMilliseconds-1)/query.BucketMilliseconds > maxHistorySeriesBuckets {
		return HistorySeries{}, errors.New("history series bucket count must be between 1 and 1000")
	}

	var edgeNodeID, seriesKey string
	var displayName, unit, valueType sql.NullString
	if err := store.db.QueryRowContext(ctx, `
		SELECT signal.edge_node_id, signal.series_key,
			COALESCE(NULLIF(profile.display_name, ''), descriptor.measurement_key, signal.series_key),
			CASE
				WHEN profile.display_unit_mode IN ('hidden', 'dimensionless') THEN ''
				WHEN profile.display_unit_mode = 'custom' THEN profile.display_unit
				ELSE COALESCE(descriptor.unit, '')
			END,
			COALESCE(descriptor.value_type, '')
		FROM site_signals AS signal
		LEFT JOIN descriptor_signals AS descriptor
			ON descriptor.edge_node_id = signal.edge_node_id
			AND descriptor.series_key = signal.series_key
		LEFT JOIN signal_profiles AS profile
			ON profile.edge_node_id = signal.edge_node_id
			AND profile.series_key = signal.series_key
		WHERE signal.signal_ref = ?
	`, query.SignalRef).Scan(&edgeNodeID, &seriesKey, &displayName, &unit, &valueType); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return HistorySeries{}, siteapp.ErrNotFound
		}
		return HistorySeries{}, err
	}

	rows, err := store.db.QueryContext(ctx, `
		SELECT CAST((raw.received_at - ?) / ? AS INTEGER) AS bucket_index,
			MIN(CAST(json_extract(raw.record_json, '$.values[0]') AS REAL)),
			AVG(CAST(json_extract(raw.record_json, '$.values[0]') AS REAL)),
			MAX(CAST(json_extract(raw.record_json, '$.values[0]') AS REAL)),
			COUNT(*)
		FROM raw_records AS raw
		WHERE raw.edge_node_id = ?
			AND json_extract(raw.record_json, '$.series_key') = ?
			AND raw.received_at >= ? AND raw.received_at < ?
			AND json_type(raw.record_json, '$.values[0]') IN ('integer', 'real')
		GROUP BY bucket_index
		ORDER BY bucket_index
	`, query.From, query.BucketMilliseconds, edgeNodeID, seriesKey, query.From, query.Until)
	if err != nil {
		return HistorySeries{}, err
	}
	defer rows.Close()

	result := HistorySeries{
		SignalRef: query.SignalRef, DisplayName: displayName.String,
		Unit: unit.String, ValueType: valueType.String,
		Points: make([]HistorySeriesPoint, 0),
	}
	for rows.Next() {
		var bucket int64
		var point HistorySeriesPoint
		if err := rows.Scan(&bucket, &point.Minimum, &point.Average,
			&point.Maximum, &point.SampleCount); err != nil {
			return HistorySeries{}, err
		}
		point.BucketStart = query.From + bucket*query.BucketMilliseconds
		result.SampleCount += point.SampleCount
		result.Points = append(result.Points, point)
	}
	if err := rows.Err(); err != nil {
		return HistorySeries{}, err
	}
	return result, nil
}
