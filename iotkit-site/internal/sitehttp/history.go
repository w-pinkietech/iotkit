package sitehttp

import (
	"encoding/base64"
	"encoding/csv"
	"encoding/json"
	"errors"
	"net/http"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/store"
)

const maxHistoryExportRows = 100_000

type historyAPIPage struct {
	Records    []store.HistoryRecord `json:"records"`
	HasMore    bool                  `json:"has_more"`
	NextCursor string                `json:"next_cursor,omitempty"`
}

func parseHistoryRequest(request *http.Request, defaultLimit int) (store.HistoryQuery, error) {
	query := request.URL.Query()
	from, err := strconv.ParseInt(query.Get("from"), 10, 64)
	if err != nil {
		return store.HistoryQuery{}, errors.New("invalid history from")
	}
	until, err := strconv.ParseInt(query.Get("to"), 10, 64)
	if err != nil {
		return store.HistoryQuery{}, errors.New("invalid history to")
	}
	limit := defaultLimit
	if raw := query.Get("limit"); raw != "" {
		limit, err = strconv.Atoi(raw)
		if err != nil {
			return store.HistoryQuery{}, errors.New("invalid history limit")
		}
		if defaultLimit != maxHistoryExportRows && (limit < 1 || limit > 1_000) {
			return store.HistoryQuery{}, errors.New("invalid history limit")
		}
	}
	result := store.HistoryQuery{
		SignalRef: query.Get("signal_ref"), EdgeNodeID: query.Get("edge_node_id"),
		From: from, Until: until, Limit: limit,
	}
	if raw := query.Get("cursor"); raw != "" {
		cursor, err := decodeHistoryCursor(raw)
		if err != nil {
			return store.HistoryQuery{}, err
		}
		result.Before = &cursor
	}
	return result, nil
}

func encodeHistoryCursor(cursor store.HistoryCursor) (string, error) {
	payload, err := json.Marshal(cursor)
	if err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(payload), nil
}

func decodeHistoryCursor(raw string) (store.HistoryCursor, error) {
	var cursor store.HistoryCursor
	payload, err := base64.RawURLEncoding.DecodeString(raw)
	if err != nil {
		return cursor, errors.New("invalid history cursor")
	}
	if err := json.Unmarshal(payload, &cursor); err != nil ||
		cursor.ReceivedAt < 0 || cursor.EdgeNodeID == "" ||
		cursor.LedgerEpoch == "" || cursor.PubSeq < 1 {
		return store.HistoryCursor{}, errors.New("invalid history cursor")
	}
	return cursor, nil
}

func (server *Server) listHistory(response http.ResponseWriter, request *http.Request) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	query, err := parseHistoryRequest(request, 200)
	if err != nil {
		server.badRequest(response)
		return
	}
	page, err := server.store.QueryHistory(request.Context(), query)
	if err != nil {
		server.operationError(response, err)
		return
	}
	result := historyAPIPage{Records: page.Records, HasMore: page.HasMore}
	if page.Next != nil {
		result.NextCursor, err = encodeHistoryCursor(*page.Next)
		if err != nil {
			server.operationError(response, err)
			return
		}
	}
	writeJSON(response, http.StatusOK, result)
}

func (server *Server) getHistorySeries(response http.ResponseWriter, request *http.Request) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	query := request.URL.Query()
	from, fromErr := strconv.ParseInt(query.Get("from"), 10, 64)
	until, untilErr := strconv.ParseInt(query.Get("to"), 10, 64)
	bucket, bucketErr := strconv.ParseInt(query.Get("bucket_ms"), 10, 64)
	if fromErr != nil || untilErr != nil || bucketErr != nil {
		server.badRequest(response)
		return
	}
	series, err := server.store.QueryHistorySeries(request.Context(), store.HistorySeriesQuery{
		SignalRef: query.Get("signal_ref"), From: from, Until: until,
		BucketMilliseconds: bucket,
	})
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, series)
}

