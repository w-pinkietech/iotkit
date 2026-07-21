package contract

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"strings"
)

const (
	SchemaVersion   = 1
	MaxBatchRecords = 256
	MaxBatchBytes   = 1024 * 1024
)

type RecordBatch struct {
	SchemaVersion uint32            `json:"schema_version"`
	EdgeNodeID    string            `json:"edge_node_id"`
	LedgerEpoch   string            `json:"ledger_epoch"`
	PublicationID string            `json:"publication_id"`
	CursorStart   int64             `json:"cursor_start"`
	CursorEnd     int64             `json:"cursor_end"`
	Records       []json.RawMessage `json:"records"`
}

type AcceptedThrough struct {
	SchemaVersion   uint32 `json:"schema_version"`
	EdgeNodeID      string `json:"edge_node_id"`
	LedgerEpoch     string `json:"ledger_epoch"`
	PublicationID   string `json:"publication_id"`
	AcceptedThrough int64  `json:"accepted_through"`
}

type recordHeader struct {
	Family        string `json:"family"`
	SchemaVersion uint32 `json:"schema_version"`
	Epoch         string `json:"epoch"`
	PubSeq        int64  `json:"pub_seq"`
}

type measurementRecord struct {
	Family          string          `json:"family"`
	SchemaVersion   uint32          `json:"schema_version"`
	Epoch           string          `json:"epoch"`
	PubSeq          int64           `json:"pub_seq"`
	SeriesKey       string          `json:"series_key"`
	Values          []float64       `json:"values"`
	EventTime       *int64          `json:"event_time"`
	EventTimeSource string          `json:"event_time_source"`
	TimeSource      string          `json:"time_source"`
	TimeQuality     string          `json:"time_quality"`
	ReceivedAt      *int64          `json:"received_at"`
	DeviceTime      json.RawMessage `json:"device_time"`
}

type annotationRecord struct {
	Family        string `json:"family"`
	SchemaVersion uint32 `json:"schema_version"`
	Epoch         string `json:"epoch"`
	PubSeq        int64  `json:"pub_seq"`
	Subtype       string `json:"subtype"`
	PriorEpoch    string `json:"prior_epoch"`
}

type commissioningSmokeRecord struct {
	Family        string `json:"family"`
	SchemaVersion uint32 `json:"schema_version"`
	Epoch         string `json:"epoch"`
	PubSeq        int64  `json:"pub_seq"`
	TestID        string `json:"test_id"`
}

func DecodeBatch(payload []byte) (RecordBatch, error) {
	var batch RecordBatch
	if len(payload) > MaxBatchBytes {
		return batch, invalid("batch exceeds encoded byte limit")
	}
	if err := decodeOneStrict(payload, &batch); err != nil {
		return batch, fmt.Errorf("decode record batch: %w", err)
	}
	if err := batch.Validate(); err != nil {
		return batch, err
	}
	return batch, nil
}

func DecodeAcceptedThrough(payload []byte) (AcceptedThrough, error) {
	var ack AcceptedThrough
	if err := decodeOneStrict(payload, &ack); err != nil {
		return ack, fmt.Errorf("decode accepted-through: %w", err)
	}
	if err := ack.Validate(); err != nil {
		return ack, err
	}
	return ack, nil
}

