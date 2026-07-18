package semantics

import (
	"encoding/json"
	"errors"
	"math"
)

type Kind string

const (
	KindNumeric           Kind = "numeric"
	KindBoolean           Kind = "boolean"
	KindCumulativeCounter Kind = "cumulative_counter"
	KindAlarm             Kind = "alarm"
)

type ConditionMode string

const (
	ConditionNone    ConditionMode = ""
	ConditionBoolean ConditionMode = "boolean_equals"
	ConditionAbove   ConditionMode = "above"
	ConditionBelow   ConditionMode = "below"
)

type TriggerMode string

const (
	TriggerTransition   TriggerMode = "on_transition"
	TriggerActiveSample TriggerMode = "on_notification"
)

type Condition struct {
	Mode       ConditionMode `json:"mode"`
	BoolValue  bool          `json:"bool_value"`
	Threshold  float64       `json:"threshold"`
	Hysteresis float64       `json:"hysteresis"`
}

type DefinitionSpec struct {
	Kind      Kind        `json:"kind"`
	Scale     float64     `json:"scale"`
	Offset    float64     `json:"offset"`
	Condition Condition   `json:"condition"`
	Trigger   TriggerMode `json:"trigger"`
}

func (spec DefinitionSpec) Validate() error {
	if !finite(spec.Scale) || spec.Scale == 0 {
		return errors.New("semantic scale must be a finite non-zero number")
	}
	if !finite(spec.Offset) {
		return errors.New("semantic offset must be finite")
	}
	if !finite(spec.Condition.Threshold) || !finite(spec.Condition.Hysteresis) ||
		spec.Condition.Hysteresis < 0 {
		return errors.New("semantic threshold and hysteresis must be finite and non-negative")
	}
	switch spec.Kind {
	case KindNumeric:
		if spec.Condition.Mode != ConditionNone || spec.Trigger != "" {
			return errors.New("numeric semantic definition cannot have a condition or trigger")
		}
	case KindBoolean:
		if !validCondition(spec.Condition.Mode) || spec.Condition.Mode == ConditionNone {
			return errors.New("boolean semantic definition requires a condition")
		}
		if spec.Trigger != "" {
			return errors.New("boolean semantic definition cannot have a trigger")
		}
	case KindCumulativeCounter:
		if !validCondition(spec.Condition.Mode) || spec.Condition.Mode == ConditionNone {
			return errors.New("cumulative counter requires a condition")
		}
		if spec.Trigger != TriggerTransition && spec.Trigger != TriggerActiveSample {
			return errors.New("cumulative counter requires a supported trigger")
		}
	case KindAlarm:
		if spec.Condition.Mode != ConditionAbove && spec.Condition.Mode != ConditionBelow {
			return errors.New("alarm semantic definition requires an above or below threshold")
		}
		if spec.Trigger != "" {
			return errors.New("alarm semantic definition cannot have a trigger")
		}
	default:
		return errors.New("unsupported semantic definition kind")
	}
	return nil
}

func validCondition(mode ConditionMode) bool {
	return mode == ConditionBoolean || mode == ConditionAbove || mode == ConditionBelow
}

func finite(value float64) bool {
	return !math.IsNaN(value) && !math.IsInf(value, 0)
}

type State struct {
	Initialized bool
	Active      bool
	Counter     int64
}

type Result struct {
	Emitted    bool     `json:"emitted"`
	Number     *float64 `json:"number,omitempty"`
	Boolean    *bool    `json:"boolean,omitempty"`
	Integer    *int64   `json:"integer,omitempty"`
	Calibrated float64  `json:"calibrated"`
}

type Definition struct {
	ID         string `json:"definition_id"`
	Revision   int64  `json:"revision"`
	SignalRef  string `json:"signal_ref"`
	EdgeNodeID string `json:"edge_node_id"`
	SeriesKey  string `json:"-"`
	SeriesID   string `json:"series_id"`
	DefinitionSpec
	Active    bool  `json:"active"`
	CreatedAt int64 `json:"created_at"`
}

type Observation struct {
	RowID              int64           `json:"row_id"`
	ObservationID      string          `json:"observation_id"`
	SeriesID           string          `json:"series_id"`
	Sequence           int64           `json:"sequence"`
	DefinitionID       string          `json:"definition_id"`
	DefinitionRevision int64           `json:"definition_revision"`
	Kind               Kind            `json:"kind"`
	Value              json.RawMessage `json:"value"`
	SignalRef          string          `json:"signal_ref"`
	EdgeNodeID         string          `json:"edge_node_id"`
	LedgerEpoch        string          `json:"-"`
	SourcePubSeq       int64           `json:"source_pub_seq"`
	ObservedAt         int64           `json:"observed_at"`
	CreatedAt          int64           `json:"created_at"`
}