func (server *Server) exportHistoryCSV(response http.ResponseWriter, request *http.Request) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	query, err := parseHistoryRequest(request, maxHistoryExportRows)
	if err != nil || query.Before != nil {
		server.badRequest(response)
		return
	}
	query.Limit = maxHistoryExportRows
	page, err := server.store.QueryHistory(request.Context(), query)
	if err != nil {
		server.operationError(response, err)
		return
	}
	if page.HasMore {
		server.writeError(response, http.StatusUnprocessableEntity, "history_export_too_large",
			"CSVの件数が多すぎます。期間またはセンサーを絞り込んでください。", nil)
		return
	}

	response.Header().Set("Content-Type", "text/csv; charset=utf-8")
	response.Header().Set("Content-Disposition", `attachment; filename="iotkit-history.csv"`)
	response.WriteHeader(http.StatusOK)
	_, _ = response.Write([]byte{0xef, 0xbb, 0xbf})
	writer := csv.NewWriter(response)
	_ = writer.Write([]string{
		"received_at", "observed_at", "edge_node_id", "signal_ref",
		"series_key", "sensor_name", "values", "unit",
	})
	for _, record := range page.Records {
		_ = writer.Write([]string{
			formatHistoryTime(record.ReceivedAt), formatHistoryTime(record.ObservedAt),
			csvSafeCell(record.EdgeNodeID), csvSafeCell(record.SignalRef),
			csvSafeCell(record.SeriesKey), csvSafeCell(record.DisplayName),
			csvSafeCell(string(record.Values)), csvSafeCell(record.Unit),
		})
	}
	writer.Flush()
}

func (server *Server) exportSemanticHistoryCSV(
	response http.ResponseWriter,
	request *http.Request,
) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	rawQuery, err := parseHistoryRequest(request, maxHistoryExportRows)
	if err != nil || rawQuery.Before != nil {
		server.badRequest(response)
		return
	}
	page, err := server.store.QuerySemanticHistory(
		request.Context(),
		store.SemanticHistoryQuery{
			SignalRef: rawQuery.SignalRef, EdgeNodeID: rawQuery.EdgeNodeID,
			From: rawQuery.From, Until: rawQuery.Until,
			Limit: maxHistoryExportRows,
		},
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeSemanticHistoryCSV(response, page)
}

func writeSemanticHistoryCSV(
	response http.ResponseWriter,
	page store.SemanticHistoryPage,
) {
	if page.HasMore {
		writeJSON(response, http.StatusUnprocessableEntity, errorEnvelope{Error: apiError{
			Code:      "semantic_history_export_too_large",
			Message:   "加工後CSVの件数が多すぎます。期間またはセンサーを絞り込んでください。",
			RequestID: newRequestID(),
		}})
		return
	}

	response.Header().Set("Content-Type", "text/csv; charset=utf-8")
	response.Header().Set(
		"Content-Disposition",
		`attachment; filename="iotkit-processed-history.csv"`,
	)
	response.WriteHeader(http.StatusOK)
	_, _ = response.Write([]byte{0xef, 0xbb, 0xbf})
	writer := csv.NewWriter(response)
	_ = writer.Write([]string{
		"observed_at", "processed_at", "edge_node_id", "signal_ref",
		"sensor_name", "rule_name", "kind", "value", "unit", "series_id",
		"sequence", "observation_id", "rule_revision", "calibration_revision",
		"source_pub_seq",
	})
	for _, record := range page.Records {
		_ = writer.Write([]string{
			formatHistoryTime(record.ObservedAt), formatHistoryTime(record.ProcessedAt),
			csvSafeCell(record.EdgeNodeID), csvSafeCell(record.SignalRef),
			csvSafeCell(record.SensorName), csvSafeCell(record.RuleName),
			csvSafeCell(record.Kind), csvSafeCell(string(record.Value)),
			csvSafeCell(record.Unit), csvSafeCell(record.SeriesID),
			strconv.FormatInt(record.Sequence, 10), csvSafeCell(record.ObservationID),
			strconv.FormatInt(record.RuleRevision, 10),
			strconv.FormatInt(record.CalibrationRevision, 10),
			strconv.FormatInt(record.SourcePubSeq, 10),
		})
	}
	writer.Flush()
}

func formatHistoryTime(milliseconds int64) string {
	if milliseconds <= 0 {
		return ""
	}
	return time.UnixMilli(milliseconds).UTC().Format(time.RFC3339Nano)
}

func csvSafeCell(value string) string {
	trimmed := strings.TrimLeft(value, " \t\r\n")
	if trimmed == "" {
		return value
	}
	first, _ := utf8.DecodeRuneInString(trimmed)
	if first == '=' || first == '+' || first == '-' || first == '@' {
		return "'" + value
	}
	return value
}