func (batch RecordBatch) Validate() error {
	if batch.SchemaVersion != SchemaVersion {
		return invalid("unsupported schema_version")
	}
	if err := validateTopicSegment("edge_node_id", batch.EdgeNodeID); err != nil {
		return err
	}
	if err := validateIdentityComponent("ledger_epoch", batch.LedgerEpoch); err != nil {
		return err
	}
	if batch.CursorStart < 1 || batch.CursorEnd < batch.CursorStart {
		return invalid("cursor range must be positive and non-empty")
	}
	expectedCount := batch.CursorEnd - batch.CursorStart + 1
	if expectedCount != int64(len(batch.Records)) {
		return invalid("cursor range does not match record count")
	}
	if len(batch.Records) > MaxBatchRecords {
		return invalid("batch exceeds record limit")
	}
	expectedPublicationID := PublicationID(
		batch.EdgeNodeID,
		batch.LedgerEpoch,
		batch.CursorStart,
		batch.CursorEnd,
	)
	if batch.PublicationID != expectedPublicationID {
		return invalid("publication_id does not match batch identity")
	}
	for index, raw := range batch.Records {
		var header recordHeader
		if err := decodeOne(raw, &header); err != nil {
			return invalid(fmt.Sprintf("record %d header: %v", index, err))
		}
		if header.SchemaVersion != SchemaVersion {
			return invalid("record schema_version mismatch")
		}
		if header.Epoch != batch.LedgerEpoch {
			return invalid("record epoch mismatch")
		}
		if header.PubSeq != batch.CursorStart+int64(index) {
			return invalid("record pub_seq is not contiguous")
		}
		switch header.Family {
		case "measurement", "annotation", "commissioning_smoke":
		default:
			return invalid("record family is unsupported")
		}
		if err := validateRecordFamily(raw, header.Family); err != nil {
			return invalid(fmt.Sprintf("record %d: %v", index, err))
		}
	}
	encoded, err := json.Marshal(batch)
	if err != nil {
		return invalid(fmt.Sprintf("batch encoding failed: %v", err))
	}
	if len(encoded) > MaxBatchBytes {
		return invalid("batch exceeds encoded byte limit")
	}
	return nil
}

func validateRecordFamily(raw json.RawMessage, family string) error {
	switch family {
	case "measurement":
		var record measurementRecord
		if err := decodeOneStrict(raw, &record); err != nil {
			return fmt.Errorf("measurement: %w", err)
		}
		if record.SeriesKey == "" || len(record.Values) == 0 || len(record.DeviceTime) == 0 ||
			record.EventTime == nil || record.ReceivedAt == nil {
			return errors.New("measurement fields are missing")
		}
		for _, value := range record.Values {
			if math.IsNaN(value) || math.IsInf(value, 0) {
				return errors.New("measurement values must be finite")
			}
		}
		deviceTime, err := decodeNullableInt64(record.DeviceTime)
		if err != nil {
			return errors.New("measurement device_time must be an integer or null")
		}
		if !oneOf(record.TimeSource, "device_ntp", "device_rtc", "edge_node", "edge_node_adjusted") {
			return errors.New("measurement time_source is invalid")
		}
		if !oneOf(record.TimeQuality, "synced", "holdover", "unsynced") {
			return errors.New("measurement time_quality is invalid")
		}
		switch record.EventTimeSource {
		case "received_at":
			if *record.EventTime != *record.ReceivedAt {
				return errors.New("measurement received_at event_time is inconsistent")
			}
		case "device":
			if !oneOf(record.TimeSource, "device_ntp", "device_rtc") ||
				deviceTime == nil || *record.EventTime != *deviceTime {
				return errors.New("measurement device event_time is inconsistent")
			}
		case "edge_node_adjusted":
			if record.TimeSource != "edge_node_adjusted" ||
				deviceTime == nil || *record.EventTime != *deviceTime {
				return errors.New("measurement adjusted event_time is inconsistent")
			}
		default:
			return errors.New("measurement event_time_source is invalid")
		}
	case "annotation":
		var record annotationRecord
		if err := decodeOneStrict(raw, &record); err != nil {
			return fmt.Errorf("annotation: %w", err)
		}
		if record.Subtype != "epoch_start" || record.PriorEpoch == "" {
			return errors.New("annotation must be epoch_start with prior_epoch")
		}
	case "commissioning_smoke":
		var record commissioningSmokeRecord
		if err := decodeOneStrict(raw, &record); err != nil {
			return fmt.Errorf("commissioning_smoke: %w", err)
		}
		if !validCommissioningSmokeTestID(record.TestID) {
			return errors.New("commissioning_smoke test_id is invalid")
		}
	default:
		return errors.New("record family is unsupported")
	}
	return nil
}

