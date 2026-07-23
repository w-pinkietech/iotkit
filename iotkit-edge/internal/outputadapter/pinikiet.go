package outputadapter

import (
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
)

const PinikietConfigSchemaVersion = 1

type PinikietKind string

const (
	PinikietProduction PinikietKind = "production"
	PinikietOnOff      PinikietKind = "onoff"
	PinikietGanttChart PinikietKind = "gantt_chart"
	PinikietAlarm      PinikietKind = "alarm"
)

var pinikietID = regexp.MustCompile(
	`^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$`,
)

type PinikietConfig struct {
	SourceID string       `json:"source_id"`
	SensorID string       `json:"sensor_id"`
	Kind     PinikietKind `json:"kind"`
	Reason   string       `json:"reason,omitempty"`
}

type PinikietAdapter struct{}

var _ Adapter = PinikietAdapter{}

func (PinikietAdapter) Descriptor() Descriptor {
	return Descriptor{
		ID:                  "pinikiet.mqtt.v1",
		DisplayName:         "Pinikiet MQTT v1",
		ConfigSchemaVersion: PinikietConfigSchemaVersion,
		Modes: []Mode{
			{
				Key: "production", DisplayName: "累積値",
				Accepts: []ObservationKind{KindCumulativeValue},
			},
			{
				Key: "onoff", DisplayName: "ON/OFF",
				Accepts: []ObservationKind{KindBoolean},
			},
			{
				Key: "gantt_chart", DisplayName: "稼働状態",
				Accepts: []ObservationKind{KindBoolean},
			},
			{
				Key: "alarm", DisplayName: "アラーム",
				Accepts: []ObservationKind{KindAlarm},
			},
		},
	}
}

func EncodePinikietConfig(config PinikietConfig) (json.RawMessage, error) {
	return json.Marshal(struct {
		SchemaVersion int `json:"schema_version"`
		PinikietConfig
	}{
		SchemaVersion:  PinikietConfigSchemaVersion,
		PinikietConfig: config,
	})
}

func (PinikietAdapter) ValidateConfig(
	raw json.RawMessage,
	sourceKind ObservationKind,
) error {
	config, err := decodePinikietConfig(raw)
	if err != nil {
		return err
	}
	if !pinikietID.MatchString(config.SourceID) ||
		!pinikietID.MatchString(config.SensorID) {
		return fmt.Errorf(
			"%w: Pinikiet source_id and sensor_id must use the closed topic ID syntax",
			ErrInvalidConfiguration,
		)
	}
	if len(config.Reason) > 512 {
		return fmt.Errorf("%w: Pinikiet alarm reason exceeds 512 bytes",
			ErrInvalidConfiguration)
	}
	switch config.Kind {
	case PinikietProduction, PinikietOnOff, PinikietGanttChart, PinikietAlarm:
	default:
		return fmt.Errorf("%w: unsupported Pinikiet mode %q",
			ErrInvalidConfiguration, config.Kind)
	}
	if config.Kind != PinikietAlarm && config.Reason != "" {
		return fmt.Errorf("%w: reason is only valid for Pinikiet alarm",
			ErrInvalidConfiguration)
	}
	if !compatiblePinikietKind(sourceKind, config.Kind) {
		return fmt.Errorf("%w: %s cannot produce Pinikiet %s",
			ErrUnsupportedObservation, sourceKind, config.Kind)
	}
	return nil
}

func (adapter PinikietAdapter) Transform(
	raw json.RawMessage,
	observation Observation,
) (MQTTPublication, error) {
	var noPublication MQTTPublication
	if err := observation.Validate(); err != nil {
		return noPublication, err
	}
	if err := adapter.ValidateConfig(raw, observation.Kind); err != nil {
		return noPublication, err
	}
	config, err := decodePinikietConfig(raw)
	if err != nil {
		return noPublication, err
	}
	if config.Kind == PinikietProduction {
		var value int64
		if err := json.Unmarshal(observation.Value, &value); err != nil ||
			value > maxSafeInteger {
			return noPublication, fmt.Errorf(
				"%w: Pinikiet production exceeds its portable integer range",
				ErrInvalidObservation,
			)
		}
	}
	payload := struct {
		SchemaVersion int             `json:"schema_version"`
		ObservationID string          `json:"observation_id"`
		SeriesID      string          `json:"series_id"`
		Sequence      int64           `json:"sequence"`
		ObservedAt    int64           `json:"observed_at"`
		Kind          PinikietKind    `json:"kind"`
		Value         json.RawMessage `json:"value"`
		Reason        string          `json:"reason,omitempty"`
		Reading       *float64        `json:"reading,omitempty"`
	}{
		SchemaVersion: 1,
		ObservationID: observation.ObservationID,
		SeriesID:      observation.SeriesID,
		Sequence:      observation.Sequence,
		ObservedAt:    observation.ObservedAt,
		Kind:          config.Kind,
		Value:         observation.Value,
	}
	if config.Kind == PinikietAlarm {
		payload.Reason = config.Reason
		payload.Reading = observation.Reading
	}
	encoded, err := json.Marshal(payload)
	if err != nil {
		return noPublication, err
	}
	publication := MQTTPublication{
		Topic: "pinikiet/v1/sources/" + config.SourceID + "/sensors/" +
			config.SensorID + "/observations",
		QoS:     1,
		Retain:  false,
		Payload: encoded,
	}
	if err := publication.Validate(); err != nil {
		return noPublication, err
	}
	return publication, nil
}

func decodePinikietConfig(raw json.RawMessage) (PinikietConfig, error) {
	var wire struct {
		SchemaVersion int `json:"schema_version"`
		PinikietConfig
	}
	if err := decodeClosedConfig(raw, &wire); err != nil {
		return PinikietConfig{}, fmt.Errorf("%w: %v",
			ErrInvalidConfiguration, err)
	}
	if wire.SchemaVersion != PinikietConfigSchemaVersion {
		return PinikietConfig{}, fmt.Errorf(
			"%w: unsupported Pinikiet config schema version %d",
			ErrInvalidConfiguration,
			wire.SchemaVersion,
		)
	}
	return wire.PinikietConfig, nil
}

func DecodePinikietConfig(raw json.RawMessage) (PinikietConfig, error) {
	return decodePinikietConfig(raw)
}

func compatiblePinikietKind(
	source ObservationKind,
	target PinikietKind,
) bool {
	switch target {
	case PinikietProduction:
		return source == KindCumulativeValue
	case PinikietOnOff, PinikietGanttChart:
		return source == KindBoolean
	case PinikietAlarm:
		return source == KindAlarm
	default:
		return false
	}
}

func PinikietStatus(
	sourceID string,
	reportedAt int64,
) (MQTTPublication, error) {
	var noPublication MQTTPublication
	if !pinikietID.MatchString(sourceID) ||
		reportedAt < 0 ||
		reportedAt > maxUnixMillis {
		return noPublication, errors.New("invalid Pinikiet source status")
	}
	payload, err := json.Marshal(struct {
		SchemaVersion int    `json:"schema_version"`
		ReportedAt    int64  `json:"reported_at"`
		State         string `json:"state"`
	}{1, reportedAt, "online"})
	if err != nil {
		return noPublication, err
	}
	publication := MQTTPublication{
		Topic:   "pinikiet/v1/sources/" + sourceID + "/status",
		QoS:     1,
		Retain:  true,
		Payload: payload,
	}
	if err := publication.Validate(); err != nil {
		return noPublication, err
	}
	return publication, nil
}
