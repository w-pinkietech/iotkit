package outputadapter

import (
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
)

const YokaKitConfigSchemaVersion = 1

type YokaKitKind string

const (
	YokaKitProduction YokaKitKind = "production"
	YokaKitOnOff      YokaKitKind = "onoff"
	YokaKitGanttChart YokaKitKind = "gantt_chart"
	YokaKitAlarm      YokaKitKind = "alarm"
)

var yokakitID = regexp.MustCompile(
	`^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$`,
)

type YokaKitConfig struct {
	SourceID string      `json:"source_id"`
	SignalID string      `json:"signal_id"`
	Kind     YokaKitKind `json:"kind"`
	Reason   string      `json:"reason,omitempty"`
}

type YokaKitAdapter struct{}

var _ Adapter = YokaKitAdapter{}

func (YokaKitAdapter) Descriptor() Descriptor {
	return Descriptor{
		ID:                  "yokakit.mqtt.v1",
		DisplayName:         "YokaKit MQTT v1",
		ConfigSchemaVersion: YokaKitConfigSchemaVersion,
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

func EncodeYokaKitConfig(config YokaKitConfig) (json.RawMessage, error) {
	return json.Marshal(struct {
		SchemaVersion int `json:"schema_version"`
		YokaKitConfig
	}{
		SchemaVersion: YokaKitConfigSchemaVersion,
		YokaKitConfig: config,
	})
}

func (YokaKitAdapter) ValidateConfig(
	raw json.RawMessage,
	sourceKind ObservationKind,
) error {
	config, err := decodeYokaKitConfig(raw)
	if err != nil {
		return err
	}
	if !yokakitID.MatchString(config.SourceID) ||
		!yokakitID.MatchString(config.SignalID) {
		return fmt.Errorf(
			"%w: YokaKit source_id and signal_id must use the closed topic ID syntax",
			ErrInvalidConfiguration,
		)
	}
	if len(config.Reason) > 512 {
		return fmt.Errorf("%w: YokaKit alarm reason exceeds 512 bytes",
			ErrInvalidConfiguration)
	}
	switch config.Kind {
	case YokaKitProduction, YokaKitOnOff, YokaKitGanttChart, YokaKitAlarm:
	default:
		return fmt.Errorf("%w: unsupported YokaKit mode %q",
			ErrInvalidConfiguration, config.Kind)
	}
	if config.Kind != YokaKitAlarm && config.Reason != "" {
		return fmt.Errorf("%w: reason is only valid for YokaKit alarm",
			ErrInvalidConfiguration)
	}
	if !compatibleYokaKitKind(sourceKind, config.Kind) {
		return fmt.Errorf("%w: %s cannot produce YokaKit %s",
			ErrUnsupportedObservation, sourceKind, config.Kind)
	}
	return nil
}

func (adapter YokaKitAdapter) Transform(
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
	config, err := decodeYokaKitConfig(raw)
	if err != nil {
		return noPublication, err
	}
	if config.Kind == YokaKitProduction {
		var value int64
		if err := json.Unmarshal(observation.Value, &value); err != nil ||
			value > maxSafeInteger {
			return noPublication, fmt.Errorf(
				"%w: YokaKit production exceeds its portable integer range",
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
		Kind          YokaKitKind     `json:"kind"`
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
	if config.Kind == YokaKitAlarm {
		payload.Reason = config.Reason
		payload.Reading = observation.Reading
	}
	encoded, err := json.Marshal(payload)
	if err != nil {
		return noPublication, err
	}
	publication := MQTTPublication{
		Topic: "yokakit/v1/sources/" + config.SourceID + "/signals/" +
			config.SignalID + "/observations",
		QoS:     1,
		Retain:  false,
		Payload: encoded,
	}
	if err := publication.Validate(); err != nil {
		return noPublication, err
	}
	return publication, nil
}

func decodeYokaKitConfig(raw json.RawMessage) (YokaKitConfig, error) {
	var wire struct {
		SchemaVersion int `json:"schema_version"`
		YokaKitConfig
	}
	if err := decodeClosedConfig(raw, &wire); err != nil {
		return YokaKitConfig{}, fmt.Errorf("%w: %v",
			ErrInvalidConfiguration, err)
	}
	if wire.SchemaVersion != YokaKitConfigSchemaVersion {
		return YokaKitConfig{}, fmt.Errorf(
			"%w: unsupported YokaKit config schema version %d",
			ErrInvalidConfiguration,
			wire.SchemaVersion,
		)
	}
	return wire.YokaKitConfig, nil
}

func DecodeYokaKitConfig(raw json.RawMessage) (YokaKitConfig, error) {
	return decodeYokaKitConfig(raw)
}

func compatibleYokaKitKind(
	source ObservationKind,
	target YokaKitKind,
) bool {
	switch target {
	case YokaKitProduction:
		return source == KindCumulativeValue
	case YokaKitOnOff, YokaKitGanttChart:
		return source == KindBoolean
	case YokaKitAlarm:
		return source == KindAlarm
	default:
		return false
	}
}

func YokaKitStatus(
	sourceID string,
	reportedAt int64,
) (MQTTPublication, error) {
	var noPublication MQTTPublication
	if !yokakitID.MatchString(sourceID) ||
		reportedAt < 0 ||
		reportedAt > maxUnixMillis {
		return noPublication, errors.New("invalid YokaKit source status")
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
		Topic:   "yokakit/v1/sources/" + sourceID + "/status",
		QoS:     1,
		Retain:  true,
		Payload: payload,
	}
	if err := publication.Validate(); err != nil {
		return noPublication, err
	}
	return publication, nil
}
