package outputadapter

import (
	"encoding/json"
	"fmt"
)

const GenericMQTTJSONConfigSchemaVersion = 1

type GenericMQTTJSONConfig struct {
	Topic string `json:"topic"`
}

type GenericMQTTJSONAdapter struct{}

var _ Adapter = GenericMQTTJSONAdapter{}

func (GenericMQTTJSONAdapter) Descriptor() Descriptor {
	return Descriptor{
		ID:                  "iotkit.mqtt-json.v1",
		DisplayName:         "IoTKit MQTT JSON v1",
		ConfigSchemaVersion: GenericMQTTJSONConfigSchemaVersion,
		Modes: []Mode{
			{
				Key:         "observation",
				DisplayName: "IoTKit共通Observation",
				Accepts: []ObservationKind{
					KindNumeric,
					KindBoolean,
					KindCumulativeValue,
					KindAlarm,
				},
			},
		},
	}
}

func EncodeGenericMQTTJSONConfig(
	config GenericMQTTJSONConfig,
) (json.RawMessage, error) {
	return json.Marshal(struct {
		SchemaVersion int `json:"schema_version"`
		GenericMQTTJSONConfig
	}{
		SchemaVersion:         GenericMQTTJSONConfigSchemaVersion,
		GenericMQTTJSONConfig: config,
	})
}

func (GenericMQTTJSONAdapter) ValidateConfig(
	raw json.RawMessage,
	sourceKind ObservationKind,
) error {
	config, err := decodeGenericMQTTJSONConfig(raw)
	if err != nil {
		return err
	}
	if !validObservationKind(sourceKind) {
		return fmt.Errorf(
			"%w: unsupported generic observation kind %q",
			ErrUnsupportedObservation,
			sourceKind,
		)
	}
	if !validExactMQTTTopic(config.Topic) {
		return fmt.Errorf(
			"%w: topic must be an exact MQTT UTF-8 topic",
			ErrInvalidConfiguration,
		)
	}
	return nil
}

func (adapter GenericMQTTJSONAdapter) Transform(
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
	config, err := decodeGenericMQTTJSONConfig(raw)
	if err != nil {
		return noPublication, err
	}
	payload, err := json.Marshal(struct {
		SchemaVersion int             `json:"schema_version"`
		ObservationID string          `json:"observation_id"`
		SeriesID      string          `json:"series_id"`
		Sequence      int64           `json:"sequence"`
		ObservedAt    int64           `json:"observed_at"`
		Kind          ObservationKind `json:"kind"`
		Value         json.RawMessage `json:"value"`
		Reading       *float64        `json:"reading,omitempty"`
	}{
		SchemaVersion: 1,
		ObservationID: observation.ObservationID,
		SeriesID:      observation.SeriesID,
		Sequence:      observation.Sequence,
		ObservedAt:    observation.ObservedAt,
		Kind:          observation.Kind,
		Value:         observation.Value,
		Reading:       observation.Reading,
	})
	if err != nil {
		return noPublication, err
	}
	publication := MQTTPublication{
		Topic:   config.Topic,
		QoS:     1,
		Retain:  false,
		Payload: payload,
	}
	if err := publication.Validate(); err != nil {
		return noPublication, err
	}
	return publication, nil
}

func decodeGenericMQTTJSONConfig(
	raw json.RawMessage,
) (GenericMQTTJSONConfig, error) {
	var wire struct {
		SchemaVersion int `json:"schema_version"`
		GenericMQTTJSONConfig
	}
	if err := decodeClosedConfig(raw, &wire); err != nil {
		return GenericMQTTJSONConfig{}, fmt.Errorf(
			"%w: %v",
			ErrInvalidConfiguration,
			err,
		)
	}
	if wire.SchemaVersion != GenericMQTTJSONConfigSchemaVersion {
		return GenericMQTTJSONConfig{}, fmt.Errorf(
			"%w: unsupported IoTKit MQTT JSON config schema %d",
			ErrInvalidConfiguration,
			wire.SchemaVersion,
		)
	}
	return wire.GenericMQTTJSONConfig, nil
}

func DecodeGenericMQTTJSONConfig(
	raw json.RawMessage,
) (GenericMQTTJSONConfig, error) {
	return decodeGenericMQTTJSONConfig(raw)
}
