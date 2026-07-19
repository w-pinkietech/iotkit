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

type DetectorMode string

const (
	DetectorNone              DetectorMode = ""
	DetectorBooleanHighActive DetectorMode = "boolean_high_active"
	DetectorBooleanLowActive  DetectorMode = "boolean_low_active"
	DetectorHighActive        DetectorMode = "high_active"
	DetectorLowActive         DetectorMode = "low_active"
)

type TriggerMode string

const (
	TriggerTransition   TriggerMode = "on_transition"
	TriggerActiveSample TriggerMode = "on_notification"
)

const maxDebounceMS = int64(300_000)

type Detector struct {
	Mode           DetectorMode `json:"mode"`
	RiseThreshold  float64      `json:"rise_threshold"`
	FallThreshold  float64      `json:"fall_threshold"`
	RiseDebounceMS int64        `json:"rise_debounce_ms"`
	FallDebounceMS int64        `json:"fall_debounce_ms"`
}

type DefinitionSpec struct {
	Kind     Kind        `json:"kind"`
	Scale    float64     `json:"scale"`
	Offset   float64     `json:"offset"`
	Detector Detector    `json:"detector"`
	Trigger  TriggerMode `json:"trigger"`
}

type Calibration struct {
	SignalRef string  `json:"signal_ref"`
	Revision  int64   `json:"revision"`
	Scale     float64 `json:"scale"`
	Offset    float64 `json:"offset"`
	CreatedAt int64   `json:"created_at"`
}

func (calibration Calibration) Validate() error {
	if !finite(calibration.Scale) || calibration.Scale == 0 {
		return errors.New("semantic calibration scale must be a finite non-zero number")
	}
	if !finite(calibration.Offset) {
		return errors.New("semantic calibration offset must be finite")
	}
	return nil
}

func (calibration Calibration) Apply(input float64) (float64, error) {
	if err := calibration.Validate(); err != nil {
		return 0, err
	}
	if !finite(input) {
		return 0, errors.New("semantic input must be finite")
	}
	calibrated := input*calibration.Scale + calibration.Offset
	if !finite(calibrated) {
		return 0, errors.New("calibrated semantic input must be finite")
	}
	return calibrated, nil
}

type RuleSpec struct {
	Kind     Kind        `json:"kind"`
	Detector Detector    `json:"detector"`
	Trigger  TriggerMode `json:"trigger"`
}

func (spec RuleSpec) Validate() error {
	return DefinitionSpec{
		Kind:     spec.Kind,
		Scale:    1,
		Detector: spec.Detector,
		Trigger:  spec.Trigger,
	}.Validate()
}

type Rule struct {
	ID          string `json:"rule_id"`
	SignalRef   string `json:"signal_ref"`
	DisplayName string `json:"display_name"`
	SeriesID    string `json:"series_id"`
	Revision    int64  `json:"revision"`
	RuleSpec
	Active    bool   `json:"active"`
	CreatedAt int64  `json:"created_at"`
	RetiredAt *int64 `json:"retired_at,omitempty"`
}

type Configuration struct {
	SignalRef   string      `json:"signal_ref"`
	Revision    int64       `json:"revision"`
	Calibration Calibration `json:"calibration"`
	Rules       []Rule      `json:"rules"`
}

type CounterReset struct {
	ID               string `json:"reset_id"`
	RuleID           string `json:"rule_id"`
	LedgerEpoch      string `json:"ledger_epoch"`
	ApplyAfterPubSeq int64  `json:"apply_after_pub_seq"`
	RequestedAt      int64  `json:"requested_at"`
	AppliedAt        *int64 `json:"applied_at,omitempty"`
}

