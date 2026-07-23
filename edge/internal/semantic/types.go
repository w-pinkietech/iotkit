package semantic

import (
	"errors"
	"strings"
)

type Meaning string

const MeaningProductionPulse Meaning = "production_pulse"

type TriggerMode string

const (
	TriggerActiveSample TriggerMode = "active_sample"
	TriggerActiveEdge   TriggerMode = "active_edge"
)

type MappingSpec struct {
	EdgeNodeID  string      `json:"edge_node_id"`
	SeriesKey   string      `json:"series_key"`
	Meaning     Meaning     `json:"meaning"`
	TriggerMode TriggerMode `json:"trigger_mode"`
	ActiveValue int         `json:"active_value"`
}

type Mapping struct {
	ID       string `json:"mapping_id"`
	Revision int64  `json:"revision"`
	MappingSpec
	Active    bool  `json:"active"`
	CreatedAt int64 `json:"created_at"`
}

func (spec MappingSpec) Validate() error {
	if strings.TrimSpace(spec.EdgeNodeID) == "" {
		return errors.New("edge_node_id must not be empty")
	}
	if strings.ContainsAny(spec.EdgeNodeID, "/+#") {
		return errors.New("edge_node_id must not contain /, +, or #")
	}
	if strings.TrimSpace(spec.SeriesKey) == "" {
		return errors.New("series_key must not be empty")
	}
	if spec.Meaning != MeaningProductionPulse {
		return errors.New("unsupported semantic meaning")
	}
	if spec.TriggerMode != TriggerActiveSample && spec.TriggerMode != TriggerActiveEdge {
		return errors.New("unsupported semantic trigger mode")
	}
	if spec.ActiveValue != 0 && spec.ActiveValue != 1 {
		return errors.New("active_value must be 0 or 1")
	}
	return nil
}
