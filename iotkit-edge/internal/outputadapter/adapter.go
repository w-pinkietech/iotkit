package outputadapter

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"regexp"
	"strings"
	"unicode/utf8"
)

const (
	maxSafeInteger = int64(9_007_199_254_740_991)
	maxUnixMillis  = int64(253_402_300_799_999)
)

var (
	adapterIDPattern = regexp.MustCompile(
		`^[a-z][a-z0-9]*(?:[.-][a-z0-9]+)*$`,
	)
	modeKeyPattern = regexp.MustCompile(`^[a-z][a-z0-9_]*$`)
	uuidPattern    = regexp.MustCompile(
		`^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`,
	)

	ErrInvalidDescriptor      = errors.New("invalid output adapter descriptor")
	ErrInvalidConfiguration   = errors.New("invalid output adapter configuration")
	ErrInvalidObservation     = errors.New("invalid output adapter observation")
	ErrUnsupportedObservation = errors.New(
		"output adapter does not support the observation",
	)
	ErrInvalidPublication = errors.New("invalid MQTT output publication")
)

// ObservationKind is IoTKit's provider-neutral meaning at the Output Adapter
// boundary. It deliberately does not contain application names such as
// production or gantt_chart.
type ObservationKind string

const (
	KindNumeric         ObservationKind = "numeric"
	KindBoolean         ObservationKind = "boolean"
	KindCumulativeValue ObservationKind = "cumulative_value"
	KindAlarm           ObservationKind = "alarm"
)

// Observation is the complete, transport-independent input to an Output
// Adapter. It contains no EdgeNode custody cursor or application-specific identity.
type Observation struct {
	ObservationID string
	SeriesID      string
	Sequence      int64
	ObservedAt    int64
	Kind          ObservationKind
	Value         json.RawMessage
	Reading       *float64
}

func (observation Observation) Validate() error {
	if !uuidPattern.MatchString(observation.ObservationID) {
		return fmt.Errorf("%w: observation_id must be a lowercase UUID",
			ErrInvalidObservation)
	}
	if !uuidPattern.MatchString(observation.SeriesID) {
		return fmt.Errorf("%w: series_id must be a lowercase UUID",
			ErrInvalidObservation)
	}
	if observation.Sequence < 1 || observation.Sequence > maxSafeInteger {
		return fmt.Errorf("%w: sequence is outside the portable integer range",
			ErrInvalidObservation)
	}
	if observation.ObservedAt < 0 || observation.ObservedAt > maxUnixMillis {
		return fmt.Errorf("%w: observed_at is outside the supported range",
			ErrInvalidObservation)
	}
	if !json.Valid(observation.Value) {
		return fmt.Errorf("%w: value must be valid JSON",
			ErrInvalidObservation)
	}
	if observation.Reading != nil &&
		(math.IsNaN(*observation.Reading) || math.IsInf(*observation.Reading, 0)) {
		return fmt.Errorf("%w: reading must be finite",
			ErrInvalidObservation)
	}
	switch observation.Kind {
	case KindNumeric:
		var value float64
		if err := json.Unmarshal(observation.Value, &value); err != nil ||
			math.IsNaN(value) || math.IsInf(value, 0) {
			return fmt.Errorf("%w: numeric value must be finite",
				ErrInvalidObservation)
		}
	case KindBoolean, KindAlarm:
		var value bool
		if err := json.Unmarshal(observation.Value, &value); err != nil {
			return fmt.Errorf("%w: %s value must be boolean",
				ErrInvalidObservation, observation.Kind)
		}
	case KindCumulativeValue:
		var value int64
		if err := json.Unmarshal(observation.Value, &value); err != nil ||
			value < 0 {
			return fmt.Errorf("%w: cumulative value must be a non-negative integer",
				ErrInvalidObservation)
		}
	default:
		return fmt.Errorf("%w: unknown observation kind %q",
			ErrInvalidObservation, observation.Kind)
	}
	return nil
}

type Mode struct {
	Key         string            `json:"key"`
	DisplayName string            `json:"display_name"`
	Accepts     []ObservationKind `json:"accepts"`
}

