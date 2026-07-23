package applicationcontract

import (
	"errors"
	"fmt"
	"strings"
)

const (
	ProductionPulseSchemaVersion uint32 = 1
	ProductionPulseMeaning              = "production_pulse"
)

type ProductionPulseV1 struct {
	SchemaVersion   uint32 `json:"schema_version"`
	EventID         string `json:"event_id"`
	MappingID       string `json:"mapping_id"`
	MappingRevision int64  `json:"mapping_revision"`
	EventSequence   int64  `json:"event_sequence"`
	Meaning         string `json:"meaning"`
	EdgeNodeID      string `json:"edge_node_id"`
	SourceSeriesKey string `json:"source_series_key"`
	SourcePubSeq    int64  `json:"source_pub_seq"`
	OccurredAt      int64  `json:"occurred_at"`
	Count           int64  `json:"count"`
}

func (event ProductionPulseV1) Validate() error {
	if event.SchemaVersion != ProductionPulseSchemaVersion {
		return fmt.Errorf("schema_version must be %d", ProductionPulseSchemaVersion)
	}
	if strings.TrimSpace(event.EventID) == "" {
		return errors.New("event_id must be non-empty")
	}
	if strings.TrimSpace(event.MappingID) == "" {
		return errors.New("mapping_id must be non-empty")
	}
	if event.MappingRevision < 1 {
		return errors.New("mapping_revision must be positive")
	}
	if event.EventSequence < 1 {
		return errors.New("event_sequence must be positive")
	}
	if event.Meaning != ProductionPulseMeaning {
		return fmt.Errorf("meaning must be %q", ProductionPulseMeaning)
	}
	if strings.TrimSpace(event.EdgeNodeID) == "" {
		return errors.New("edge_node_id must be non-empty")
	}
	if strings.TrimSpace(event.SourceSeriesKey) == "" {
		return errors.New("source_series_key must be non-empty")
	}
	if event.SourcePubSeq < 1 {
		return errors.New("source_pub_seq must be positive")
	}
	if event.OccurredAt < 0 {
		return errors.New("occurred_at must be non-negative")
	}
	if event.Count != event.EventSequence {
		return errors.New("count must equal event_sequence")
	}
	return nil
}