func decodeNullableInt64(raw json.RawMessage) (*int64, error) {
	if bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
		return nil, nil
	}
	var value int64
	if err := decodeOne(raw, &value); err != nil {
		return nil, err
	}
	return &value, nil
}

func oneOf(value string, candidates ...string) bool {
	for _, candidate := range candidates {
		if value == candidate {
			return true
		}
	}
	return false
}

func validCommissioningSmokeTestID(testID string) bool {
	const prefix = "smoke-"
	if !strings.HasPrefix(testID, prefix) || len(testID) != len(prefix)+32 {
		return false
	}
	for _, character := range testID[len(prefix):] {
		if (character < '0' || character > '9') && (character < 'a' || character > 'f') {
			return false
		}
	}
	return true
}

func (ack AcceptedThrough) Validate() error {
	if ack.SchemaVersion != SchemaVersion {
		return invalid("unsupported ack schema_version")
	}
	if err := validateTopicSegment("edge_node_id", ack.EdgeNodeID); err != nil {
		return err
	}
	if err := validateIdentityComponent("ledger_epoch", ack.LedgerEpoch); err != nil {
		return err
	}
	if ack.PublicationID == "" || ack.AcceptedThrough < 1 {
		return invalid("ack correlation fields are missing")
	}
	return nil
}

func (ack AcceptedThrough) ValidateFor(batch RecordBatch, priorCursor int64) error {
	if err := batch.Validate(); err != nil {
		return err
	}
	if err := ack.Validate(); err != nil {
		return err
	}
	if ack.EdgeNodeID != batch.EdgeNodeID {
		return invalid("ack edge_node_id mismatch")
	}
	if ack.LedgerEpoch != batch.LedgerEpoch {
		return invalid("ack ledger_epoch mismatch")
	}
	if ack.PublicationID != batch.PublicationID {
		return invalid("ack publication_id mismatch")
	}
	if ack.AcceptedThrough != batch.CursorEnd {
		return invalid("ack must accept the complete initial-window batch")
	}
	if ack.AcceptedThrough <= priorCursor {
		return invalid("ack does not advance the cursor")
	}
	return nil
}

func PublicationID(edgeNodeID, ledgerEpoch string, cursorStart, cursorEnd int64) string {
	return fmt.Sprintf("%s:%s:%d:%d", edgeNodeID, ledgerEpoch, cursorStart, cursorEnd)
}

func decodeOne(payload []byte, destination any) error {
	decoder := json.NewDecoder(bytes.NewReader(payload))
	return decodeSingleValue(decoder, destination)
}

func decodeOneStrict(payload []byte, destination any) error {
	decoder := json.NewDecoder(bytes.NewReader(payload))
	decoder.DisallowUnknownFields()
	return decodeSingleValue(decoder, destination)
}

func decodeSingleValue(decoder *json.Decoder, destination any) error {
	if err := decoder.Decode(destination); err != nil {
		return err
	}
	if err := decoder.Decode(&struct{}{}); !errors.Is(err, io.EOF) {
		if err == nil {
			return errors.New("multiple JSON values")
		}
		return err
	}
	return nil
}

func validateTopicSegment(name, value string) error {
	if err := validateIdentityComponent(name, value); err != nil {
		return err
	}
	if strings.ContainsAny(value, "/+#") {
		return invalid(name + " is not a safe MQTT topic segment")
	}
	return nil
}

func validateIdentityComponent(name, value string) error {
	if value == "" || strings.Contains(value, ":") {
		return invalid(name + " is empty or contains ':'")
	}
	return nil
}

func invalid(message string) error {
	return fmt.Errorf("invalid egress v1 message: %s", message)
}