type Descriptor struct {
	ID                  string `json:"id"`
	DisplayName         string `json:"display_name"`
	ConfigSchemaVersion int    `json:"config_schema_version"`
	Modes               []Mode `json:"modes"`
}

func (descriptor Descriptor) Validate() error {
	if !adapterIDPattern.MatchString(descriptor.ID) {
		return fmt.Errorf("%w: invalid adapter ID", ErrInvalidDescriptor)
	}
	if strings.TrimSpace(descriptor.DisplayName) == "" ||
		len(descriptor.DisplayName) > 128 {
		return fmt.Errorf("%w: invalid display name", ErrInvalidDescriptor)
	}
	if descriptor.ConfigSchemaVersion < 1 {
		return fmt.Errorf("%w: config schema version must be positive",
			ErrInvalidDescriptor)
	}
	if len(descriptor.Modes) == 0 {
		return fmt.Errorf("%w: at least one mode is required",
			ErrInvalidDescriptor)
	}
	modeKeys := make(map[string]struct{}, len(descriptor.Modes))
	for _, mode := range descriptor.Modes {
		if !modeKeyPattern.MatchString(mode.Key) ||
			strings.TrimSpace(mode.DisplayName) == "" ||
			len(mode.Accepts) == 0 {
			return fmt.Errorf("%w: invalid mode", ErrInvalidDescriptor)
		}
		if _, duplicate := modeKeys[mode.Key]; duplicate {
			return fmt.Errorf("%w: duplicate mode %q",
				ErrInvalidDescriptor, mode.Key)
		}
		modeKeys[mode.Key] = struct{}{}
		accepted := make(map[ObservationKind]struct{}, len(mode.Accepts))
		for _, kind := range mode.Accepts {
			if !validObservationKind(kind) {
				return fmt.Errorf("%w: unknown accepted observation kind %q",
					ErrInvalidDescriptor, kind)
			}
			if _, duplicate := accepted[kind]; duplicate {
				return fmt.Errorf("%w: duplicate accepted observation kind %q",
					ErrInvalidDescriptor, kind)
			}
			accepted[kind] = struct{}{}
		}
	}
	return nil
}

func validObservationKind(kind ObservationKind) bool {
	switch kind {
	case KindNumeric, KindBoolean, KindCumulativeValue, KindAlarm:
		return true
	default:
		return false
	}
}

// MQTTPublication is a fully rendered application message. The delivery layer
// persists and publishes it; the Adapter never performs network I/O itself.
type MQTTPublication struct {
	Topic   string
	QoS     byte
	Retain  bool
	Payload json.RawMessage
}

func (publication MQTTPublication) Validate() error {
	if !validExactMQTTTopic(publication.Topic) {
		return fmt.Errorf("%w: topic must be an exact MQTT UTF-8 topic",
			ErrInvalidPublication)
	}
	if publication.QoS != 1 {
		return fmt.Errorf("%w: v1 requires QoS 1", ErrInvalidPublication)
	}
	if !json.Valid(publication.Payload) {
		return fmt.Errorf("%w: payload must be valid JSON",
			ErrInvalidPublication)
	}
	return nil
}

func validExactMQTTTopic(topic string) bool {
	return topic != "" &&
		len(topic) <= 65_535 &&
		utf8.ValidString(topic) &&
		!strings.ContainsAny(topic, "\x00+#")
}

// Adapter is an in-process, deterministic transformer. Implementations must
// not access a Broker, secrets, storage, clocks, or the network.
type Adapter interface {
	Descriptor() Descriptor
	ValidateConfig(json.RawMessage, ObservationKind) error
	Transform(json.RawMessage, Observation) (MQTTPublication, error)
}

func decodeClosedConfig(raw json.RawMessage, target any) error {
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(target); err != nil {
		return err
	}
	if err := decoder.Decode(&struct{}{}); !errors.Is(err, io.EOF) {
		if err == nil {
			return errors.New("configuration contains multiple JSON values")
		}
		return err
	}
	return nil
}