// UnmarshalJSON keeps existing Site databases readable while new writes use the
// explicit rising/falling detector contract.
func (spec *DefinitionSpec) UnmarshalJSON(data []byte) error {
	type wireSpec struct {
		Kind      Kind     `json:"kind"`
		Scale     float64  `json:"scale"`
		Offset    float64  `json:"offset"`
		Detector  Detector `json:"detector"`
		Condition struct {
			Mode       string  `json:"mode"`
			BoolValue  bool    `json:"bool_value"`
			Threshold  float64 `json:"threshold"`
			Hysteresis float64 `json:"hysteresis"`
		} `json:"condition"`
		Trigger TriggerMode `json:"trigger"`
	}
	var wire wireSpec
	if err := json.Unmarshal(data, &wire); err != nil {
		return err
	}
	*spec = DefinitionSpec{
		Kind: wire.Kind, Scale: wire.Scale, Offset: wire.Offset,
		Detector: wire.Detector, Trigger: wire.Trigger,
	}
	if spec.Detector.Mode != DetectorNone || wire.Condition.Mode == "" {
		return nil
	}
	switch wire.Condition.Mode {
	case "boolean_equals":
		if wire.Condition.BoolValue {
			spec.Detector.Mode = DetectorBooleanHighActive
		} else {
			spec.Detector.Mode = DetectorBooleanLowActive
		}
	case "above":
		spec.Detector = Detector{
			Mode:          DetectorHighActive,
			RiseThreshold: wire.Condition.Threshold,
			FallThreshold: wire.Condition.Threshold - wire.Condition.Hysteresis,
		}
	case "below":
		spec.Detector = Detector{
			Mode:          DetectorLowActive,
			RiseThreshold: wire.Condition.Threshold + wire.Condition.Hysteresis,
			FallThreshold: wire.Condition.Threshold,
		}
	}
	return nil
}

func (spec DefinitionSpec) Validate() error {
	if !finite(spec.Scale) || spec.Scale == 0 {
		return errors.New("semantic scale must be a finite non-zero number")
	}
	if !finite(spec.Offset) {
		return errors.New("semantic offset must be finite")
	}
	if !finite(spec.Detector.RiseThreshold) || !finite(spec.Detector.FallThreshold) {
		return errors.New("semantic detector thresholds must be finite")
	}
	if spec.Detector.RiseDebounceMS < 0 || spec.Detector.RiseDebounceMS > maxDebounceMS ||
		spec.Detector.FallDebounceMS < 0 || spec.Detector.FallDebounceMS > maxDebounceMS {
		return errors.New("semantic detector debounce must be between 0 and 300000 milliseconds")
	}
	if analogDetector(spec.Detector.Mode) &&
		spec.Detector.FallThreshold > spec.Detector.RiseThreshold {
		return errors.New("semantic falling threshold cannot exceed rising threshold")
	}
	switch spec.Kind {
	case KindNumeric:
		if spec.Detector.Mode != DetectorNone || spec.Trigger != "" {
			return errors.New("numeric semantic definition cannot have a detector or trigger")
		}
	case KindBoolean:
		if !validDetector(spec.Detector.Mode) {
			return errors.New("boolean semantic definition requires a detector")
		}
		if spec.Trigger != "" {
			return errors.New("boolean semantic definition cannot have a trigger")
		}
	case KindCumulativeCounter:
		if !validDetector(spec.Detector.Mode) {
			return errors.New("cumulative counter requires a detector")
		}
		if spec.Trigger != TriggerTransition && spec.Trigger != TriggerActiveSample {
			return errors.New("cumulative counter requires a supported trigger")
		}
	case KindAlarm:
		if !validDetector(spec.Detector.Mode) {
			return errors.New("alarm semantic definition requires a detector")
		}
		if spec.Trigger != "" {
			return errors.New("alarm semantic definition cannot have a trigger")
		}
	default:
		return errors.New("unsupported semantic definition kind")
	}
	return nil
}

func validDetector(mode DetectorMode) bool {
	return mode == DetectorBooleanHighActive ||
		mode == DetectorBooleanLowActive ||
		mode == DetectorHighActive ||
		mode == DetectorLowActive
}

func analogDetector(mode DetectorMode) bool {
	return mode == DetectorHighActive || mode == DetectorLowActive
}

func finite(value float64) bool {
	return !math.IsNaN(value) && !math.IsInf(value, 0)
}

type State struct {
	Initialized   bool
	Active        bool
	Counter       int64
	Pending       bool
	PendingActive bool
	PendingSince  int64
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

type RuleObservation struct {
	RowID               int64           `json:"row_id"`
	ObservationID       string          `json:"observation_id"`
	RuleID              string          `json:"rule_id"`
	RuleRevision        int64           `json:"rule_revision"`
	CalibrationRevision int64           `json:"calibration_revision"`
	SeriesID            string          `json:"series_id"`
	Sequence            int64           `json:"sequence"`
	Kind                Kind            `json:"kind"`
	Value               json.RawMessage `json:"value"`
	SignalRef           string          `json:"signal_ref"`
	EdgeNodeID          string          `json:"edge_node_id"`
	LedgerEpoch         string          `json:"-"`
	SourcePubSeq        int64           `json:"source_pub_seq"`
	ObservedAt          int64           `json:"observed_at"`
	CreatedAt           int64           `json:"created_at"`
}
